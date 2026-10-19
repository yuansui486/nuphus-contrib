use nuphus::agent::events::{EventEmitter, NuphusEvent};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};

/// 前端心跳超时：超过该时长未收到心跳即判定主界面失联。
///
/// 90s 的依据：浏览器对后台/最小化页签的定时器有节流（Chrome 最低约 1 分钟
/// 一次），阈值必须大于这个间隔，否则「用户把窗口最小化」会被误判成
/// 「界面死了」而误丢事件。代价是真正死亡后最长 90s 才触发节流——这段时间的
/// 错误洪水由日志层限流（`log_collapse`）兜住，不会刷屏。
const UI_DEAD_AFTER_MS: u64 = 90_000;
/// 主窗口标题（tauri.conf.json main.title）。失联时切换为带告警的标题，
/// 恢复时还原——标题是**原生窗口属性**，webview 死了也改得动、看得见，
/// 是「界面已死」时唯一保证可达的用户可见通道。
const MAIN_WINDOW_TITLE: &str = "Nuphus - 桌面智能助手";
const MAIN_WINDOW_TITLE_DEAD: &str = "Nuphus - 界面连接中断（任务仍在后台运行）";
/// 失联/恢复时 HUD 提示文案（HUD 是独立窗口，主 webview 死后可能仍可见）。
const HUD_DEAD_TEXT: &str = "界面连接中断，任务仍在后台执行";

/// 窗口置前但【不抢键盘焦点】。
///
/// 大王要求：执行完成窗口自动置前（能看到结果），但输入框不自动聚焦
/// （不打断用户在其他窗口打字）。Tauri 的 set_focus() 会同时置前+激活抢焦点，
/// 无 NOACTIVATE 变体——这里用原生 SetWindowPos(HWND_TOP, SWP_NOACTIVATE) 实现
/// 「z-order 提升到最前，但不改变键盘焦点」。
#[cfg(target_os = "windows")]
fn bring_to_front_no_activate<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    let _ = win.show();
    let _ = win.unminimize();
    if let Ok(hwnd) = win.hwnd() {
        // tauri hwnd() 返回 windows crate 的 HWND(pub *mut c_void)，取 .0 原始指针
        let raw: *mut core::ffi::c_void = hwnd.0;
        unsafe {
            let _ = SetWindowPos(
                raw,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn bring_to_front_no_activate<R: tauri::Runtime>(win: &tauri::WebviewWindow<R>) {
    // 非 Windows 平台无 NOACTIVATE 语义，退化为 show（不抢焦点即可）
    let _ = win.show();
    let _ = win.unminimize();
}

#[derive(Debug, Clone, Serialize)]
pub struct FramedEvent {
    pub seq: u64,
    #[serde(rename = "event")]
    pub inner: NuphusEvent,
}

// ═══════════════════════════════════════════════════
// UI 失联检测（heartbeat liveness）
// ═══════════════════════════════════════════════════

/// 主界面（webview）心跳活性状态。
///
/// 为什么需要它：Tauri 的 `app.emit` 是**异步投递**——webview 渲染进程死亡后
/// 调用方拿到的是 `Ok`，失败发生在 wry 消息循环里（HRESULT 0x8007139F 刷屏），
/// Rust 侧没有任何返回信号。唯一可靠的死亡信号是**前端心跳停摆**：
/// 前端每 3s 调一次 `ui_heartbeat`，超过 [`UI_DEAD_AFTER_MS`] 没有心跳即判失联。
///
/// 职责边界：本结构只做**纯状态判定**（可单测、无 Tauri 依赖）；副作用
/// （标题切换、HUD 提示、恢复通知）由持有 `AppHandle` 的调用方执行。
#[derive(Debug, Default)]
pub struct UiLiveness {
    /// 最近一次前端心跳（Unix ms）；0 = 从未收到（启动前，不判定死亡）。
    last_ping_ms: AtomicU64,
    /// 当前是否处于「已判定失联」。
    dead: AtomicBool,
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl UiLiveness {
    pub fn new() -> Self {
        Self::default()
    }

    /// 前端心跳到达。返回 `true` 表示这是一次**死而复生**（此前已判定失联），
    /// 调用方应执行恢复动作（还原标题 + 通知前端同步）。
    pub fn record_ping(&self, now_ms: u64) -> bool {
        self.last_ping_ms.store(now_ms, Ordering::SeqCst);
        self.dead.swap(false, Ordering::SeqCst)
    }

    /// 到点判定：从未收到心跳（前端还没加载完）不判死；超时未 ping 则置失联位。
    /// 返回 `true` 表示本次刚刚发生 alive→dead 迁移（只做一次副作用）。
    pub fn declare_dead_if_due(&self, now_ms: u64) -> bool {
        let last = self.last_ping_ms.load(Ordering::SeqCst);
        if last == 0 || now_ms.saturating_sub(last) <= UI_DEAD_AFTER_MS {
            return false;
        }
        // 只有第一次迁移返回 true（compare_exchange 防并发双触发）。
        self.dead
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    /// 休眠/挂起唤醒后的基线重置：挂起期间的心跳空档不是「界面死亡」的证据。
    /// 返回 `true` 表示此前已判定失联（标题已切到告警态，调用方需还原）。
    pub fn reset_after_resume(&self, now_ms: u64) -> bool {
        self.last_ping_ms.store(now_ms, Ordering::SeqCst);
        self.dead.swap(false, Ordering::SeqCst)
    }

    /// 失联期间是否丢弃该事件（见 [`NuphusEvent::is_essential_when_ui_dead`]）。
    pub fn should_drop(&self, event: &NuphusEvent) -> bool {
        self.is_dead() && !event.is_essential_when_ui_dead()
    }
}

static UI_LIVENESS: OnceLock<Arc<UiLiveness>> = OnceLock::new();

/// 进程级 UI 活性状态（一个应用实例一份）。
pub fn ui_liveness() -> Arc<UiLiveness> {
    UI_LIVENESS
        .get_or_init(|| Arc::new(UiLiveness::new()))
        .clone()
}

/// alive→dead 迁移的可见副作用：日志 + 原生窗口标题 + HUD 提示。
///
/// 独立于 `TauriEventEmitter`：emit 路径与**看门狗线程**（事件静默期也要判死，
/// 见 `ui_watchdog`）共用同一套动作，避免两处漂移。
pub fn perform_ui_dead_actions<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    tracing::error!(
        "[UI-LINK] main webview considered dead (no frontend heartbeat for {}s) — \
         switching to degraded mode: non-essential events dropped",
        UI_DEAD_AFTER_MS / 1000
    );
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.set_title(MAIN_WINDOW_TITLE_DEAD);
    }
    crate::commands::hud::show(app, HUD_DEAD_TEXT, "warning");
}

/// 休眠/挂起唤醒后的基线重置（看门狗调用）：挂起期间的心跳空档不算失联。
/// 若此前已判定失联，还原标题（与 `on_ui_revived` 同一收尾，但不推 `ui_reconnected`
/// ——前端从未失联，只是系统睡着了）。
pub fn perform_ui_resume_reset<R: tauri::Runtime>(app: &tauri::AppHandle<R>, now_ms: u64) -> bool {
    let was_dead = ui_liveness().reset_after_resume(now_ms);
    if was_dead {
        tracing::warn!(
            "[UI-LINK] suspend/resume: heartbeat gap is expected — liveness baseline reset, title restored"
        );
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.set_title(MAIN_WINDOW_TITLE);
        }
    }
    was_dead
}

// Runtime 泛化（默认 Wry）：生产路径零改动；测试可用 MockRuntime 构造完整事件链。
// Clone 手写实现——derive 会额外要求 R: Clone（MockRuntime 不满足）。
pub struct TauriEventEmitter<R: tauri::Runtime = tauri::Wry> {
    pub app: tauri::AppHandle<R>,
    pub seq: Arc<AtomicU64>,
}

impl<R: tauri::Runtime> Clone for TauriEventEmitter<R> {
    fn clone(&self) -> Self {
        Self {
            app: self.app.clone(),
            seq: self.seq.clone(),
        }
    }
}

impl<R: tauri::Runtime> TauriEventEmitter<R> {
    /// alive→dead 迁移时执行一次：日志、主窗口标题（原生通道，webview 死了也可见）、
    /// HUD 提示（独立窗口，可能还活着）。
    fn on_ui_dead(&self) {
        perform_ui_dead_actions(&self.app);
    }

    /// dead→alive 迁移时执行一次：还原标题，并推一条 `ui_reconnected` 警告，
    /// 前端据此 reload 同步状态（失联期间被丢弃的事件不补偿——历史由 DB 重建）。
    pub fn on_ui_revived(&self) {
        tracing::warn!("[UI-LINK] frontend heartbeat resumed — UI reconnected, restoring");
        if let Some(main) = self.app.get_webview_window("main") {
            let _ = main.set_title(MAIN_WINDOW_TITLE);
        }
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        let _ = self.app.emit(
            "nuphus-event",
            FramedEvent {
                seq,
                inner: NuphusEvent::Warning {
                    code: "ui_reconnected".to_string(),
                    message: "界面连接已恢复，正在同步状态…".to_string(),
                    attempt: None,
                    max_attempts: None,
                },
            },
        );
    }
}

impl<R: tauri::Runtime> EventEmitter for TauriEventEmitter<R> {
    fn emit(&self, event: NuphusEvent) {
        // ── UI 失联检测与事件节流 ──
        // 主 webview 死亡后 Tauri 的 emit 仍返回 Ok（失败在 wry 消息循环内），
        // 唯一信号是前端心跳停摆。到点即判失联：切标题 + HUD 提示（一次性），
        // 之后丢弃一切非关键事件——它们正是「往死 webview 狂发 JS」的
        // 错误洪水来源（线上实测 46 条/秒，像崩溃一样刷屏）。
        let liveness = ui_liveness();
        if liveness.declare_dead_if_due(now_unix_ms()) {
            self.on_ui_dead();
        }
        if liveness.should_drop(&event) {
            return;
        }

        let seq = self.seq.fetch_add(1, Ordering::SeqCst);

        // ── 窗口可见性管理 ──
        // HudUpdate: 恢复 HUD 窗口尺寸+位置，确保可见（创建时 visible=false）
        // UserInputRequest: 确保主窗口在前台（用户能看到输入弹窗）
        // ExecutionCompleted: Leader/工作流完成时主窗口置顶
        match &event {
            NuphusEvent::HudUpdate { text, phase, .. } => {
                // Delegate to shared show/hide logic (off-screen positioning,
                // force-close fallback, etc.)
                if phase == "hidden" || phase == "hide" {
                    crate::commands::hud::hide(&self.app);
                } else {
                    crate::commands::hud::show(&self.app, text, phase);
                }
            }
            NuphusEvent::UserInputRequest { .. } => {
                if let Some(main) = self.app.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
            }
            NuphusEvent::SecurityCheck { tool, .. }
                if tool == nuphus::tools::desktop_approval::EVENT_TOOL =>
            {
                if let Some(main) = self.app.get_webview_window("main") {
                    bring_to_front_no_activate(&main);
                }
            }
            NuphusEvent::ExecutionCompleted { .. } => {
                // 不做窗口激活——子任务完成（sub_task_loop）和全局完成
                // 都会发此事件。不做 show/focus，避免每次工具调用都抢前台。
            }
            NuphusEvent::LeaderDone { .. } => {
                // 窗口置前但【不抢键盘焦点】：执行完成窗口自动跑到最前
                // （大王要求保留置前），但输入框不自动聚焦、不打断其他窗口打字
                // （2026-08-08 大王要求移除执行完自动聚焦）。
                if let Some(main) = self.app.get_webview_window("main") {
                    bring_to_front_no_activate(&main);
                }
            }
            _ => {}
        }

        let _ = self
            .app
            .emit("nuphus-event", FramedEvent { seq, inner: event });
    }
}
// ============================================================================
// CompoundEmitter — 事件双推（桌面 Tauri + 手机 WS）
// ============================================================================

/// 移动端 WS 推送端：序列化事件后经 broadcast channel 发给所有已连接的手机客户端。
/// 复活的旧 GatewayEventEmitter 模式（2fd603e 删除），单 mpsc 升级为 broadcast（多客户端）。
#[derive(Clone)]
pub struct MobileWsEmitter {
    tx: tokio::sync::broadcast::Sender<String>,
}

impl MobileWsEmitter {
    pub fn new(tx: tokio::sync::broadcast::Sender<String>) -> Self {
        Self { tx }
    }
}

impl EventEmitter for MobileWsEmitter {
    fn emit(&self, event: NuphusEvent) {
        let json = match serde_json::to_string(&event) {
            Ok(json) => json,
            Err(e) => {
                tracing::error!("[MobileWsEmitter] serialize error: {}", e);
                return;
            }
        };
        // 无接收者（手机未连接）是常态，静默丢弃；channel 满时 lagged 由订阅侧处理
        let _ = self.tx.send(json);
    }
}

/// 复合 emitter：事件同时推桌面（Tauri IPC）与手机（WebSocket broadcast）。
/// `mobile` 为 None（mobile_server 未启动）时退化为纯 Tauri 推送，行为与
/// 单独使用 TauriEventEmitter 完全等价（桌面零回归保证）。
///
/// 注意：构造时 seq 只取一次快照——Tauri 端 seq 由 TauriEventEmitter 内部
/// 维护共享计数，WS 端推送的是裸 NuphusEvent JSON（不含 seq 帧头），
/// 手机客户端如需排序可在 P2 自行包装。
pub struct CompoundEmitter<R: tauri::Runtime = tauri::Wry> {
    pub tauri: TauriEventEmitter<R>,
    pub mobile: Option<MobileWsEmitter>,
}

impl<R: tauri::Runtime> Clone for CompoundEmitter<R> {
    fn clone(&self) -> Self {
        Self {
            tauri: self.tauri.clone(),
            mobile: self.mobile.clone(),
        }
    }
}

impl<R: tauri::Runtime> CompoundEmitter<R> {
    /// 从 AppState 当前状态构造：server 运行（mobile_ws_tx = Some）→ 双推；否则纯 Tauri。
    pub fn new(app: tauri::AppHandle<R>, state: &crate::state::AppState) -> Self {
        let mobile = state
            .mobile_ws_tx
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(MobileWsEmitter::new);
        Self {
            tauri: TauriEventEmitter {
                app,
                seq: state.event_seq.clone(),
            },
            mobile,
        }
    }
}

impl<R: tauri::Runtime> EventEmitter for CompoundEmitter<R> {
    fn emit(&self, event: NuphusEvent) {
        self.tauri.emit(event.clone());
        if let Some(ref m) = self.mobile {
            m.emit(event);
        }
    }
}

#[cfg(test)]
mod ui_liveness_tests {
    use super::*;

    fn noise() -> NuphusEvent {
        NuphusEvent::LlmTextDelta {
            text: "x".into(),
            is_thinking: false,
            from_task: false,
        }
    }

    fn essential() -> NuphusEvent {
        NuphusEvent::HudUpdate {
            text: "t".into(),
            phase: "running".into(),
            step_kind: None,
        }
    }

    /// 没收到过心跳（前端还没加载完）→ 永不判死，行为与旧版一致。
    #[test]
    fn no_ping_never_declares_dead() {
        let lv = UiLiveness::new();
        assert!(!lv.declare_dead_if_due(1_000_000_000));
        assert!(!lv.should_drop(&noise()), "启动前不得丢事件");
    }

    /// 心跳停摆超阈值 → 判死并丢进度事件；边界内不误判。
    #[test]
    fn stall_declares_dead_and_drops_noise() {
        let base = 1_000_000u64; // 非 0：0 是「从未心跳」哨兵
        let lv = UiLiveness::new();
        lv.record_ping(base);
        assert!(
            !lv.declare_dead_if_due(base + UI_DEAD_AFTER_MS),
            "阈值边界内（刚好等于）不得判死"
        );
        assert!(
            lv.declare_dead_if_due(base + UI_DEAD_AFTER_MS + 1),
            "刚过阈值应判死，并报告一次性迁移"
        );
        assert!(lv.is_dead(), "状态位应保持失联");
        assert!(lv.should_drop(&noise()), "失联后进度事件必须丢弃");
        assert!(!lv.should_drop(&essential()), "失联后关键事件必须放行");
    }

    /// 首次 alive→dead 迁移只触发一次（副作用只做一遍）。
    #[test]
    fn dead_transition_fires_once() {
        let base = 1_000_000u64;
        let lv = UiLiveness::new();
        lv.record_ping(base);
        assert!(
            lv.declare_dead_if_due(base + UI_DEAD_AFTER_MS + 10),
            "首次超时应返回 true（调用方执行一次性副作用）"
        );
        assert!(
            !lv.declare_dead_if_due(base + UI_DEAD_AFTER_MS + 20_000),
            "重复调用不得重复触发副作用"
        );
    }

    /// 心跳恢复 → record_ping 返回 true（死而复生），状态复位、恢复放行。
    #[test]
    fn ping_revives_and_resumes() {
        let base = 1_000_000u64;
        let lv = UiLiveness::new();
        lv.record_ping(base);
        lv.declare_dead_if_due(base + UI_DEAD_AFTER_MS + 10);
        assert!(lv.is_dead());
        assert!(
            lv.record_ping(base + UI_DEAD_AFTER_MS + 20_000),
            "复活心跳须报告迁移"
        );
        assert!(!lv.is_dead(), "复活后状态位必须复位");
        assert!(!lv.should_drop(&noise()), "复活后恢复放行");
        assert!(
            !lv.record_ping(base + UI_DEAD_AFTER_MS + 30_000),
            "常规心跳不得报告复活"
        );
    }
}
