//! 命令执行上下文约定——防回流闸门（契约测试）。
//!
//! ## 为什么需要它（2026-10-15 事故）
//!
//! 非 async 的 `#[tauri::command]` 走 Tauri 的 **Blocking 执行上下文**：函数体在
//! 主线程消息循环里执行（tauri-macros `wrapper.rs` 默认 `ExecutionContext::Blocking`；
//! 官方文档：async 命令首选，非 async 会 **result in UI freezes**）。
//! 本仓曾有 169 处这样的命令，其中一批直连 SQLite / 文件系统；连接池 `acquire()`
//! 在池满时最长阻塞 30s——收尾期（记忆落盘 + 自动提炼）一旦占满连接，UI 命令
//! 就把主线程卡死，Windows 判定「无响应」并弹「结束进程」。
//!
//! 现全部改为 `#[tauri::command(async)]`（函数体移出主线程）。本测试保证**不再回流**：
//! 新写的同步命令必须显式写豁免注释说明理由，否则测试失败。
//!
//! ## 豁免写法（必须紧邻属性上方三行内）
//!
//! ```ignore
//! // tauri-sync-ok: <为什么必须同步执行>
//! #[tauri::command]
//! pub fn ... { }
//! ```

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// 豁免标记（必须带理由，逼着每个同步命令被显式审视）
    const EXEMPTION: &str = "tauri-sync-ok:";

    fn collect_rs(root: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }

    /// 属性行后（跳过文档注释与其它属性）的第一条声明若不是 `async fn`、属性本身不含
    /// `async`、且上方三行内无豁免注释 → 违规。
    fn violations_in(text: &str) -> Vec<(usize, String)> {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        for (idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            // 整行属性（注释里的示例以 `//` 开头，不会命中）
            if !trimmed.starts_with("#[tauri::command") {
                continue;
            }
            if trimmed.contains("async") {
                continue; // 显式 (async) 或含 async 参数
            }
            let mut j = idx + 1;
            while j < lines.len() {
                let t = lines[j].trim_start();
                if t.starts_with("///") || t.starts_with("//!") || t.starts_with("#[") {
                    j += 1;
                    continue;
                }
                break;
            }
            if j >= lines.len() {
                continue;
            }
            if lines[j].contains("async fn") {
                continue; // async fn 本身就是异步命令
            }
            let from = idx.saturating_sub(3);
            if lines[from..idx].iter().any(|l| l.contains(EXEMPTION)) {
                continue;
            }
            out.push((idx + 1, trimmed.to_string()));
        }
        out
    }

    #[test]
    fn scanner_detects_sync_command_and_respects_exemption() {
        // 自检：闸门本身要能抓出违规（否则闸门是假绿）
        let bad = "#[tauri::command]\npub fn a() {}\n";
        assert_eq!(violations_in(bad).len(), 1);

        let ok_attr = "#[tauri::command(async)]\npub fn a() {}\n";
        assert!(violations_in(ok_attr).is_empty());

        let ok_async = "#[tauri::command]\n/// 文档\npub async fn a() {}\n";
        assert!(violations_in(ok_async).is_empty());

        let exempt = format!(
            "// {EXEMPTION} 仅读内存，任何情况下 <1ms\n#[tauri::command]\npub fn a() {{}}\n"
        );
        assert!(violations_in(&exempt).is_empty());
    }

    #[test]
    fn every_tauri_command_runs_off_the_main_thread() {
        let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        collect_rs(&src_root, &mut files);
        assert!(!files.is_empty(), "未扫描到源文件，闸门失效");

        let mut violations = Vec::new();
        for path in &files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for (line_no, line) in violations_in(&text) {
                violations.push(format!("{}:{line_no} | {line}", path.display()));
            }
        }

        assert!(
            violations.is_empty(),
            "发现 {} 处同步 #[tauri::command]：函数体会在**主线程**执行，\
             任何触库/触盘都可能让窗口「无响应」。\n\
             修复：改为 `#[tauri::command(async)]`（函数体移出主线程）；\
             确需同步执行时，在上方三行内写 `// {EXEMPTION} <理由>`。\n{}",
            violations.len(),
            violations.join("\n")
        );
    }
}
