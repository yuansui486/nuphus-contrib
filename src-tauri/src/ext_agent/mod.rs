//! ext_agent — agent_dispatch 工具编排实现（桌面壳侧）
//!
//! 完整链路（以本文件实现为准）：
//!   ① 校验 + 上板：validate_agent/validate_task_id → ensure_handoff_at 写 brief
//!      （brief 内嵌 build_contract 渲染的门铃契约）→ status.json 置 dispatched
//!      （上板≠执行：in_progress 由外部 Agent 第一声 ready/progress 门铃触发）
//!   ② 进程捕获（当次实况，Leader 主导启动模型）：显式 pid → 当次 windows_list
//!      按 process_id 匹配可见窗口（**直配落空自动回溯父进程链**——TUI 窗口常建在
//!      宿主 shell 名下，如 opencode.exe 的窗口宿主是 powershell.exe）；缺省 → 按
//!      window_hint/process 全表扫描。禁止历史缓存句柄、禁止隐式冷启动——进程生命
//!      周期归 Leader（skill §2 启动 SOP）
//!   ②b 在途闸（上板前）：板上有未终态任务且 task_id 不同 → 拒绝派发（不落 error、
//!      不动看板——在途任务的 state 属于那一轮）；同 task_id 重派/续派放行
//!   ③ SeqRunner：按 team.toml dispatch_steps 工具序列确定性执行（不经 LLM），
//!      每步成败结构化回传，禁止静默
//!   ④ 门铃异步：同步路径不做任何等待——门铃事件到达后自动注入 Leader 上下文，
//!      在此等待只会顶撞工具层超时上限（registry 的 agent_dispatch 档位）
//!   ⑤ 失败收口：投递中断 → status.json 落 state=error + error_reason（否则永久
//!      停在 dispatched 幽灵态）+ HUD error 显式收尾（running 常驻无 autoHide）
//!
//! 注册：main.rs setup 调用 init_bridge(app)，注入 fn 指针到 nuphus::ext_agent_bridge。
//! 工具 executor 是同步 fn，经 run_blocking + Handle::block_on 在 Leader 的
//! tokio 上下文中驱动本模块的 async 编排。

use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use nuphus::desktop::DesktopClient;
use std::collections::HashMap;
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

mod seq_runner;

use seq_runner::SeqError;

/// AppHandle for the bridge path（工具 executor 无 Tauri State 访问，模式对齐 render/video）
static APP: OnceLock<AppHandle> = OnceLock::new();

/// main.rs setup 调用：存 AppHandle + 注册桥实现。
pub fn init_bridge(app: &AppHandle) {
    let _ = APP.set(app.clone());
    nuphus::ext_agent_bridge::register_agent_dispatch_impl(bridge_dispatch);
    tracing::info!("[ext_agent] agent_dispatch bridge registered");
}

/// 桥入口（同步 fn）：在 Leader 的 tokio 上下文内驱动 async 编排。
fn bridge_dispatch(params: &serde_json::Value) -> Result<String, String> {
    let app = APP.get().ok_or_else(|| {
        "agent_dispatch 桥接未初始化（桌面壳未注册 ext_agent bridge）".to_string()
    })?;
    let app = app.clone();
    let params = params.clone();
    nuphus::tools::builtin::run_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(dispatch_async(app, params))
    })
}

// ────────────────────────────────────────────────────────────────────────────
// 编排
// ────────────────────────────────────────────────────────────────────────────

/// 派发失败统一收口：先给该 agent 落 `state="error"` + 人类可读原因，再原样返回错误。
///
/// 为什么必须有：上板成功但投递失败时，status.json 会永久停在 `dispatched`
/// （面板显示「已派发·待确认」 forever），用户看不到这一轮其实已经失败。
/// 前端早已把 `error` 映射为 is-error，缺的只是「有人写它」。
/// `reason` 需能指认失败环节（进程没起来 / 窗口没捕获 / 输入没进去），由调用方给出。
fn fail_dispatch(
    root: &std::path::Path,
    agent: &str,
    task_id: &str,
    reason: String,
) -> Result<String, String> {
    crate::commands::config::handoff::mark_agent_error_at(root, agent, Some(task_id), &reason);
    Err(reason)
}

/// agent_dispatch 编排主流程。两分支均以 Ok(JSON) 返回：ok:true（投递序列完成）
/// / ok:false（序列中断，带 failed_step/hint）；上板前失败走 fail_dispatch 的 Err
/// 文本路径（executor 包装为「agent_dispatch 失败：…」）。同步路径不等待门铃；
/// 外层超时口径见 registry 的 agent_dispatch 档位（超时不取消序列，只停止等待）。
async fn dispatch_async(app: AppHandle, params: serde_json::Value) -> Result<String, String> {
    let agent = params
        .get("agent")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "agent 必填".to_string())?
        .to_string();
    let task_id = params
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "task_id 必填".to_string())?
        .to_string();
    let brief = params
        .get("brief")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "brief 必填".to_string())?
        .to_string();
    let message_override = params
        .get("message")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // Leader 启动外部 Agent 时持有的进程 PID（§2 启动 SOP）：显式传入则对本轮实况
    // 校验存活并解析其窗口；缺省则退回按 window_hint 当次扫描。禁止任何历史缓存句柄。
    let requested_pid = params
        .get("pid")
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        })
        .map(|p| p as u32);

    let state = app.state::<AppState>();
    let emitter = crate::emitter::CompoundEmitter::new(app.clone(), &state);

    // ① 校验 + 上板（brief 内嵌门铃契约，token 说明指向 brief 中的令牌行）
    crate::commands::config::handoff::validate_agent(&agent)?;
    crate::commands::config::handoff::validate_task_id(&task_id)?;
    let root = crate::commands::config::handoff::handoff_root();
    let agent_dir = root.join(&agent);
    let contract = crate::commands::config::handoff::build_contract(&agent, &task_id, &agent_dir);
    let full_brief = format!("{brief}\n\n---\n{contract}\n");
    // 可选目标工作区（审计基线）：声明后该目录的 HEAD 会被记入 status.json，完工时比对
    // 是否出现未派发提交。给了就必须是已存在目录 —— 基线记错比不记更糟（会给出错误的
    // 「无未派发提交」结论），所以宁可当场报错，也不静默忽略。
    let workspace = params
        .get("workspace")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(ws) = workspace {
        if !std::path::Path::new(ws).is_dir() {
            return fail_dispatch(
                &root,
                &agent,
                &task_id,
                format!("workspace 不是已存在的目录: {ws}"),
            );
        }
    }
    // agent 登记读取必须前移到上板之前：「agent 根本没登记」属于上板前失败，
    // 也要先落 error 态——否则状态栏停在上一轮 state，用户看不到本轮派发失败。
    let cfg = match crate::commands::config::team::agent_config(&agent) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return fail_dispatch(
                &root,
                &agent,
                &task_id,
                format!("agent「{agent}」未在 team.toml 登记，请先在外部 Agent 配置中心登记"),
            )
        }
        Err(e) => {
            return fail_dispatch(
                &root,
                &agent,
                &task_id,
                format!("读取 agent[{agent}] 配置失败: {e}"),
            )
        }
    };

    // 实测记录（note）：Leader 专属的特别注意事项备忘，随 team 配置一并读取，
    // 注入工具结果供 Leader 派发决策参考（如「ctrl+v 无效用直输」）。
    // 前端 UI 禁止编辑该字段（upsert 不传时后端保留原值），由编排层/手动维护。
    let field_note = cfg
        .get("note")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    // ② 在途闸（上板前最后一道）：agent 有未终态的在途任务时拒绝派发——两个任务会
    // 在同一个 TUI 里交错执行（task_id 一致性闸只保状态不串，保不了终端执行不互扰）。
    // 拒绝**不落 error 态、不动看板**：在途任务还在跑，它的 state/task_id 属于那一轮，
    // 不能被本轮拒绝污染（mark_agent_error_at 无条件写 state=error，故此处不能用
    // fail_dispatch）。同 task_id 的重派/续派放行（失败重试的唯一出口）。
    if let Some(reason) = crate::commands::config::handoff::in_flight_block_reason(
        crate::commands::config::handoff::read_status_at(&root, &agent).as_ref(),
        &task_id,
    ) {
        tracing::warn!("[ext_agent] {agent}::{task_id} 被在途闸拒绝");
        return Err(reason);
    }

    // 上板失败（brief 写不进 / 目录建不出 / 基线记不下）同样落 error 态：
    // 这一刻任务并没有真正上板，停留在上一轮 state 会让用户误判。
    if let Err(e) = crate::commands::config::handoff::ensure_handoff_at(
        &root,
        &agent,
        &task_id,
        &full_brief,
        workspace,
    ) {
        return fail_dispatch(&root, &agent, &task_id, format!("上板失败: {e}"));
    }
    // 可选产物子目录（对齐 read.md「产物写 projects/{project}/」）
    if let Some(project) = params
        .get("project")
        .and_then(|v| v.as_str())
        .filter(|p| !p.is_empty())
    {
        if !project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return fail_dispatch(
                &root,
                &agent,
                &task_id,
                "project 只能包含字母、数字、下划线、连字符".to_string(),
            );
        }
        if let Err(e) = std::fs::create_dir_all(agent_dir.join("projects").join(project)) {
            return fail_dispatch(&root, &agent, &task_id, format!("创建产物子目录失败: {e}"));
        }
    }
    let brief_path = agent_dir.join("briefs").join(format!("{task_id}-brief.md"));
    let brief_path_str = brief_path.to_string_lossy().to_string();

    emitter.emit(NuphusEvent::HudUpdate {
        text: format!("agent_dispatch 上板 {agent}::{task_id}"),
        phase: "running".to_string(),
        step_kind: Some("tool".to_string()),
    });

    // ② 进程捕获（复用 DesktopClient）
    let client = match state.tools.desktop_client() {
        Some(c) => c,
        None => {
            return fail_dispatch(
                &root,
                &agent,
                &task_id,
                "桌面自动化不可用（desktop_client 未连接）：agent 进程与窗口无从捕获".to_string(),
            )
        }
    };

    let mut vars = match capture_process(&agent, &cfg, &client, requested_pid).await {
        Ok(v) => v,
        // 进程没起来 / 窗口没捕获（capture_process 的文案已区分两种成因）
        Err(e) => return fail_dispatch(&root, &agent, &task_id, format!("进程/窗口捕获失败: {e}")),
    };
    vars.insert("task_id".to_string(), task_id.clone());
    vars.insert("brief_path".to_string(), brief_path_str.clone());

    // 渲染投递指令：message 覆盖模板或默认单行指令。
    // 终端直输只承载一行指针——任务细节全部走 brief 文件（多行/中文直输有 IME 上屏风险，
    // 实测不可靠）；协议纪律由契约自身携带，无需在指令中反复叮嘱。
    let mut message = message_override
        .unwrap_or_else(|| format!("Read {brief_path_str} and execute it exactly as written."));
    for (k, v) in &vars {
        message = message.replace(&format!("{{{k}}}"), v);
    }
    vars.insert("message".to_string(), message.clone());

    // ③ SeqRunner 执行 dispatch_steps —— 每步成败结构化回传，禁止静默
    let steps = cfg
        .get("dispatch_steps")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let outcome: Result<usize, SeqError> =
        seq_runner::run_steps(&steps, &vars, &client, Some(&emitter)).await;
    match outcome {
        Err(e) => {
            // 失败就地完整暴露：哪一步、什么工具、什么原因——Leader 无需复跑即可定位
            // 终态 HUD（error 15s autoHide）：同成功分支，running 常驻必须显式收尾。
            // 上板已完成、投递未完成：落 error 态，否则 status.json 永久停在 dispatched。
            let reason = format!(
                "投递失败（第 {} 步 {}）：{}",
                e.step_index + 1,
                e.tool,
                e.message
            );
            crate::commands::config::handoff::mark_agent_error_at(
                &root,
                &agent,
                Some(&task_id),
                &reason,
            );
            emitter.emit(NuphusEvent::HudUpdate {
                text: format!("agent_dispatch {agent}::{task_id} {reason}"),
                phase: "error".to_string(),
                step_kind: Some("tool".to_string()),
            });
            let out = serde_json::json!({
                "ok": false,
                // submitted = brief 已上板（走到本分支即保证上板成功）；它不代表
                // 指令已被外部 Agent 收到——是否补投看 hint，禁止据此字段直接重试
                "submitted": true,
                "brief_path": brief_path_str,
                "error": e.to_string(),
                "failed_step": { "index": e.step_index, "tool": e.tool },
                // 实情口径：序列未被取消（ext_agent 无 kill 路径），已执行步骤不可
                // 回滚，后续步骤可能仍在后台跑。hint 禁止出现「补投递」这类无条件
                // 重试指引——双序列会交错敲键、任务被执行两遍。
                "hint": format!(
                    "上板已完成（brief 已落盘，勿重新上板）；投递序列在第 {} 步「{}」后中断，\
                     序列未被取消——已执行步骤不可回滚，后续步骤是否仍在后台执行未知。\n\
                     接管 SOP（skill §5 失败回退第 6 条）：① Read status.json 确认上板态；\
                     ② process_list/windows_list 核对进程与窗口实况，截图确认指令是否已进入终端；\
                     ③ 已进入 → 勿重投（两条序列会交错敲键、任务被外部 Agent 执行两遍）；\
                     确认未进入 → 才 desktop_window_activate + desktop_input 补输单行指令\
                     「Read {brief_path_str} and execute it.」补完投递；\
                     ④ 进程已死或从未启动 → 重走 §2 启动 SOP。\
                     全程以文件与实况为准，禁止凭本返回文本猜根因。",
                    e.step_index + 1,
                    e.tool
                ),
                "note": field_note,
            });
            Ok(out.to_string())
        }
        Ok(n) => {
            tracing::info!("[ext_agent] {agent}::{task_id} dispatch_steps 完成 {n} 步");
            // 终态 HUD：running 无 autoHide（执行中常驻语义），编排结束必须显式收尾，
            // 否则 HUD 面板永远显示最后一步转动（done/error 均有 15s autoHide）。
            emitter.emit(NuphusEvent::HudUpdate {
                text: format!("agent_dispatch {agent}::{task_id} 投递完成（{n} 步，门铃异步回传）"),
                phase: "done".to_string(),
                step_kind: Some("tool".to_string()),
            });
            let tool_names: Vec<String> = steps
                .iter()
                .take(n)
                .filter_map(|s| {
                    s.get("tool")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();
            // 门铃为异步推送（事件到达后自动注入 Leader 上下文）——同步路径不做任何等待，
            // 第一声拉铃与本轮工具结果本就分属两条链路，等待只会顶撞工具层超时上限。
            let out = serde_json::json!({
                "ok": true,
                "submitted": true,
                "brief_path": brief_path_str,
                "workspace": workspace,
                "window": {
                    "pid": vars.get("pid"),
                    "hwnd": vars.get("hwnd"),
                    "title": vars.get("title"),
                },
                "steps_executed": n,
                "steps": tool_names,
                "note": field_note,
            });
            Ok(out.to_string())
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// ② 进程捕获
// ────────────────────────────────────────────────────────────────────────────

/// 进程目标解析（Leader 主导启动模型）——只对「当次实况」负责：
/// 1. 显式 pid：在当次 windows_list 中按 process_id 匹配可见窗口（进程死即明确报错）；
/// 2. 无 pid：按 window_hint/process 当次全表扫描；
///    禁止读取历史缓存句柄、禁止隐式冷启动——进程生命周期归 Leader（skill §2 启动 SOP），
///    PID/hwnd 每次启动必变且 hwnd 编号会被 OS 复用，任何固化缓存都是错误派发依据。
async fn capture_process(
    agent: &str,
    cfg: &serde_json::Value,
    client: &DesktopClient,
    pid: Option<u32>,
) -> Result<HashMap<String, String>, String> {
    let hint = cfg
        .get("window_hint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            cfg.get("process")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| format!("agent「{agent}」未配置 window_hint/process，无法捕获窗口"))?;

    if let Some(pid) = pid {
        return resolve_hwnd_by_pid(client, pid).await.ok_or_else(|| {
            format!(
                "PID {pid} 及其父进程链上均无可见窗口（外部 Agent「{agent}」）。\n\
                 （TUI 窗口建在宿主进程名下时已自动回溯父进程——此处是回溯后仍失败。）\n\
                 可能原因：① agent 已被关闭；② TUI 尚在启动中、窗口未就绪（等 5–10s 重试）。\n\
                 处置：process_list / windows_list 核对实况后重派——PID/hwnd 按当次解析，禁止缓存。"
            )
        });
    }

    find_window(client, &hint).await.ok_or_else(|| {
        format!(
        "未在当次窗口列表中找到「{hint}」（agent 未启动、窗口未就绪、或标题被覆写导致特征失配）。\n\
         请按 skill §2 启动 SOP 手动启动/核验后重试——禁止依赖历史缓存句柄。"
    )
    })
}

/// 按 PID 在当次 windows_list 匹配可见窗口并提取 hwnd。**TUI 类 agent 的顶层窗口
/// 常建在宿主 shell 名下**（实测：opencode.exe 11332 的窗口宿主是 powershell.exe 11248），
/// 直配 agent PID 会落空——因此按「PID 自身 → 父进程链（至多 4 级）」逐级匹配，取最近命中。
/// PID/hwnd 每次启动必变且会被 OS 复用，只对当次实况负责，禁止缓存。
async fn resolve_hwnd_by_pid(client: &DesktopClient, pid: u32) -> Option<HashMap<String, String>> {
    let list = client.windows_list().await.ok()?;
    let windows = list.get("result")?.as_array()?;
    resolve_hwnd_by_pid_chain(windows, &pid_ancestor_chain(pid, 4))
}

/// 纯匹配核（可单测，不碰桌面/进程 API）：窗口列表 × PID 链，取链上第一个拥有
/// 可见窗口的 PID；vars 记**命中窗口自身**的 pid/hwnd/title。
fn resolve_hwnd_by_pid_chain(
    windows: &[serde_json::Value],
    chain: &[u32],
) -> Option<HashMap<String, String>> {
    for pid in chain {
        for w in windows {
            if w.get("process_id").and_then(|v| v.as_u64()) == Some(*pid as u64) {
                let hwnd = w.get("hwnd").and_then(|v| v.as_i64())?;
                let mut vars = HashMap::new();
                vars.insert("hwnd".to_string(), hwnd.to_string());
                vars.insert("pid".to_string(), pid.to_string());
                if let Some(t) = w.get("title").and_then(|v| v.as_str()) {
                    if !t.is_empty() {
                        vars.insert("title".to_string(), t.to_string());
                    }
                }
                return Some(vars);
            }
        }
    }
    None
}

/// PID 自身 + 至多 max_depth 级父进程（去重、防环）。跨平台走 sysinfo——
/// TUI 窗口建在宿主进程名下时，父链是「agent 进程 → 宿主窗口」的唯一连接。
fn pid_ancestor_chain(pid: u32, max_depth: usize) -> Vec<u32> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything(),
    );
    let parent_of = |p: u32| {
        sys.process(sysinfo::Pid::from(p as usize))
            .and_then(|proc| proc.parent())
            .map(|parent| parent.as_u32())
    };
    ancestor_chain_with(parent_of, pid, max_depth)
}

/// 祖先链纯核（可单测）：自身打头，逐级取父进程；链内去重防环，深度封顶
/// max_depth（链路意外深时不穷追）。
fn ancestor_chain_with(
    parent_of: impl Fn(u32) -> Option<u32>,
    pid: u32,
    max_depth: usize,
) -> Vec<u32> {
    let mut chain = vec![pid];
    let mut cur = pid;
    for _ in 0..max_depth {
        match parent_of(cur) {
            Some(p) if !chain.contains(&p) => {
                chain.push(p);
                cur = p;
            }
            _ => break,
        }
    }
    chain
}

/// 按 window_hint 在 windows_list 中匹配（标题/进程名包含，大小写不敏感）。
async fn find_window(client: &DesktopClient, hint: &str) -> Option<HashMap<String, String>> {
    let list = client.windows_list().await.ok()?;
    let windows = list.get("result")?.as_array()?;
    let hint_lower = hint.to_lowercase();
    for w in windows {
        let title = w.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let process = w.get("process_name").and_then(|v| v.as_str()).unwrap_or("");
        if title.to_lowercase().contains(&hint_lower)
            || process.to_lowercase().contains(&hint_lower)
        {
            let mut vars = HashMap::new();
            if let Some(hwnd) = w.get("hwnd").and_then(|v| v.as_i64()) {
                vars.insert("hwnd".to_string(), hwnd.to_string());
            }
            if !title.is_empty() {
                vars.insert("title".to_string(), title.to_string());
            }
            if let Some(pid) = w.get("process_id").and_then(|v| v.as_i64()) {
                vars.insert("pid".to_string(), pid.to_string());
            }
            if vars.contains_key("hwnd") {
                return Some(vars);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(pid: u32, hwnd: i64, title: &str) -> serde_json::Value {
        serde_json::json!({
            "hwnd": hwnd,
            "title": title,
            "process_id": pid,
            "process_name": "x.exe",
        })
    }

    /// opencode 实况复刻：agent 11332 无窗口，宿主 11248 持有 hwnd 1509460
    #[test]
    fn test_resolve_hwnd_by_pid_chain_parent_hit() {
        let list = vec![win(11248, 1509460, "OC | Fix E0599")];
        let vars = resolve_hwnd_by_pid_chain(&list, &[11332, 11248]).expect("父链应命中宿主窗口");
        assert_eq!(vars.get("hwnd").map(String::as_str), Some("1509460"));
        // vars 里的 pid 必须是命中窗口自身的宿主 pid，不是请求的 agent pid
        assert_eq!(vars.get("pid").map(String::as_str), Some("11248"));
        assert_eq!(
            vars.get("title").map(String::as_str),
            Some("OC | Fix E0599")
        );
    }

    #[test]
    fn test_resolve_hwnd_by_pid_chain_direct_hit_preferred() {
        let list = vec![win(100, 11, "self"), win(200, 22, "host")];
        let vars = resolve_hwnd_by_pid_chain(&list, &[100, 200]).expect("自身直配优先");
        assert_eq!(vars.get("hwnd").map(String::as_str), Some("11"));
        assert_eq!(vars.get("pid").map(String::as_str), Some("100"));
    }

    #[test]
    fn test_resolve_hwnd_by_pid_chain_no_hit() {
        let list = vec![win(999, 11, "other")];
        assert!(resolve_hwnd_by_pid_chain(&list, &[11332, 11248]).is_none());
        assert!(resolve_hwnd_by_pid_chain(&list, &[]).is_none());
        assert!(resolve_hwnd_by_pid_chain(&[], &[11332]).is_none());
    }

    #[test]
    fn test_ancestor_chain_with_basic_and_cycle() {
        // 简单链 5→4→3→2，深度封顶 4
        let parents = std::collections::HashMap::from([(5u32, 4u32), (4, 3), (3, 2)]);
        assert_eq!(
            ancestor_chain_with(|p| parents.get(&p).copied(), 5, 4),
            vec![5, 4, 3, 2]
        );
        // 无父进程：链只有自身
        assert_eq!(ancestor_chain_with(|_| None, 7, 4), vec![7]);
        // 防环：5→4→5 必须终止且不重复
        let cyc = std::collections::HashMap::from([(5u32, 4u32), (4, 5)]);
        assert_eq!(
            ancestor_chain_with(|p| cyc.get(&p).copied(), 5, 8),
            vec![5, 4]
        );
        // 深度封顶：链再深也只取 max_depth 级
        let deep = std::collections::HashMap::from([(1u32, 2u32), (2, 3), (3, 4), (4, 5), (5, 6)]);
        assert_eq!(
            ancestor_chain_with(|p| deep.get(&p).copied(), 1, 2),
            vec![1, 2, 3]
        );
    }
}
