//! Mode 命令 — 获取/切换运行模式

use crate::emitter::CompoundEmitter;
use crate::state::AppState;
use nuphus::agent::events::{EventEmitter, NuphusEvent};
use std::str::FromStr;
use tauri::State;

/// 设置运行模式（"leader" / "workflow"）
///
/// 无效模式（含旧版残留的 "free"/"plan"）fallback 到 Leader 默认值，
/// 不报错——旧持久化数据不得导致功能不可用。
///
/// 模式切换：
/// - 跨 mode 切换（2026-10-14 会话生命周期显式化）：对**离开的 mode** 的 runtime 会话
///   执行 `archive_active` 归档进 session 工作台（is_active=false 历史条目）+ 清空该槽
///   agent（回 welcome）——旧行为只改 mode 标志，槽内会话对工作台隐身、切回原地复活。
///   切回原 mode 后为 welcome，恢复只能经工作台 switch_session 显式进行，无任何自动复活。
/// - 切到 Workflow: 目标槽不动（workflow_agent 若存在则保留，否则懒初始化在
///   send_message_cmd 中完成）
/// - 切到 Leader/Custom: 对存活的 leader_agent 同步 set_mode
/// - 同 mode 重复调用（前端启动同步等）：不转场，保持幂等
///
/// 泛型核心：桌面 IPC（具体 Wry thin wrapper `set_mode`）与手机端 mobile_server
/// （泛型 Runtime）共用同一实现；`#[tauri::command]` 由下方 thin wrapper 提供。
pub async fn set_mode_impl<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    // ── 执行态判定（复用唯一权威执行态，不新增状态源）──
    // 执行中（Running/Finalizing）不得改 mode：主指令受理时已按「发送时刻 mode」建立/
    // 绑定当前 session（process.rs 规则2），执行中改写会让
    //  ① append 分支的 mode 兜底刷新（process.rs 规则3）把当前轮次的路由改掉；
    //  ② chat_history 按 current_mode 选 agent 会话 → 读到另一条会话的历史。
    // 桌面端 mode chip / 手机端模式列表都在执行期锁定（同一 stage 投影），此处为后端兜底
    // （手机 /switch-mode 与 UI 竞态窗口一并拒掉）。执行结束后（Idle）即可切换。
    if state.busy.stage().is_busy() {
        tracing::warn!("[MODE] 执行期拒绝切换模式: {}（stage 占用中）", mode);
        return Err("任务执行中，暂不可切换模式（执行结束后可切换）".to_string());
    }
    let parsed = nuphus::runtime::Mode::from_str(&mode).unwrap_or_else(|_| {
        tracing::warn!("[MODE] Unknown mode '{}', falling back to leader", mode);
        nuphus::runtime::Mode::default()
    });

    // ── 同 mode 幂等：前端启动同步 / 重复点击目标 mode 不做转场 ──
    // 转场（归档+清槽）只在真实跨 mode 时执行，避免把当前会话误归档。
    {
        let cur = state
            .current_mode
            .read()
            .map(|g| g.clone())
            .unwrap_or_default();
        if crate::commands::process::shelf::normalize_mode(&cur) == parsed.as_str() {
            tracing::info!("[MODE] Switched to: {} (no-op, same mode)", parsed.as_str());
            return Ok(());
        }
    }

    let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;

    // ── 跨 mode 转场：归档离开槽的 active 会话 + 清空该槽 agent（2026-10-14）──
    // from_kind 归一化与 active_session 的读槽规则同构（shelf.rs active_session：
    // kind=="workflow" 读 workflow 槽，else 读 leader 槽——custom 归 leader 槽）。
    // archive_active：快照 → write_mirror → upsert_meta_row → shelf.put（完整保留，
    // 含超容量淘汰与镜像保护）；空会话（无消息）内部直接跳过，不产生工作台条目。
    // 清槽 = agent 整槽置 None（welcome 语义，与 new_chat_session 同范式）：Runtime
    // 无需跨槽保留的非 session 状态（工具权限/接线每轮 sync_before_run 重设、mode 每次
    // 发送由前端重 apply），且恢复链路走 switch_session 的 backup 中转（槽 None 时
    // 目标会话进 session_backup，下次发送经 run_runtime_with_config 恢复完整上下文）。
    // 必须在写 current_mode 之前、runtime 锁持有期间完成（锁序：archive_active 内部
    // 已处理好 shelf/db 锁序，此处不额外长持外层锁）。
    let from_mode = state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string());
    let from_kind = crate::commands::process::shelf::normalize_mode(&from_mode);
    crate::commands::process::shelf::archive_active(state.inner(), &mut guard, from_kind);
    match from_kind {
        "workflow" => {
            guard.workflow_agent = None;
            // 增强模式是 Workflow 开发会话状态，不跨会话继承（对齐 new_chat_session）
            state
                .workflow_enhanced_mode
                .store(false, std::sync::atomic::Ordering::SeqCst);
        }
        _ => guard.leader_agent = None,
    }
    // 会话边界残留清理：残留 backup 会破坏发送路径空态判据（slot_empty && backup_none），
    // 让目标 mode 首发把旧会话当续聊上下文恢复（跨 mode 串会话）；last_message 同理
    // 会被 chat_history 的 backup 回退路径拼进新会话首页。对齐 new_chat_session 边界动作。
    if let Ok(mut sb) = state.session.lock() {
        sb.session_backup = None;
        if from_kind == "workflow" {
            sb.pending_workflow_enhanced_mode = None;
        }
        sb.last_message.clear();
        sb.last_send_id = None;
        sb.last_message_images.clear();
    }

    match parsed {
        nuphus::runtime::Mode::Workflow => {
            // WorkflowAgent lazy init in send_message_cmd
            tracing::info!("[MODE] Switched to: workflow");
        }
        nuphus::runtime::Mode::Leader => {
            if let Some(ref mut runtime) = guard.leader_agent {
                runtime.set_mode(parsed);
            }
            tracing::info!("[MODE] Switched to: leader");
        }
        nuphus::runtime::Mode::Custom => {
            // Custom runs on the Leader main loop; L2 comes from active custom card.
            // set_mode triggers prompt-cache invalidation so L2 rebuilds from the card.
            if let Some(ref mut runtime) = guard.leader_agent {
                runtime.set_mode(parsed);
            }
            tracing::info!("[MODE] Switched to: custom");
        }
    }
    drop(guard);

    // 后端权威 current_mode：chat_history 按此选择 agent 会话（替换旧「最近活跃」猜测）。
    // current_mode 是独立 RwLock，与已释放的 runtime 锁无冲突。
    // 切换语义（2026-08-30 解耦后）：手动切换只更新 current_mode，不设任何 pending 状态。
    // 会话归属判定完全由 submit_user_message 实时比较「发送 mode vs session 绑定 mode」
    // 决定（规则2）：不一致 → 新建该 mode 会话；一致 → 续聊当前 session。
    {
        if let Ok(mut cm) = state.current_mode.write() {
            *cm = parsed.as_str().to_string();
        }
    }

    // 广播 mode 变更：双推桌面 Tauri + 手机 WS（mobile_server 未启动时 CompoundEmitter
    // 退化为纯 Tauri 推送，桌面端零回归）。
    // 后端是 mode 的唯一权威源：桌面 set_mode 与手机 /switch-mode 共用此命令，
    // 切换后双端（手机自身 + 桌面端实时一致）同步「当前模式」。
    let emitter = CompoundEmitter::new(app, &state);
    emitter.emit(NuphusEvent::ModeChanged {
        mode: parsed.as_str().to_string(),
    });

    Ok(())
}

/// 桌面 IPC 命令入口（thin wrapper）：委托泛型核心 `set_mode_impl`。
/// 桌面端切换运行模式时由前端 invoke；事件双推（桌面 Tauri + 手机 WS）。
#[tauri::command]
pub async fn set_mode(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> Result<(), String> {
    set_mode_impl(app, state, mode).await
}

/// 获取当前权威 mode：前端启动时调用，使 mode state 与后端镜像恢复结果一致
/// （启动恢复 current_mode from 镜像——leader/workflow/custom 三态）。
#[tauri::command(async)]
pub fn get_current_mode(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state
        .current_mode
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "leader".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::process::shelf;
    use nuphus::session::Session;
    use nuphus::state::ExecutionStage;
    use tauri::Manager;

    // ── 测试 helper：与 session.rs / shelf.rs 测试模块同款 Mock 构造 ──

    struct MockApiClient;
    #[async_trait::async_trait]
    impl nuphus::api::ApiClient for MockApiClient {
        async fn stream(
            &self,
            _request: nuphus::api::MessageRequest,
        ) -> nuphus::Result<Vec<nuphus::api::AssistantEvent>> {
            Ok(vec![])
        }
        fn model_name(&self) -> &str {
            "mock"
        }
        fn provider_kind(&self) -> nuphus::api::ProviderKind {
            nuphus::api::ProviderKind::MiniMax
        }
        fn provider_name(&self) -> &str {
            ""
        }
    }

    /// leader 槽 Runtime + 指定对话内容
    fn leader_runtime_with(sess: Session) -> nuphus::runtime::Runtime {
        let mut rt = nuphus::runtime::RuntimeBuilder::new()
            .llm(std::sync::Arc::new(MockApiClient))
            .tools(nuphus::ToolRegistry::builtin())
            .build()
            .expect("RuntimeBuilder::build 应成功");
        rt.session_mut().replace_messages(sess.messages().to_vec());
        rt
    }

    /// 带 user/assistant 一轮对话的非空 Session
    fn session_with_user(text: &str) -> Session {
        let mut s = Session::new();
        s.push_user(text.to_string());
        s.push_assistant(vec![nuphus::session::ContentBlock::Text {
            text: "好".to_string(),
            reasoning: None,
        }]);
        s
    }

    fn set_current_mode(state: &AppState, mode: &str) {
        let mut cm = state.current_mode.write().unwrap();
        *cm = mode.to_string();
    }

    /// 工作台 items 中找指定 id 的条目（None = 不在台）
    fn shelf_item_of<'a>(
        payload: &'a serde_json::Value,
        id: &str,
    ) -> Option<&'a serde_json::Value> {
        payload["items"]
            .as_array()
            .and_then(|items| items.iter().find(|it| it["id"] == *id))
    }

    /// 测试自清理：sessions 行 + session_meta 行（store 测试同款约定，
    /// archive_active 会写真实 SQLite —— 绝不污染用户数据）
    fn cleanup_rows(ids: &[&str]) {
        for id in ids {
            let _ = nuphus::store::session::delete_session(id);
            if let Ok(conn) = nuphus::store::db::acquire() {
                let _ = conn.execute(
                    "DELETE FROM session_meta WHERE session_id = ?1",
                    rusqlite::params![*id],
                );
            }
        }
    }

    /// 执行期（Running/Finalizing）改 mode 必须被拒；回到 Idle 才能切。
    /// 判据是唯一权威执行态 `execution_stage`，不是另设标记。
    #[test]
    fn set_mode_rejected_while_executing() {
        tokio_test::block_on(async {
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();

            for stage in [ExecutionStage::Running, ExecutionStage::Finalizing] {
                state.busy.set_stage(stage);
                let err = set_mode_impl(
                    handle.clone(),
                    handle.state::<AppState>(),
                    "workflow".into(),
                )
                .await
                .expect_err("执行中改 mode 必须被拒");
                assert!(
                    err.contains("执行中"),
                    "拒绝原因需可读且说明是执行态拦截，实际: {err}"
                );
                // 被拒路径不得改写权威 mode（否则 chat_history 选路与 append 兜底都会错位）
                assert_eq!(*state.current_mode.read().unwrap(), "leader");
            }

            // 收尾结束 → Idle：切换恢复可用，并写入权威 current_mode
            state.busy.set_stage(ExecutionStage::Idle);
            set_mode_impl(
                handle.clone(),
                handle.state::<AppState>(),
                "workflow".into(),
            )
            .await
            .expect("空闲时切换必须成功");
            assert_eq!(*state.current_mode.read().unwrap(), "workflow");
        });
    }

    /// 验收序列①③：leader 有会话 → 切 workflow → 工作台出现 leader 条目
    /// （is_active=false）+ leader 槽清空；切回 leader → leader 条目仍在工作台
    /// （is_active=false）→ 经 switch_session 恢复历史可继续对话（验收序列④）。
    #[test]
    fn mode_switch_archives_leaving_session_and_does_not_revive() {
        tokio_test::block_on(async {
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();

            // leader 槽有会话（非空）。注意 id 取自槽内 agent（RuntimeBuilder::build
            // 创建全新 Session，replace_messages 只换消息体——传入 sess 的 id 不可用，
            // 对齐 shelf.rs 测试「从槽读 id」的既有模式）
            {
                let mut guard = state.runtime.lock().unwrap();
                guard.leader_agent = Some(leader_runtime_with(session_with_user(
                    "帮我看看这个 leader 会话",
                )));
            }
            let leader_session_id = {
                let guard = state.runtime.lock().unwrap();
                guard
                    .leader_agent
                    .as_ref()
                    .expect("前置：leader 槽应有 agent")
                    .session()
                    .id
                    .clone()
            };
            set_current_mode(&state, "leader");

            // ① 切 workflow：工作台出现 leader 条目（is_active=false）+ leader 槽清空
            set_mode_impl(
                handle.clone(),
                handle.state::<AppState>(),
                "workflow".into(),
            )
            .await
            .expect("切换必须成功");
            {
                let payload = shelf::list_shelf_sessions_inner(state.inner()).unwrap();
                let item = shelf_item_of(&payload, &leader_session_id)
                    .unwrap_or_else(|| panic!("归档后工作台必须含 leader 条目"));
                assert_eq!(item["is_active"], false, "归档条目不得是 active");
            }
            {
                let guard = state.runtime.lock().unwrap();
                assert!(
                    guard.leader_agent.is_none(),
                    "离开槽（leader）必须被清空（welcome 语义）"
                );
            }

            // ② workflow 模式发送指令建新会话——由 process.rs 空态判据/规则2 完成，
            //    此处验证 workflow 槽空（不残留 leader 归档影响）
            {
                let guard = state.runtime.lock().unwrap();
                assert!(guard.workflow_agent.is_none());
            }

            // ③ 切回 leader：leader 条目仍在工作台（is_active=false），槽仍空
            set_mode_impl(handle.clone(), handle.state::<AppState>(), "leader".into())
                .await
                .expect("切回必须成功");
            {
                let payload = shelf::list_shelf_sessions_inner(state.inner()).unwrap();
                let item = shelf_item_of(&payload, &leader_session_id)
                    .unwrap_or_else(|| panic!("切回后工作台仍须含 leader 条目"));
                assert_eq!(item["is_active"], false, "切回后不得自动复活为 active");
            }
            {
                let guard = state.runtime.lock().unwrap();
                assert!(
                    guard.leader_agent.is_none(),
                    "切回原 mode 槽必须为空（不自动恢复，回 welcome）"
                );
            }

            // ④ 经工作台 switch_session 恢复。槽为 None（切 mode 已清）→ 走既有降级
            //    契约（shelf.rs switch_session_inner_mode：无 agent 槽 → backup 中转）：
            //    目标会话进 session_backup（get_chat_history 经 backup 回退显示历史）+
            //    放回展示台（rail 不丢条目）；下次发消息 run_runtime_with_config 从
            //    session_backup_json 恢复完整上下文继续对话。
            shelf::switch_session_inner_mode(
                state.inner(),
                leader_session_id.clone(),
                Some("leader".into()),
            )
            .expect("工作台点击恢复必须成功");
            {
                let sb = state.session.lock().unwrap();
                let backup_json = sb
                    .session_backup
                    .as_ref()
                    .expect("降级切换必须写 session_backup 中转");
                let restored: Session =
                    serde_json::from_str(backup_json).expect("backup 应为合法 Session");
                assert_eq!(restored.id, leader_session_id, "中转的必须是原会话");
                assert!(!restored.is_empty(), "恢复后会话内容完整（可继续对话）");
            }
            assert!(
                state.shelf.lock().unwrap().contains(&leader_session_id),
                "目标必须放回展示台（rail 不丢条目）"
            );

            cleanup_rows(&[&leader_session_id]);
        });
    }

    /// 验收序列⑤：空会话（无消息）切 mode 不产生工作台条目
    /// （archive_active 对 empty 会话已自动跳过：sess_ref.is_empty() → return）。
    #[test]
    fn mode_switch_with_empty_session_leaves_no_shelf_entry() {
        tokio_test::block_on(async {
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();

            // leader 槽有**空**会话（无消息）。id 从槽内读取（同 helper 缺陷修正：
            // RuntimeBuilder::build 自建 Session，传入 sess 的 id 与槽内不一致）
            {
                let mut guard = state.runtime.lock().unwrap();
                guard.leader_agent = Some(leader_runtime_with(Session::new()));
            }
            let empty_id = {
                let guard = state.runtime.lock().unwrap();
                guard
                    .leader_agent
                    .as_ref()
                    .expect("前置：leader 槽应有 agent")
                    .session()
                    .id
                    .clone()
            };
            set_current_mode(&state, "leader");

            set_mode_impl(
                handle.clone(),
                handle.state::<AppState>(),
                "workflow".into(),
            )
            .await
            .expect("切换必须成功");

            let payload = shelf::list_shelf_sessions_inner(state.inner()).unwrap();
            assert!(
                shelf_item_of(&payload, &empty_id).is_none(),
                "空会话归档必须被跳过：不产生工作台条目"
            );

            // 空会话没有写库，仍做防御性清理（幂等）
            cleanup_rows(&[&empty_id]);
        });
    }

    /// 同 mode 重复调用幂等：前端启动同步 / 重复点击目标 mode 不触发转场，
    /// 当前会话不得被误归档（槽内容保持原样）。
    #[test]
    fn set_mode_same_mode_is_noop_transition() {
        tokio_test::block_on(async {
            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            handle.manage(AppState::default());
            let state = handle.state::<AppState>();

            {
                let mut guard = state.runtime.lock().unwrap();
                guard.leader_agent =
                    Some(leader_runtime_with(session_with_user("同 mode 幂等守卫")));
            }
            let sess_id = {
                let guard = state.runtime.lock().unwrap();
                guard
                    .leader_agent
                    .as_ref()
                    .expect("前置：leader 槽应有 agent")
                    .session()
                    .id
                    .clone()
            };
            set_current_mode(&state, "leader");

            set_mode_impl(handle.clone(), handle.state::<AppState>(), "leader".into())
                .await
                .expect("同 mode 调用必须成功");

            {
                let guard = state.runtime.lock().unwrap();
                let current = guard
                    .leader_agent
                    .as_ref()
                    .map(|rt| rt.session().id.clone())
                    .expect("同 mode 调用不得清空当前槽");
                assert_eq!(current, sess_id, "同 mode 调用不得动当前会话");
            }
        });
    }
}
