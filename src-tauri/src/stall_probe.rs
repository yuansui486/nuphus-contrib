//! 主线程停摆探针——「界面无响应」的留证工具（2026-10-16 主线程阻塞排查引入）。
//!
//! 向主线程投一条空任务（`run_on_main_thread`），测量它被执行前的排队时长：
//! Windows 约 5s 未泵消息就把窗口标成「无响应」，这里取 3s 提前留证——**任何**
//! 原因（我们的命令、插件、将来的新代码）卡住主线程都会留下日志，不再只能靠
//! 用户报「无响应」。日志里带上连接池可用数，便于区分「DB 争用」与「其它主线程
//! 工作」。
//!
//! 本探针**只打日志，不做任何干预**（无重载、无弹窗、无状态迁移）——它是诊断
//! 工具而非自愈机制。

use std::time::Duration;
use tauri::AppHandle;

/// 巡检间隔：独立 OS 线程周期巡检（不占 tokio worker，也不受前端事件循环影响）。
const TICK_MS: u64 = 15_000;
/// 主线程停摆判定阈值：Windows 约 5s 未泵消息就把窗口标成「无响应」，
/// 这里取 3s 提前留证——**任何**原因（我们的命令、插件、将来的新代码）卡住
/// 主线程都会留下日志，不再只能靠用户报「无响应」。
const MAIN_THREAD_STALL_MS: Duration = Duration::from_secs(3);

/// 启动探针线程。线程创建失败只是「失去停摆留证」，绝不影响应用启动。
pub fn spawn(app: AppHandle) {
    let spawned = std::thread::Builder::new()
        .name("stall-probe".into())
        .spawn(move || run(app));
    if let Err(e) = spawned {
        tracing::warn!("[STALL-PROBE] 探针线程创建失败，主线程停摆将不再留证：{e}");
    }
}

fn run(app: AppHandle) {
    loop {
        std::thread::sleep(Duration::from_millis(TICK_MS));
        probe_main_thread(&app);
    }
}

/// 主线程停摆探针：向主线程投一条空任务，测量它被执行前的排队时长。
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
            "[STALL-PROBE] 主线程停摆 >{}ms（run_on_main_thread 未在时限内执行）——\
             界面此刻无响应。连接池可用 {}；排查是否有同步命令体/长事务占住主线程。",
            MAIN_THREAD_STALL_MS.as_millis(),
            nuphus::store::db::pool_available()
        );
    }
}
