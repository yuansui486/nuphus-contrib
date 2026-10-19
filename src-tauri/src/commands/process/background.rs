//! background — 后台任务（跨工具调用存活的子进程）管理命令
//!
//! 数据源只有一个：`nuphus::tools::background_tasks::global()`（进程级单例账本）。
//! 本文件**只做传输**，不做任何状态推断——面板/上层看到什么完全取决于账本。
//!
//! 语义固定：
//! - 列举：只返回**存活**条目（死条目在读取时被顺带清理），每项含命令 / pid /
//!   已跑时长 / retain / 输出文件位置。
//! - 结束：按 id 终止该任务的**整个进程树**。这是显式意图，因此**包含保留项**
//!   ——与「取消只杀前台」的自动路径互为对照。**与强制中断解耦**：不置全局
//!   `cancel_flag`，正在执行的轮次不受面板操作影响；终止事实写入该任务的
//!   输出日志，等待其结果的 Agent 由此读到明确结局，不会误判重跑。

use nuphus::tools::background_tasks::{self, BackgroundTaskView};
use tauri::State;

use crate::state::AppState;

/// List still-running background tasks (dead entries are pruned on read).
#[tauri::command(async)]
pub fn list_background_tasks() -> Result<Vec<BackgroundTaskView>, String> {
    let now = background_tasks::now_ms();
    Ok(background_tasks::global()
        .list()
        .into_iter()
        .map(|t| t.view(now))
        .collect())
}

/// Terminate one background task (including retained ones) by its registry id.
///
/// **语义边界（与强制中断解耦）**：这是「后台任务面板」的手动结束——只杀该任务
/// 的进程树并向其输出日志附加终止事实。**不置**全局 `cancel_flag`：面板操作
/// 不表达「中断整轮执行」的意图；后者是输入框终止按钮（强制终止）的职责，
/// 两者互不牵连。
#[tauri::command(async)]
pub fn kill_background_task(_state: State<'_, AppState>, id: String) -> Result<String, String> {
    let outcome = background_tasks::global().kill_one(&id)?;
    let verb = if outcome.already_gone {
        "已自行结束（清理账本条目）"
    } else {
        "已终止（含子孙进程）"
    };
    tracing::info!(
        "[BG] kill_background_task {}: pid={} {}",
        id,
        outcome.task.pid,
        verb
    );
    Ok(format!(
        "后台任务 {}（{}，PID {}）{}；终止报告已写入其输出日志，Agent 侧不会误判为失败重跑",
        id, outcome.task.command, outcome.task.pid, verb
    ))
}

/// Count of still-running **retained** background tasks.
///
/// 单列一个命令的理由：中断汇报需要「保留 M 个」这个数字，而列举是重操作
/// （每条目一次存活探测）。这里仍走同一本账，只是过滤后计数。
#[tauri::command(async)]
pub fn count_retained_background_tasks(_state: State<'_, AppState>) -> Result<usize, String> {
    Ok(background_tasks::global().retained_count())
}
