//! UI 心跳命令——主界面（webview）活性信号。
//!
//! 背景：Tauri 的 `emit` 在 webview 渲染进程死亡后**仍然返回 Ok**（投递失败发生
//! 在 wry 消息循环内，表现为 HRESULT 0x8007139F 错误洪水），Rust 侧没有任何
//! 返回信号。前端每 3s 调一次本命令即构成活性证明；停摆超过 90s（见
//! `emitter::UI_DEAD_AFTER_MS`）由事件发送侧判定失联并进入降级模式
//! （丢非关键事件 + 切窗口标题 + HUD 提示）。
//!
//! 阈值 90s 的依据：浏览器对后台/最小化页签定时器有节流（Chrome ~1 分钟一次），
//! 阈值必须大于该间隔才不会把「窗口最小化」误判为「界面已死」。

use tauri::AppHandle;

/// 前端心跳。`state` 用于在「死而复生」时复用共享 seq 计数器推送恢复通知。
#[tauri::command(async)]
pub fn ui_heartbeat(app: AppHandle, state: tauri::State<'_, crate::state::AppState>) {
    let liveness = crate::emitter::ui_liveness();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if liveness.record_ping(now) {
        // 死而复生：还原标题 + 通知前端同步（前端收到 ui_reconnected 会 reload）。
        crate::emitter::CompoundEmitter::new(app, &state)
            .tauri
            .on_ui_revived();
    }
}
