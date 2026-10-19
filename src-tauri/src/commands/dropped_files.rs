//! 拖拽/选择落盘：把浏览器 `File` 的字节内容写入应用缓存目录，换取绝对路径。
//!
//! 背景（单通道重构）：前端统一用 HTML5 `drop` / `<input type=file>` 作为**唯一**
//! 附件通道后，浏览器出于安全屏蔽了 `File.path`——非图片附件拿不到绝对路径，
//! 而引用栏与后端读文件都需要真实路径。此命令负责把字节落盘并回传路径，
//! 使「拖拽的文档」与「历史原生通道拿到的文档」在下游完全等价。
//!
//! 设计要点：
//! - 落盘到应用缓存目录 `Nuphus/dropped/`，不污染用户目录、不覆盖既有文件；
//! - 文件名做安全净化（仅保留字母数字/`.`/`-`/`_`/中文），杜绝路径穿越；
//! - 同名冲突加时间戳后缀，不静默覆盖用户已有文件；
//! - 50MB 上限，与 `read_image_base64` 对齐，防止超大文件打爆内存。

use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 落盘目录：`NUPHUS_DROP_DIR` > `NUPHUS_DATA_DIR/dropped` > `<data_dir>/Nuphus/dropped`
fn dropped_dir() -> PathBuf {
    std::env::var("NUPHUS_DROP_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var("NUPHUS_DATA_DIR")
                .ok()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    dirs::data_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join("Nuphus")
                });
            base.join("dropped")
        })
}

/// 净化文件名：剔除路径分隔符与控制字符，仅保留安全字符；为空则回退 `file`。
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(raw)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

/// 把前端传来的 base64 内容落盘，返回 `{ path, name }`。
///
/// - `file_name`：原始文件名，仅取末段并净化；
/// - `base64_data`：文件字节的 standard base64（前端 `FileReader.readAsDataURL` 去掉 data URL 前缀后所得）。
#[tauri::command]
pub async fn save_dropped_file(file_name: String, base64_data: String) -> Result<Value, String> {
    use std::io::Write;

    // base64 体积 ≈ 原始字节 * 4/3，先按长度粗筛，再按解码后精确校验
    if base64_data.len() > 50 * 1024 * 1024 * 4 / 3 + 16 {
        return Err("文件超过 50MB 上限".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.trim())
        .map_err(|e| format!("base64 解码失败: {e}"))?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err("文件超过 50MB 上限".to_string());
    }

    let safe_name = sanitize_name(&file_name);
    let dir = dropped_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建落盘目录失败: {e}"))?;

    // 同名冲突加时间戳，绝不覆盖既有文件
    let mut target = dir.join(&safe_name);
    if target.exists() {
        let stem = target
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file")
            .to_string();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        target = dir.join(format!(
            "{stem}-{stamp}{}",
            target
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default()
        ));
    }

    let mut file = std::fs::File::create(&target).map_err(|e| format!("创建文件失败: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("写入文件失败: {e}"))?;

    let path_str = target.to_string_lossy().to_string();
    Ok(json!({ "path": path_str, "name": safe_name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    /// 文件名净化：只取末段，剔除控制字符与前导点，防路径穿越。
    #[test]
    fn sanitize_name_strips_paths_and_control_chars() {
        assert_eq!(sanitize_name("a.txt"), "a.txt");
        // Windows 路径与 POSIX 路径都只取末段
        assert_eq!(sanitize_name(r"C:\Users\me\secret.txt"), "secret.txt");
        assert_eq!(sanitize_name("/home/me/secret.txt"), "secret.txt");
        // 经典穿越串：取末段后已无分隔符
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        // 控制字符剔除
        assert_eq!(sanitize_name("a\u{0}b.txt"), "ab.txt");
        // 前导点（隐藏文件语义）被剥掉，避免落盘成 `.` / `..`
        assert_eq!(sanitize_name(".hidden"), "hidden");
        assert_eq!(sanitize_name(".."), "file");
        // 空名回退
        assert_eq!(sanitize_name(""), "file");
    }

    /// 落盘往返：写入的内容与解码后字节一致，且返回路径真实可读。
    #[tokio::test]
    async fn saves_file_and_returns_readable_path() {
        let dir = std::env::temp_dir().join(format!("nuphus_drop_test_{}", std::process::id()));
        std::env::set_var("NUPHUS_DROP_DIR", &dir);

        let payload = b"hello dropped";
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let out = save_dropped_file("note.txt".into(), b64)
            .await
            .expect("应落盘成功");

        let path = out["path"].as_str().expect("应返回 path");
        let name = out["name"].as_str().expect("应返回 name");
        assert_eq!(name, "note.txt");
        assert_eq!(std::fs::read(path).expect("路径应可读"), payload);

        std::fs::remove_dir_all(&dir).ok();
        std::env::remove_var("NUPHUS_DROP_DIR");
    }

    /// 非法 base64 必须报错而非写出半个文件。
    #[tokio::test]
    async fn rejects_invalid_base64() {
        std::env::set_var(
            "NUPHUS_DROP_DIR",
            std::env::temp_dir().join(format!("nuphus_drop_bad_{}", std::process::id())),
        );
        let err = save_dropped_file("x.txt".into(), "!!!not base64!!!".into())
            .await
            .expect_err("非法 base64 应报错");
        assert!(err.contains("base64"), "错误信息应指向解码失败: {err}");
        std::env::remove_var("NUPHUS_DROP_DIR");
    }
}
