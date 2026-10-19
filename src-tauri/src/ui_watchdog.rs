//! 主界面自愈看门狗——把用户从「透明/空白窗口」里救出来。
//!
//! ## 为什么需要它（2026-10-15 用户报告）
//!
//! Win11 上窗口长时间闲置后会变成「透明窗口」。已核实的同类机制（Tauri 生态实测案例）：
//! **WebView2 浏览器进程退出后，宿主窗口只剩一个裸的原生窗口**——如果宿主不处理
//! WebView2 的 process-failed，它就永不恢复；transparent 窗口更是直接透出桌面壁纸。
//! 本仓此前对界面失联只做「日志 + 窗口标题 + HUD 提示」（`emitter::perform_ui_dead_actions`），
//! **没有任何恢复动作**——用户只能重启应用。
//!
//! ## 两级判死 + 三级阶梯
//!
//! - 判死信号一（既有）：前端心跳停摆 > `UI_DEAD_AFTER_MS`（90s）。此判定原先只在
//!   事件发送路径上顺手做，**事件静默期（窗口闲置）不巡检** → 闲置场景永远不判死。
//!   本看门狗补上定时巡检。
//! - 判死信号二（本模块）：墙钟与单调钟的增速差——系统挂起（合盖/睡眠/待机）期间
//!   单调钟不走、墙钟照走，用两者的差值把「刚被唤醒」和「界面真的死了」分开：
//!   唤醒后的心跳空档是预期行为，**重置基线而不是弹恢复对话框**（否则每次合盖都误报）。
//! - 恢复阶梯：①已判死 → ②再等 `GRACE_RELOAD_MS` 仍失联 → 自动重载主界面一次
//!   （治渲染进程挂死） → ③再等 `GRACE_NOTIFY_MS` 仍失联 → 原生对话框给出
//!   「重新加载界面 / 重启应用」（治浏览器进程退出这类重载救不回来的死法）。
//!
//! 对话框走 `tauri-plugin-dialog`（原生 Win32 对话框，不经 webview），界面死了也弹得出来。

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

/// 巡检间隔。
const TICK_MS: u64 = 15_000;
/// 墙钟增量 - 单调钟增量 超过该值 → 期间系统挂起过（唤醒而非死亡）。
const RESUME_GAP_MS: u64 = 30_000;
/// 判死后先等这么久仍失联才自动重载（给「系统刚唤醒、前端还没来得及补心跳」留余量）。
const GRACE_RELOAD_MS: u64 = 30_000;
/// 重载后再等这么久仍失联才弹对话框（重载要跑完一轮页面加载）。
const GRACE_NOTIFY_MS: u64 = 60_000;

/// 主线程停摆判定阈值：Windows 约 5s 未泵消息就把窗口标成「无响应」，
/// 这里取 3s 提前留证——**任何**原因（我们的命令、插件、将来的新代码）卡住
/// 主线程都会留下日志，不再只能靠用户报「无响应」。
const MAIN_THREAD_STALL_MS: Duration = Duration::from_secs(3);

/// 恢复阶梯的阶段（只前进，失联解除即回到 `Watching`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealStage {
    /// 已判死，等待宽限期。
    Watching,
    /// 已自动重载一次，等待其生效。
    Reloaded,
    /// 已弹出恢复对话框（不再重复打扰）。
    Notified,
}

fn now_wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 纯判定：墙钟与单调钟的增速差是否说明期间系统挂起过。
///
/// 正常运行时两钟同速（差≈0）；挂起期间单调钟停走而墙钟继续，差值≈挂起时长。
fn is_resume_gap(wall_delta_ms: u64, mono_delta_ms: u64) -> bool {
    wall_delta_ms.saturating_sub(mono_delta_ms) > RESUME_GAP_MS
}

/// 纯判定：按失联持续时长推进阶梯（一次只推进一格）。
fn advance(stage: HealStage, dead_for_ms: u64) -> Option<HealStage> {
    match stage {
        HealStage::Watching if dead_for_ms >= GRACE_RELOAD_MS => Some(HealStage::Reloaded),
        HealStage::Reloaded if dead_for_ms >= GRACE_RELOAD_MS + GRACE_NOTIFY_MS => {
            Some(HealStage::Notified)
        }
        _ => None,
    }
}

/// 启动看门狗线程。线程创建失败只是「失去自愈能力」，绝不影响应用启动。
pub fn spawn(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("ui-watchdog".into())
        .spawn(move || run(app));
    if let Err(e) = spawned {
        tracing::warn!("[UI-WATCHDOG] 看门狗线程创建失败，界面失联将不再自愈：{e}");
    }
}

fn run(app: AppHandle) {
    let start_mono = Instant::now();
    let mut last_wall = now_wall_ms();
    let mut last_mono = 0u64;
    let mut stage = HealStage::Watching;
    let mut dead_since: Option<u64> = None;

    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        let wall = now_wall_ms();
        let mono = start_mono.elapsed().as_millis() as u64;
        let wall_delta = wall.saturating_sub(last_wall);
        let mono_delta = mono.saturating_sub(last_mono);
        last_wall = wall;
        last_mono = mono;

        // ① 系统刚被唤醒：心跳空档是预期行为，重置基线（不判死、不弹窗）。
        if is_resume_gap(wall_delta, mono_delta) {
            crate::emitter::perform_ui_resume_reset(&app, wall);
            stage = HealStage::Watching;
            dead_since = None;
            continue;
        }

        // ② 判死（事件静默期也要判——这是闲置场景唯一能触达的巡检）。
        let liveness = crate::emitter::ui_liveness();
        if liveness.declare_dead_if_due(wall) {
            crate::emitter::perform_ui_dead_actions(&app);
        }

        // ②′ 主线程停摆探针（覆盖一切未知来源的 UI 冻结）。
        probe_main_thread(&app);

        // ③ 恢复阶梯。
        if liveness.is_dead() {
            let since = *dead_since.get_or_insert(wall);
            let dead_for = wall.saturating_sub(since);
            if let Some(next) = advance(stage, dead_for) {
                stage = next;
                match next {
                    HealStage::Reloaded => reload_main(&app, dead_for),
                    HealStage::Notified => notify_user(&app, dead_for),
                    HealStage::Watching => {}
                }
            }
        } else {
            stage = HealStage::Watching;
            dead_since = None;
        }
    }
}

/// 主线程停摆探针：向主线程投一条空任务，测量它被执行前的排队时长。
///
/// 正常时主线程在毫秒级执行该任务；若超过 [`MAIN_THREAD_STALL_MS`] 仍未执行，
/// 说明主线程被某个同步工作占住（命令体在阻塞上下文、长事务、插件回调……），
/// 此刻用户看到的就是「无响应」。日志里带上连接池可用数，便于区分
/// 「DB 争用」与「其它主线程工作」。
fn probe_main_thread(app: &AppHandle) {
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    if app
        .run_on_main_thread(move || {
            let _ = tx.send(());
        })
        .is_err()
    {
        return; // 主线程已不可用（应用正在退出）
    }
    if rx.recv_timeout(MAIN_THREAD_STALL_MS).is_err() {
        tracing::error!(
            "[UI-WATCHDOG] 主线程停摆 >{}ms（run_on_main_thread 未在时限内执行）——\
             界面此刻无响应。连接池可用 {}；排查是否有同步命令体/长事务占住主线程。",
            MAIN_THREAD_STALL_MS.as_millis(),
            nuphus::store::db::pool_available()
        );
    }
}

/// 阶梯②：自动重载主界面一次（治渲染进程挂死/前端 JS 停摆）。
fn reload_main(app: &AppHandle, dead_for_ms: u64) {
    match app.get_webview_window("main") {
        Some(main) => match main.reload() {
            Ok(()) => tracing::warn!(
                "[UI-WATCHDOG] 界面失联 {}s，已自动重载主界面（等页面重新加载）",
                dead_for_ms / 1000
            ),
            Err(e) => tracing::error!(
                "[UI-WATCHDOG] 主界面重载失败：{e}（webview 可能已退出，升级到人工恢复）"
            ),
        },
        None => tracing::error!("[UI-WATCHDOG] main 窗口不存在，无法重载"),
    }
}

/// 阶梯③：原生对话框给出人工恢复出口（重载救不回来的死法：浏览器进程退出等）。
///
/// 三按钮语义：`Yes` = 重新加载界面 / `No` = 重启应用 / `Cancel`（含关窗）= 稍后。
/// 关窗不落到「重启」分支——误触代价太大。
fn notify_user(app: &AppHandle, dead_for_ms: u64) {
    tracing::error!(
        "[UI-WATCHDOG] 界面失联 {}s 且自动重载未恢复（webview 浏览器进程可能已退出）——弹出恢复对话框",
        dead_for_ms / 1000
    );
    let handle = app.clone();
    app.dialog()
        .message(
            "界面进程已失去响应：窗口可能显示为空白或透明。\n\n\
             可以尝试重新加载界面；若仍然无效，请重启应用。",
        )
        .title("Nuphus 界面连接中断")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            "重新加载界面".to_string(),
            "重启应用".to_string(),
            "稍后".to_string(),
        ))
        .show_with_result(move |res| match res {
            MessageDialogResult::Yes => {
                if let Some(main) = handle.get_webview_window("main") {
                    let _ = main.reload();
                }
                tracing::warn!("[UI-WATCHDOG] 用户选择重新加载界面");
            }
            MessageDialogResult::No => {
                tracing::warn!("[UI-WATCHDOG] 用户选择重启应用");
                handle.restart();
            }
            _ => tracing::info!("[UI-WATCHDOG] 用户选择稍后处理界面失联"),
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_gap_detected_only_on_clock_drift() {
        // 正常巡检：两钟同速
        assert!(!is_resume_gap(TICK_MS, TICK_MS));
        // 挂起 10 分钟：墙钟走了 600s，单调钟只走了 15s
        assert!(is_resume_gap(600_000, TICK_MS));
        // 漂移未到阈值（25s < 30s）：不当作唤醒
        assert!(!is_resume_gap(40_000, TICK_MS));
        // 时钟回拨（墙钟增量为 0）不应误判为唤醒
        assert!(!is_resume_gap(0, TICK_MS));
    }

    #[test]
    fn heal_ladder_advances_one_step_at_a_time() {
        assert_eq!(advance(HealStage::Watching, 10_000), None);
        assert_eq!(
            advance(HealStage::Watching, GRACE_RELOAD_MS),
            Some(HealStage::Reloaded)
        );
        // 已重载：未到弹窗时限不再动作（避免重复弹窗）
        assert_eq!(advance(HealStage::Reloaded, GRACE_RELOAD_MS), None);
        assert_eq!(
            advance(HealStage::Reloaded, GRACE_RELOAD_MS + GRACE_NOTIFY_MS),
            Some(HealStage::Notified)
        );
        // 已通知：不再重复打扰用户
        assert_eq!(advance(HealStage::Notified, 10_000_000), None);
    }
}
