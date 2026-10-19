use super::*;
use crate::workflow::events::EventBus;
use crate::workflow::inputs::resolve_declared_inputs;
use crate::workflow::store::WorkflowStore;
use crate::workflow::types::Workflow;
use crate::workflow::types::{
    Action, ForEachDef, InputKind, InputSpec, LoopDef, RunStatus, Step, StepRunStatus, VarRef,
};
use std::result::Result as StdResult;

fn make_tool_step(name: &str, tool: &str, params: serde_json::Value) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Tool {
            tool: tool.to_string(),
            with: params,
        },
        ..Default::default()
    }
}

async fn ok_tool_exec(_tool: String, _params: serde_json::Value) -> StdResult<String, String> {
    Ok("ok".to_string())
}

async fn fail_tool_exec(_tool: String, _params: serde_json::Value) -> StdResult<String, String> {
    Err("simulated tool failure".to_string())
}

#[test]
fn bracket_fields_keep_types_and_work_in_all_consuming_contexts() {
    let vars = HashMap::from([(
        "wn".into(),
        serde_json::json!({"window_id":123,"title":"微信","a.b":[null,"003"],"x}}y":true}),
    )]);
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{wn[\"window_id\"]}}"), &vars),
        serde_json::json!(123)
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{wn[\"a.b\"][1]}}"), &vars),
        serde_json::json!("003")
    );
    assert_eq!(
        variables::resolve_vars_str("窗口：{{wn[\"title\"]}}", &vars),
        "窗口：微信"
    );
    assert_eq!(
        variables::resolve_vars_str("{{wn[\"x}}y\"]}}", &vars),
        "true"
    );
    assert_eq!(
        variables::resolve_var_by_path("wn[\"a.b\"][0]", &vars),
        Some(&serde_json::Value::Null)
    );
    assert!(variables::eval_condition(
        &Condition::Equals {
            equals: vec![
                VarRef::Var {
                    var: "wn[\"window_id\"]".into()
                },
                VarRef::Lit("123".into())
            ]
        },
        &vars
    ));
}

#[tokio::test]
async fn desktop_unknown_postcondition_is_not_automatically_replayed() {
    for tool in ["desktop_semantic_action", "desktop_input"] {
        let attempts = std::sync::atomic::AtomicUsize::new(0);
        let executor = Executor::new();
        let step = make_tool_step("send once", tool, serde_json::json!({}));
        let tool_exec = |_: String, _: serde_json::Value| {
            attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { Err("desktop_needs_observation: dispatch outcome unknown".to_string()) }
        };
        let result = executor
            .execute_tool_step(
                &step,
                tool,
                &serde_json::json!({}),
                &tool_exec,
                &mut HashMap::new(),
                "test-no-replay",
                &EventBus::new(),
                None,
                None,
            )
            .await;
        assert!(result.is_err());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn desktop_delivery_state_controls_retries_for_every_input_tool() {
    use crate::desktop_automation::{DesktopActionError, DispatchState};
    use std::sync::atomic::{AtomicUsize, Ordering};

    for tool in [
        "desktop_semantic_action",
        "desktop_input",
        "desktop_mouse",
        "desktop_mouse_drag",
    ] {
        for state in [
            DispatchState::NotSent,
            DispatchState::Sent,
            DispatchState::Partial,
            DispatchState::Unknown,
        ] {
            let attempts = AtomicUsize::new(0);
            let executor = Executor::new();
            let mut step = make_tool_step("delivery retry", tool, serde_json::json!({}));
            step.on_error = OnError::Retry {
                max: 2,
                backoff_ms: 1,
                backoff_multiplier: 2.0,
            };
            let tool_exec = |_: String, _: serde_json::Value| {
                attempts.fetch_add(1, Ordering::SeqCst);
                async move { Err(DesktopActionError::encode(state, "native delivery test")) }
            };
            let result = executor
                .execute_tool_step(
                    &step,
                    tool,
                    &serde_json::json!({}),
                    &tool_exec,
                    &mut HashMap::new(),
                    "test-delivery-retry",
                    &EventBus::new(),
                    None,
                    None,
                )
                .await;
            assert!(result.is_err());
            assert_eq!(
                attempts.load(Ordering::SeqCst),
                if state == DispatchState::NotSent {
                    3
                } else {
                    1
                },
                "{tool}: {state:?}"
            );
        }
    }
}

#[tokio::test]
async fn test_executor_linear_single_step() {
    let tmp = std::env::temp_dir().join("nuphus_test_exec");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("linear_test");
    wf.steps = vec![make_tool_step(
        "test_step",
        "mock_echo",
        serde_json::json!({"command": "echo success_output"}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_ok(), "Execution failed: {:?}", result.err());
}

#[tokio::test]
async fn test_compiler_rejects_empty_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_compiler");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let wf = Workflow::new("empty_wf");
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    // V2: 空工作流通过校验，执行成功（空步骤列表直接完成）
    assert!(
        result.is_ok(),
        "Empty workflow should succeed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_executor_with_real_system_shell() {
    let tmp = std::env::temp_dir().join("nuphus_test_real");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("real_shell_test");
    wf.steps = vec![make_tool_step(
        "echo_step",
        "system_shell",
        serde_json::json!({"command": "echo NUphus_TEST_REAL_SHELL_SUCCESS"}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tools = std::sync::Arc::new(crate::tools::ToolRegistry::builtin());
    let tool_exec = {
        let tools = tools.clone();
        move |tool: String, params: serde_json::Value| {
            let tools = tools.clone();
            async move {
                let result = tools
                    .execute(&tool, &params)
                    .await
                    .map_err(|e| e.to_string())?;
                if result.success {
                    Ok(result.output.unwrap_or_default())
                } else {
                    Err(result.error.unwrap_or_else(|| "unknown error".to_string()))
                }
            }
        }
    };

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "Real system_shell execution failed: {:?}",
        result.err()
    );

    // Verify last_run recorded with valid UUID
    let wf_after = store.get(&wf_id).await.unwrap();
    assert!(wf_after.last_run().is_some(), "last_run should be recorded");
    let last_run = wf_after.last_run().unwrap();
    assert!(!last_run.run_id.is_empty(), "Run ID should not be empty");
    assert_eq!(
        last_run.status,
        crate::workflow::types::RunStatus::Success,
        "Run should succeed, status: {:?}",
        last_run.status
    );
}

// ── wf_call 模块化测试 ──

fn make_call_step(name: &str, workflow_id: &str) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Call {
            call: workflow_id.to_string(),
            with: serde_json::json!({"workflow_id": workflow_id}),
        },
        ..Default::default()
    }
}

fn make_seq_step(name: &str, steps: Vec<Step>) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Seq { seq: steps },
        ..Default::default()
    }
}

#[tokio::test]
async fn test_wf_call_single_level() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall1");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 子工作流: 一个 tool 步骤
    let mut sub = Workflow::new("sub_tool");
    let sub_id = sub.id.clone();
    sub.steps = vec![make_tool_step(
        "sub_echo",
        "mock_echo",
        serde_json::json!({"cmd": "hello"}),
    )];
    store.save(&sub).await.unwrap();

    // 父工作流: 调用子工作流
    let mut parent = Workflow::new("parent");
    parent.steps = vec![make_call_step("call_sub", &sub_id)];
    let parent_id = parent.id.clone();
    store.save(&parent).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &parent_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "wf_call single level failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_with_seq_nesting() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_seq");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // L2: 最深子工作流
    let mut l2 = Workflow::new("level2");
    let l2_id = l2.id.clone();
    l2.steps = vec![
        make_tool_step("l2_step1", "mock_echo", serde_json::json!({"msg": "a"})),
        make_tool_step("l2_step2", "mock_echo", serde_json::json!({"msg": "b"})),
    ];
    store.save(&l2).await.unwrap();

    // L1: 使用 Seq 包装对 L2 的调用
    let mut l1 = Workflow::new("level1");
    let l1_id = l1.id.clone();
    l1.steps = vec![make_seq_step(
        "wrapper",
        vec![
            make_tool_step("l1_step", "mock_echo", serde_json::json!({"msg": "x"})),
            make_call_step("invoke_l2", &l2_id),
        ],
    )];
    store.save(&l1).await.unwrap();

    // 顶层: L0 seq 中调用 L1
    let mut l0 = Workflow::new("level0");
    l0.steps = vec![make_seq_step(
        "top_seq",
        vec![make_call_step("invoke_l1", &l1_id)],
    )];
    let l0_id = l0.id.clone();
    store.save(&l0).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &l0_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "Nested wf_call with seq failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_deep_nesting() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_deep");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // L2: 最深子工作流
    let mut l2 = Workflow::new("level2");
    let l2_id = l2.id.clone();
    l2.steps = vec![make_tool_step(
        "deepest",
        "mock_echo",
        serde_json::json!({"msg": "deep"}),
    )];
    store.save(&l2).await.unwrap();

    // L1: 调用 L2
    let mut l1 = Workflow::new("level1");
    let l1_id = l1.id.clone();
    l1.steps = vec![make_call_step("call_l2", &l2_id)];
    store.save(&l1).await.unwrap();

    // 顶层: L0 调用 L1
    let mut l0 = Workflow::new("level0");
    l0.steps = vec![make_call_step("call_l1", &l1_id)];
    let l0_id = l0.id.clone();
    store.save(&l0).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &l0_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "Nested wf_call L0→L1→L2 failed: {:?}",
        result.err()
    );
}

#[tokio::test]
async fn test_wf_call_error_propagation() {
    let tmp = std::env::temp_dir().join("nuphus_test_wfcall_err");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 子工作流: 会失败的步骤
    let mut sub = Workflow::new("failing_sub");
    let sub_id = sub.id.clone();
    sub.steps = vec![make_tool_step(
        "will_fail",
        "bad_tool",
        serde_json::json!({}),
    )];
    store.save(&sub).await.unwrap();

    // 父工作流
    let mut parent = Workflow::new("parent");
    parent.steps = vec![make_call_step("call_failing", &sub_id)];
    let parent_id = parent.id.clone();
    store.save(&parent).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &parent_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_err(),
        "Error in sub-workflow should propagate to parent"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("failing_sub") || err_msg.contains("simulated tool failure"),
        "Error should mention sub-workflow or failure cause, got: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_wf_call_resolves_declared_inputs_and_defaults() {
    let tmp = std::env::temp_dir().join(format!(
        "nuphus_test_wfcall_inputs_{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut sub = Workflow::new("typed_sub");
    let sub_id = sub.id.clone();
    sub.inputs = vec![
        InputSpec {
            name: "count".into(),
            kind: InputKind::Number,
            required: true,
            default: None,
            description: None,
            sensitive: false,
            options: None,
        },
        InputSpec {
            name: "label".into(),
            kind: InputKind::String,
            required: false,
            default: Some(serde_json::json!("default-label")),
            description: None,
            sensitive: false,
            options: None,
        },
    ];
    sub.steps = vec![make_tool_step(
        "observe",
        "mock_echo",
        serde_json::json!({
            "namespaced": "{{inputs.count}}",
            "top_level": "{{count}}",
            "defaulted": "{{inputs.label}}",
            "parent_clone": "{{source}}"
        }),
    )];
    store.save(&sub).await.unwrap();

    let mut parent = Workflow::new("parent");
    parent.steps = vec![Step {
        id: "call".into(),
        name: "call typed sub".into(),
        action: Action::Call {
            call: sub_id,
            with: serde_json::json!({"inputs": {"count": "{{source}}"}}),
        },
        ..Default::default()
    }];
    let parent_id = parent.id.clone();
    store.save(&parent).await.unwrap();

    let bad_result = executor
        .execute_v2(
            &parent_id,
            &store,
            &events,
            |_tool: String, _params: serde_json::Value| async { Ok("ok".to_string()) },
            None,
            None,
            None,
            Some(HashMap::from([(
                "source".into(),
                serde_json::json!("seven"),
            )])),
            false,
        )
        .await;
    assert!(bad_result
        .unwrap_err()
        .to_string()
        .contains("值类型必须是 number"));

    let observed = Arc::new(std::sync::Mutex::new(None));
    let observed_for_exec = observed.clone();
    let tool_exec = move |_tool: String, params: serde_json::Value| {
        let observed = observed_for_exec.clone();
        async move {
            *observed.lock().unwrap() = Some(params);
            Ok("ok".to_string())
        }
    };
    let result = executor
        .execute_v2(
            &parent_id,
            &store,
            &events,
            tool_exec,
            None,
            None,
            None,
            Some(HashMap::from([("source".into(), serde_json::json!(7))])),
            true,
        )
        .await;

    let params = observed.lock().unwrap().clone().expect("tool called");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(params["namespaced"], serde_json::json!(7));
    assert_eq!(params["top_level"], serde_json::json!(7));
    assert_eq!(params["defaulted"], serde_json::json!("default-label"));
    assert_eq!(params["parent_clone"], serde_json::json!(7));
}

// ── P0 修复验证：StepRunRecord 管道 / 失败记录 push / for_each 点号路径 ──

#[tokio::test]
async fn test_run_record_steps_populated() {
    let tmp = std::env::temp_dir().join("nuphus_test_runrec");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("steps_recording");
    let s1 = make_tool_step("s1", "mock_echo", serde_json::json!({"a": 1}));
    let s2 = make_tool_step("s2", "mock_echo", serde_json::json!({"b": 2}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(result.is_ok(), "Execution failed: {:?}", result.err());

    let wf_after = store.get(&wf_id).await.unwrap();
    let last_run = wf_after.last_run().expect("last_run should exist");
    assert_eq!(
        last_run.status,
        RunStatus::Success,
        "status should be Success"
    );
    // 两个步骤都被记录（含 step_id / status）
    let ids: Vec<&str> = last_run.steps.iter().map(|s| s.step_id.as_str()).collect();
    assert!(
        ids.contains(&s1_id.as_str()),
        "s1 should be recorded, got {:?}",
        ids
    );
    assert!(
        ids.contains(&s2_id.as_str()),
        "s2 should be recorded, got {:?}",
        ids
    );
    assert!(
        last_run
            .steps
            .iter()
            .all(|s| s.status == StepRunStatus::Success),
        "all recorded steps should be Success, got {:?}",
        last_run.steps
    );
}

#[tokio::test]
async fn test_for_each_nested_path() {
    let tmp = std::env::temp_dir().join("nuphus_test_for_each_path");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let body = make_tool_step(
        "loop_body",
        "mock_echo",
        serde_json::json!({"v": "{{item}}"}),
    );
    let body_id = body.id.clone();
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "foreach_panels".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: Some(ForEachDef {
                    items: VarRef::Var {
                        var: "panels.list".into(),
                    },
                    item_var: "item".into(),
                }),
                repeat: None,
                until: None,
                max: 100,
                steps: vec![body],
            },
        },
        ..Default::default()
    };
    let loop_id = loop_step.id.clone();

    let mut wf = Workflow::new("for_each_nested");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let inputs: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::from([(
            "panels".to_string(),
            serde_json::json!({"list": [1, 2, 3]}),
        )]);

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            None,
            None,
            None,
            Some(inputs),
            false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "for_each with nested path failed: {:?}",
        result.err()
    );

    let wf_after = store.get(&wf_id).await.unwrap();
    let last_run = wf_after.last_run().expect("last_run should exist");
    assert_eq!(
        last_run.status,
        RunStatus::Success,
        "status should be Success"
    );
    // {{panels.list}} 点号路径应取到嵌套数组 → 循环体执行 3 次（3 条 body 记录）
    let body_count = last_run
        .steps
        .iter()
        .filter(|s| s.step_id == body_id)
        .count();
    assert_eq!(
        body_count, 3,
        "loop body should run 3 times, got {} (steps: {:?})",
        body_count, last_run.steps
    );
    // 容器 loop 步骤本身也记录
    assert!(
        last_run.steps.iter().any(|s| s.step_id == loop_id),
        "loop container step should be recorded"
    );
}

#[tokio::test]
async fn test_failed_run_creates_error_record_without_corrupting_previous() {
    let tmp = std::env::temp_dir().join("nuphus_test_fail_push");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // Run 1: 成功
    let mut wf = Workflow::new("ok_then_fail");
    wf.steps = vec![make_tool_step("ok1", "mock_echo", serde_json::json!({}))];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let ok_tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            ok_tool_exec,
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_ok(), "first run should succeed: {:?}", r1.err());
    let wf_after_1 = store.get(&wf_id).await.unwrap();
    assert_eq!(wf_after_1.last_run().unwrap().status, RunStatus::Success);

    // Run 2: 失败
    let mut wf2 = store.get(&wf_id).await.unwrap();
    wf2.steps = vec![make_tool_step("failing", "bad_tool", serde_json::json!({}))];
    store.save(&wf2).await.unwrap();

    let fail_tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            fail_tool_exec,
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail");

    let wf_after_2 = store.get(&wf_id).await.unwrap();
    let history = &wf_after_2.run_history;
    assert!(
        history.len() >= 2,
        "history should have 2 records, got {}",
        history.len()
    );
    // 最新记录是 Error（新 push），上一条成功记录不得被污染
    assert!(
        matches!(history[0].status, RunStatus::Error(_)),
        "latest record should be Error, got {:?}",
        history[0].status
    );
    assert_eq!(
        history[1].status,
        RunStatus::Success,
        "previous success record must be untouched, got {:?}",
        history[1].status
    );
    // 失败步骤被记录为 Error
    assert!(
        history[0]
            .steps
            .iter()
            .any(|s| s.status != StepRunStatus::Success),
        "failed run should contain a non-Success step record: {:?}",
        history[0].steps
    );
}

/// B1 回归：取消标志泄漏 —— future 被 drop（plugin 300s 超时是受支持路径）时
/// 正常返回路径的清理不跑，残留 true 的 cancel flag 会让下次同 id 运行每步
/// 立即「已被取消」。RunStateGuard（Drop）必须在 guard 释放路径上移除条目，
/// 使二次运行拿到全新 false flag 正常执行。
#[tokio::test]
async fn test_cancel_flag_leak_does_not_poison_next_run() {
    let tmp = std::env::temp_dir().join("nuphus_test_cancel_leak");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("cancel_leak");
    wf.steps = vec![make_tool_step(
        "only",
        "counted_tool",
        serde_json::json!({}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move |_: String, _: serde_json::Value| {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("ok".to_string())
            }
        }
    };

    // ── 模拟泄漏：直接向 cancel_flags 插入 workflow_id → true（等价于「上次运行
    //    被取消后 future 被 drop，清理未跑」的终态）──
    {
        let mut flags = executor.cancel_flags.write().await;
        flags.insert(
            wf_id.clone(),
            Arc::new(std::sync::atomic::AtomicBool::new(true)),
        );
    }

    // ── 二次运行：不得被残留 true 的 flag 误取消 ──
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None,
            true, // force_fresh：全新执行，排除断点续连干扰
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "残留 true 的 cancel flag 不得毒化下一次运行: {:?}",
        result.err()
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "步骤应正常执行一次（每步 check_cancel 未误触发）"
    );

    // guard 释放路径：运行结束后条目已被移除，不留泄漏
    assert!(
        !executor.cancel_flags.read().await.contains_key(&wf_id),
        "运行结束后 cancel_flags 条目应被清理"
    );
}

#[tokio::test]
async fn versioned_run_keeps_host_cancellation_and_cleans_per_run_state() {
    use crate::workflow::run_context::{RunContext, CURRENT};
    use std::sync::atomic::{AtomicBool, Ordering};

    let root = tempfile::tempdir().unwrap();
    let mut workflow = Workflow::new("versioned cancellation");
    workflow.steps = vec![
        make_tool_step("first", "test_tool", serde_json::json!({})),
        make_tool_step("second", "test_tool", serde_json::json!({})),
    ];
    let id = workflow.id.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let context = Arc::new(RunContext {
        run_id: uuid::Uuid::new_v4().to_string(),
        project_dir: root.path().into(),
        cancelled: cancelled.clone(),
        store: WorkflowStore::frozen(root.path().into(), vec![workflow]),
        event_sink: None,
    });
    let executor = Executor::new();
    let events = EventBus::new();
    // A stale entry must be replaced with this host's cancellation flag.
    executor
        .cancel_flags
        .write()
        .await
        .insert(id.clone(), Arc::new(AtomicBool::new(true)));
    let calls = std::sync::atomic::AtomicUsize::new(0);
    let result = CURRENT
        .scope(
            context.clone(),
            executor.execute_v2(
                &id,
                &context.store,
                &events,
                |_, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    cancelled.store(true, Ordering::SeqCst);
                    async { Ok("host cancelled after first tool".to_string()) }
                },
                None,
                None,
                None,
                None,
                true,
            ),
        )
        .await;
    assert!(
        result.is_err(),
        "host cancellation must stop the second step"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(cancelled.load(Ordering::SeqCst));
    assert!(!executor.cancel_flags.read().await.contains_key(&id));
}

#[tokio::test]
async fn test_resume_skips_completed_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_resume");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 工作流：[s1 成功, s2 失败]；s1 带计数器，重试时不应被重新执行
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("resume_skip");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "bad_tool", serde_json::json!({}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("simulated tool failure".to_string())
                    }
                }
            }
        }
    };

    // 首次执行：s1 成功、s2 失败
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_err(), "first run should fail at s2");

    let wf_after_1 = store.get(&wf_id).await.unwrap();
    assert!(
        matches!(wf_after_1.last_run().unwrap().status, RunStatus::Error(_)),
        "first failed run must be recorded"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 executed once in first run"
    );

    // 重试：completed_ids 应从上次 Error 记录提取 s1（Success）→ s1 跳过，只重跑 s2
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail at s2 again");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 must NOT be re-executed on resume (completed_ids skip)"
    );

    // 第二次 run 记录：含继承的 s1(Success) + 本次 s2(Error)
    let wf_after_2 = store.get(&wf_id).await.unwrap();
    let last = wf_after_2.last_run().unwrap();
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s1_id && s.status == StepRunStatus::Success),
        "resumed run should inherit s1 Success record: {:?}",
        last.steps
    );
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s2_id && matches!(s.status, StepRunStatus::Error(_))),
        "resumed run should record s2 Error: {:?}",
        last.steps
    );
}
#[tokio::test]
async fn test_paused_resume_skips_completed_steps() {
    let tmp = std::env::temp_dir().join("nuphus_test_paused_resume");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("paused_resume");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "counted_tool", serde_json::json!({}));
    let s1_id = s1.id.clone();
    let s2_id = s2.id.clone();
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    // 手动构造 Paused 状态的 last_run：s1 已完成（Success），模拟"暂停后进程重启"断点
    let mut wf_paused = store.get(&wf_id).await.unwrap();
    wf_paused.push_run(RunRecord {
        run_id: "paused-run-1".into(),
        started_at: chrono::Utc::now(),
        finished_at: None,
        status: RunStatus::Paused,
        steps: vec![StepRunRecord {
            step_id: s1_id.clone(),
            started_at: chrono::Utc::now(),
            finished_at: Some(chrono::Utc::now()),
            status: StepRunStatus::Success,
            output_summary: None,
        }],
        error: None,
        variables_snapshot: std::collections::HashMap::new(),
    });
    store.save(&wf_paused).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("unexpected tool".to_string())
                    }
                }
            }
        }
    };

    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;

    assert!(
        result.is_ok(),
        "paused resume should succeed: {:?}",
        result.err()
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 (Paused 已完成) 不得重跑，只执行 s2"
    );

    let wf_after = store.get(&wf_id).await.unwrap();
    let last = wf_after.last_run().unwrap();
    assert_eq!(last.status, RunStatus::Success);
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s1_id && s.status == StepRunStatus::Success),
        "resumed run should inherit s1 Success: {:?}",
        last.steps
    );
    assert!(
        last.steps
            .iter()
            .any(|s| s.step_id == s2_id && s.status == StepRunStatus::Success),
        "resumed run should record s2 Success: {:?}",
        last.steps
    );
}

/// 生产路径复现：失败 → store.load_all() 热刷新（清缓存重读磁盘）→ 再执行。
/// 模拟 workflow_run 两次调用之间夹了一次 load_all 的情况。
/// 若 run_history(Error) 未持久化或 load_all 丢失 run_history，则 s1 会重跑 → 断言失败。
#[tokio::test]
async fn test_resume_survives_hot_reload() {
    let tmp = std::env::temp_dir().join("nuphus_test_resume_reload");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut wf = Workflow::new("resume_reload");
    let s1 = make_tool_step("s1", "counted_tool", serde_json::json!({}));
    let s2 = make_tool_step("s2", "bad_tool", serde_json::json!({}));
    wf.steps = vec![s1, s2];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = {
        let calls = calls.clone();
        move || {
            let calls = calls.clone();
            move |tool: String, _params: serde_json::Value| {
                let calls = calls.clone();
                async move {
                    if tool == "counted_tool" {
                        calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("ok".to_string())
                    } else {
                        Err("simulated tool failure".to_string())
                    }
                }
            }
        }
    };

    // 第一次执行：s1 成功、s2 失败
    let r1 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r1.is_err(), "first run should fail at s2");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    // 生产路径：workflow_run 两次调用之间 store.load_all() 热刷新（清缓存重读磁盘）
    store.load_all().await.unwrap();

    // 第二次执行：resume 应跳过 s1，只重跑 s2
    let r2 = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec(),
            None,
            None,
            None,
            None,
            false,
        )
        .await;
    assert!(r2.is_err(), "second run should fail at s2 again");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "s1 must NOT be re-executed on resume after load_all hot reload"
    );

    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

// ── Chat 步骤 per-step 模型路由 ──

/// 最小 mock ApiClient：固定返回文本（无工具调用 → chat 一轮结束）
struct MockChatClient;

#[async_trait::async_trait]
impl crate::api::ApiClient for MockChatClient {
    async fn stream(
        &self,
        _request: crate::api::MessageRequest,
    ) -> crate::Result<Vec<crate::api::AssistantEvent>> {
        Ok(vec![
            crate::api::AssistantEvent::TextDelta("mock reply".to_string()),
            crate::api::AssistantEvent::MessageStop,
        ])
    }

    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn provider_kind(&self) -> crate::api::ProviderKind {
        crate::api::ProviderKind::Custom
    }

    fn provider_name(&self) -> &str {
        ""
    }
}

fn make_chat_step(name: &str, message: &str, model: Option<&str>) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        action: Action::Chat {
            chat: message.to_string(),
            with: ChatOpts {
                model: model.map(|m| m.to_string()),
                ..Default::default()
            },
        },
        ..Default::default()
    }
}

/// registry 只含 routed-model（指向不可达地址）
fn make_test_factory() -> crate::llm::ClientFactory {
    let registry = crate::config::ModelRegistry::from_single(
        "routed-model".to_string(),
        "custom".to_string(),
        "test-key".to_string(),
        "http://127.0.0.1:1".to_string(),
        None,
    );
    crate::llm::ClientFactory::new(registry)
}

/// with.model 不在 registry → 回退主 client（裸模型名语义，向后兼容）
#[tokio::test]
async fn test_chat_model_fallback_bare_name() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_fallback");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_fallback_test");
    wf.steps = vec![make_chat_step(
        "chat_step",
        "hello",
        Some("bare-name-model"),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "unknown model id must fallback to main client: {:?}",
        result.err()
    );
}

/// with.model 命中 registry → 走 factory 装配的专属 client（不可达地址 → 失败即证明已路由，
/// 若走 mock 主 client 则会成功）
#[tokio::test]
async fn test_chat_model_routed_to_registry_client() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_routed");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_routed_test");
    wf.steps = vec![make_chat_step("chat_step", "hello", Some("routed-model"))];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    let err = result.expect_err("routed client (unreachable) must fail, proving routing happened");
    assert!(
        err.to_string().contains("LLM call failed"),
        "error should come from routed client's stream call: {}",
        err
    );
}

/// 无 with.model → 现状不变（主 client + 默认模型名）
#[tokio::test]
async fn test_chat_no_model_uses_main_client() {
    let tmp = std::env::temp_dir().join("nuphus_test_chat_default");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut executor = Executor::new();
    executor.set_client_factory(make_test_factory());

    let mut wf = Workflow::new("chat_default_test");
    wf.steps = vec![make_chat_step("chat_step", "hello", None)];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let llm = MockChatClient;
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            Some(&llm),
            None,
            None,
            None,
            false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(
        result.is_ok(),
        "chat without with.model must use main client: {:?}",
        result.err()
    );
}

/// P2 修复验证：失败步骤必须同时发 Error（message 横幅）+ StepRunCompleted{Error}。
/// 旧实现只发 Error（无 step_id），前端无法定位失败步骤，run_completed 时被误收敛为绿色
/// completed。前端据此识别 {"Error": ...} 形状标红叉。
#[tokio::test]
async fn test_failed_step_emits_error_and_step_run_completed_error() {
    let tmp = std::env::temp_dir().join("nuphus_test_fail_emit");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let mut wf = Workflow::new("fail_emit_test");
    wf.steps = vec![make_tool_step(
        "fail_step",
        "mock_fail",
        serde_json::json!({}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);

    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_err(), "模拟失败步骤应使执行失败");

    // 断言事件流：Error + StepRunCompleted{Error} 均已发出（执行期间事件已入队，逐一 drain）
    let mut saw_error = false;
    let mut saw_step_completed_error = false;
    loop {
        match rx.try_recv() {
            Ok(WorkflowEvent::Error { .. }) => saw_error = true,
            Ok(WorkflowEvent::StepRunCompleted {
                status: StepRunStatus::Error(_),
                ..
            }) => saw_step_completed_error = true,
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(_) => break,
        }
    }
    assert!(saw_error, "失败步骤应发 Error 事件（message 横幅数据源）");
    assert!(
        saw_step_completed_error,
        "失败步骤应发 StepRunCompleted{{Error}}（前端据此标红叉，而非误收敛为绿色 completed）"
    );
}

/// I-11 回归：重试路径必须发射 StepRunRetry（此前枚举字段齐全但全仓无 emit 点，
/// 前端 step_run_retry 徽章是死代码）。attempt 语义：首次重试 = 1（前端显示 retrying N）。
#[tokio::test]
async fn test_retry_emits_step_run_retry_events() {
    let tmp = std::env::temp_dir().join("nuphus_test_retry_emit");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let mut step = make_tool_step("flaky_step", "flaky_tool", serde_json::json!({}));
    step.on_error = OnError::Retry {
        max: 2,
        backoff_ms: 1,
        backoff_multiplier: 2.0,
    };
    let mut wf = Workflow::new("retry_emit_test");
    wf.steps = vec![step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    // 始终失败：max=2 → 3 次尝试、2 次重试发射
    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    let _ = tokio::fs::remove_dir_all(&tmp).await;
    assert!(result.is_err());

    let mut retry_attempts: Vec<u32> = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(WorkflowEvent::StepRunRetry { attempt, .. }) => retry_attempts.push(attempt),
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(_) => break,
        }
    }
    assert_eq!(
        retry_attempts,
        vec![1, 2],
        "两次重试应各发一次 StepRunRetry，attempt 从 1 递增: {retry_attempts:?}"
    );
}

/// wait 步骤的提示语必须做 `{{var}}` 模板替换（与 chat / script / tool / mcp 对齐）。
///
/// 回归背景：实测 HUD 上显示的是字面的 "等待: 已读取到 {{pending_raw}}…" ——
/// 用户既看不出在等什么，也看不到该步骤本该汇报的内容（待学课程清单）。
/// 根因：`Action::Wait` 把 wait 文案原样传给执行器，唯独它没走 resolve_vars_str，
/// 而同一条提示语里的变量在上游步骤已经 `capture` 成功。
#[tokio::test]
async fn wait_prompt_is_variable_substituted() {
    async fn pending_tool_exec(
        _tool: String,
        _params: serde_json::Value,
    ) -> StdResult<String, String> {
        Ok("5 门课未完成".to_string())
    }

    let tmp = std::env::temp_dir().join(format!("nuphus_test_wait_vars_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    // 上游：工具步骤把输出 capture 成变量 pending_raw（非 JSON → 原样存字符串）
    let mut capture_step = make_tool_step("read_pending", "mock_echo", serde_json::json!({}));
    capture_step.capture = Some("pending_raw".to_string());

    // 下游：wait 步骤的提示语引用该变量
    let wait_step = Step {
        id: "report_plan".to_string(),
        name: "汇报学习计划".to_string(),
        action: Action::Wait {
            wait: "已读取到 {{pending_raw}}。点击继续开始。".to_string(),
            auto: vec![],
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("wait_vars_test");
    wf.steps = vec![capture_step, wait_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| pending_tool_exec(tool, params);
    // 按事件同步，不以 900ms 硬截断磁盘/线程池调度较慢的 CI 执行。
    // 收到实际暂停事件后正常续跑，同时确认工作流最终成功；超时只作死锁保护。
    let observe_and_resume = async {
        loop {
            match rx.recv().await.expect("工作流事件通道应保持可用") {
                WorkflowEvent::StepRunPaused {
                    step_id, reason, ..
                } if step_id == "report_plan" => {
                    executor.resume(&wf_id).await;
                    break reason;
                }
                _ => {}
            }
        }
    };
    let observed = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        tokio::join!(
            executor.execute_v2(&wf_id, &store, &events, tool_exec, None, None, None, None, false,),
            observe_and_resume
        )
    })
    .await;
    tokio::fs::remove_dir_all(&tmp).await.unwrap();
    let (result, reason) = observed.expect("wait 步骤应发出 StepRunPaused 并在续跑后完成");

    assert!(result.is_ok(), "续跑后应成功完成: {result:?}");
    assert!(
        reason.contains("5 门课未完成"),
        "wait 提示语里的 {{{{pending_raw}}}} 应被替换为变量值，实际: {reason:?}"
    );
    assert!(
        !reason.contains("{{"),
        "wait 提示语不应残留模板占位符，实际: {reason:?}"
    );
}

// ── 声明式外部输入（workflow.inputs）──

fn input_spec(
    name: &str,
    required: bool,
    default: Option<serde_json::Value>,
    sensitive: bool,
) -> InputSpec {
    InputSpec {
        name: name.to_string(),
        kind: InputKind::String,
        required,
        default,
        description: None,
        sensitive,
        options: None,
    }
}

/// 未提供且非必填但有 default → 注入 default；显式提供 → 覆盖 default；非必填缺省 → 不注入
#[test]
fn resolve_declared_inputs_default_override_and_absent() {
    let specs = vec![
        input_spec("with_default", false, Some(serde_json::json!("d")), false),
        input_spec("optional_absent", false, None, false),
        input_spec("overridden", false, Some(serde_json::json!("d")), false),
    ];
    let mut provided = HashMap::new();
    provided.insert("overridden".to_string(), serde_json::json!("explicit"));

    let resolved = resolve_declared_inputs(&specs, &provided).expect("非必填缺省不应报错");
    assert_eq!(resolved.get("with_default"), Some(&serde_json::json!("d")));
    assert_eq!(
        resolved.get("overridden"),
        Some(&serde_json::json!("explicit"))
    );
    assert!(
        !resolved.contains_key("optional_absent"),
        "非必填且无 default 的输入不应注入该键: {resolved:?}"
    );
}

/// required 且无 default 且未提供 → 执行前报错，原因可读（含输入名）
#[test]
fn resolve_declared_inputs_required_missing_errors() {
    let specs = vec![input_spec("token", true, None, false)];
    let err = resolve_declared_inputs(&specs, &HashMap::new()).expect_err("缺必填输入应报错");
    let msg = err.to_string();
    assert!(msg.contains("缺少必填输入：token"), "错误原因应可读: {msg}");
}

/// required 但有 default → 用 default；显式提供优先于 required 判定
#[test]
fn resolve_declared_inputs_required_with_default() {
    let specs = vec![input_spec(
        "mode",
        true,
        Some(serde_json::json!("fast")),
        false,
    )];
    let resolved = resolve_declared_inputs(&specs, &HashMap::new()).expect("有 default 不应报错");
    assert_eq!(resolved.get("mode"), Some(&serde_json::json!("fast")));

    let mut provided = HashMap::new();
    provided.insert("mode".to_string(), serde_json::json!("slow"));
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert_eq!(resolved.get("mode"), Some(&serde_json::json!("slow")));
}

/// 未声明的显式输入不进入声明结果（是否散注入顶层由注入层决定）；空声明 → 空结果（旧工作流语义）
#[test]
fn resolve_declared_inputs_ignores_undeclared_and_empty_specs() {
    let specs: Vec<InputSpec> = Vec::new();
    let mut provided = HashMap::new();
    provided.insert("legacy".to_string(), serde_json::json!("v"));
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert!(resolved.is_empty(), "空声明应返回空结果: {resolved:?}");

    let specs = vec![input_spec("declared", false, None, false)];
    let resolved = resolve_declared_inputs(&specs, &provided).unwrap();
    assert!(
        !resolved.contains_key("legacy"),
        "未声明的运行时输入不应被提升为声明项: {resolved:?}"
    );
}

/// 声明项双写取值：{{inputs.x}} 与 {{x}} 均可取到；支持管道与文本内嵌
#[test]
fn variables_input_namespace_and_top_level() {
    let mut vars: HashMap<String, serde_json::Value> = HashMap::new();
    vars.insert(
        "inputs".to_string(),
        serde_json::json!({"name": "Nuphus", "flag": true}),
    );
    vars.insert("name".to_string(), serde_json::json!("Nuphus"));

    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.name}}"), &vars),
        serde_json::json!("Nuphus")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{name}}"), &vars),
        serde_json::json!("Nuphus")
    );
    // 类型不字符串化（布尔/数字保持原始类型）
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.flag}}"), &vars),
        serde_json::json!(true)
    );
    // 允许 {{ inputs.x }} 带空格；文本内嵌与管道写法均支持
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{ inputs.name }}"), &vars),
        serde_json::json!("Nuphus")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("dir-{{inputs.name}}/x"), &vars),
        serde_json::json!("dir-Nuphus/x")
    );
    assert_eq!(
        Executor::resolve_vars(&serde_json::json!("{{inputs.name | default \"d\"}}"), &vars),
        serde_json::json!("Nuphus")
    );
    // 字符串模板路径（wait 提示语 / chat message / script code 同源）
    assert_eq!(
        super::variables::resolve_vars_str("hi {{inputs.name}}", &vars),
        "hi Nuphus"
    );
}

// ── B9：敏感值落盘脱敏 ──

/// 纯函数契约：顶层键与 inputs 命名空间内的敏感键都被剔除；非敏感键原样保留
#[test]
fn strip_sensitive_snapshot_removes_sensitive_keys_both_positions() {
    use super::variables::strip_sensitive_from_snapshot;

    let sensitive: std::collections::HashSet<String> = ["token".to_string()].into_iter().collect();
    let vars: HashMap<String, serde_json::Value> = HashMap::from([
        ("token".to_string(), serde_json::json!("super-secret")),
        ("plain".to_string(), serde_json::json!("visible")),
        (
            "inputs".to_string(),
            serde_json::json!({"token": "super-secret", "plain": "visible"}),
        ),
    ]);

    let snapshot = strip_sensitive_from_snapshot(&vars, &sensitive);

    assert!(
        !snapshot.contains_key("token"),
        "顶层敏感键应被剔除: {snapshot:?}"
    );
    assert_eq!(snapshot.get("plain"), Some(&serde_json::json!("visible")));
    let ns = snapshot.get("inputs").and_then(|v| v.as_object()).unwrap();
    assert!(
        !ns.contains_key("token"),
        "inputs 命名空间内的敏感键应被剔除: {ns:?}"
    );
    assert_eq!(ns.get("plain"), Some(&serde_json::json!("visible")));

    // 空名单 → 原样快照（含敏感键，因为根本没有敏感声明）
    let no_sensitive: std::collections::HashSet<String> = std::collections::HashSet::new();
    let passthrough = strip_sensitive_from_snapshot(&vars, &no_sensitive);
    assert_eq!(
        passthrough.get("token"),
        Some(&serde_json::json!("super-secret"))
    );
}

/// 端到端：带 sensitive 输入的工作流执行完成后，落盘的 workflow.json
/// 中不得出现敏感明文（顶层快照与 inputs 命名空间两处都要验证）；
/// 非敏感变量照常保留。
#[tokio::test]
async fn sensitive_input_value_never_persisted_to_workflow_json() {
    let tmp = std::env::temp_dir().join("nuphus_test_b9_persist");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let mut wf = Workflow::new("b9_persist");
    wf.inputs = vec![input_spec("token", false, None, true)];
    wf.steps = vec![make_tool_step("only", "mock_echo", serde_json::json!({}))];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let inputs = HashMap::from([("token".to_string(), serde_json::json!("super-secret"))]);
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            None,
            None,
            None,
            Some(inputs),
            true,
        )
        .await;
    assert!(result.is_ok(), "执行应成功: {:?}", result.err());

    // 直接读磁盘上的 workflow.json（绕过内存缓存，验证真实落盘内容）
    let persisted = std::fs::read_to_string(tmp.join(&wf_id).join("workflow.json")).unwrap();
    assert!(
        !persisted.contains("super-secret"),
        "敏感明文不得出现在落盘的 workflow.json 中"
    );

    let wf_after = store.get(&wf_id).await.unwrap();
    let snapshot = &wf_after.last_run().unwrap().variables_snapshot;
    assert!(
        !snapshot.contains_key("token"),
        "顶层快照不得含敏感键: {snapshot:?}"
    );
    if let Some(ns) = snapshot.get("inputs").and_then(|v| v.as_object()) {
        assert!(
            !ns.contains_key("token"),
            "inputs 命名空间快照不得含敏感键: {ns:?}"
        );
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

// ── B6：exit code 解析的字节边界安全 ──

/// B6 回归：错误文本含 'İ'（U+0130，lowercase 后字节长度变化）时，
/// parse_exit_code_from_tool_error 不得 panic，且功能正常（能解析出退出码）。
/// 旧实现：pos 来自 lowercase 串、切片作用于原始 err → 字节错位 panic。
#[test]
fn parse_exit_code_byte_boundary_safety() {
    // 'İ' U+0130 lowercase 成 "i̇"（i + U+0307 组合符，3 字节 vs 原来多字节），
    // 前缀长度变化导致旧实现的 pos 相对原串错位
    let err = "İİİ exit code: 42 rest";
    let code = super::step_tool::parse_exit_code_from_tool_error(err);
    assert_eq!(code, Some(42), "含非 ASCII 前缀时仍应解析出退出码");

    // 前缀在 pattern 之后错位的极端情况：'İ' 在 pattern 前但 lowercase 后错位
    // 落在多字节字符中间——旧实现会 panic，新实现安全返回
    let tricky = format!("{}{}", "İ".repeat(3), " Exit Code: -1");
    assert_eq!(
        super::step_tool::parse_exit_code_from_tool_error(&tricky),
        Some(-1),
        "大小写混合 + 非 ASCII 前缀应解析出负退出码"
    );

    // 纯 ASCII 行为不回归
    assert_eq!(
        super::step_tool::parse_exit_code_from_tool_error("exit code: 3"),
        Some(3)
    );
    assert_eq!(
        super::step_tool::parse_exit_code_from_tool_error("EXIT CODE: 7"),
        Some(7),
        "大写 pattern 应通过 lowercase 匹配命中"
    );
    assert_eq!(
        super::step_tool::parse_exit_code_from_tool_error("no exit code here"),
        None,
        "无 pattern 时应返回 None"
    );
}

/// B6 端到端：含 'İ' 的工具错误文本走完整 tool step（AllowCodes 路径）不 panic，
/// 白名单退出码照常被视为成功。
#[tokio::test]
async fn tool_step_with_turkish_capital_i_error_does_not_panic() {
    let executor = Executor::new();
    let mut step = make_tool_step("tricky_exit", "mock_tool", serde_json::json!({}));
    step.on_error = OnError::AllowCodes { codes: vec![2] };
    let tool_exec = |_tool: String, _params: serde_json::Value| async move {
        Err("İİ exit code: 2".to_string())
    };
    let result = executor
        .execute_tool_step(
            &step,
            "mock_tool",
            &serde_json::json!({}),
            &tool_exec,
            &mut HashMap::new(),
            "test-b6-turkish",
            &EventBus::new(),
            None,
            None,
        )
        .await;
    assert!(
        result.is_ok(),
        "白名单退出码（含 'İ' 错误前缀）应被视为成功且不 panic: {result:?}"
    );
}

// ── B4/B5：RunCompleted 事件与落盘 RunRecord 一致性 ──

/// B4/B5 回归：失败运行的 RunCompleted 事件载荷必须与落盘 RunRecord 同源——
/// 同 status（Error）同 error 文本（redacted 真实原因，非硬编码 "execution failed"），
/// 且 run_record.error 字段被持久化（此前从不赋值，失败原因永远丢失）。
#[tokio::test]
async fn failed_run_runcompleted_matches_persisted_record() {
    let tmp = std::env::temp_dir().join("nuphus_test_b45_consistency");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let mut wf = Workflow::new("b45_consistency");
    wf.steps = vec![make_tool_step(
        "fail_step",
        "mock_fail",
        serde_json::json!({}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| fail_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_err(), "失败步骤应使执行失败");

    // 事件流：取 RunCompleted 载荷
    let mut run_completed_status: Option<RunStatus> = None;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::RunCompleted { status, .. } = ev {
            run_completed_status = Some(status);
        }
    }
    let event_status = run_completed_status.expect("RunCompleted 事件应已发出");

    // 落盘记录：取最新 RunRecord
    let wf_after = store.get(&wf_id).await.unwrap();
    let last = wf_after.last_run().expect("失败记录应已落盘");

    // 一致性断言：同 status 同 error 文本（单一真相源）
    assert_eq!(
        event_status, last.status,
        "RunCompleted 事件 status 必须与落盘 RunRecord.status 一致"
    );
    let persisted_error = last
        .error
        .clone()
        .expect("失败原因必须持久化到 run_record.error");
    match &event_status {
        RunStatus::Error(text) => {
            assert_eq!(
                text, &persisted_error,
                "事件携带的错误文本必须与持久化的 error 字段相同"
            );
            assert!(
                text.contains("simulated tool failure"),
                "错误文本应是真实原因（含工具失败信息），而非硬编码 'execution failed': {text}"
            );
        }
        other => panic!("失败运行的事件 status 应为 Error，实际: {other:?}"),
    }
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B4/B5 回归：成功运行的 RunCompleted 载荷与落盘记录同为 Success，
/// 且 error 字段为 None（不得误写）。
#[tokio::test]
async fn successful_run_runcompleted_matches_persisted_record() {
    let tmp = std::env::temp_dir().join("nuphus_test_b45_success");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let mut wf = Workflow::new("b45_success");
    wf.steps = vec![make_tool_step(
        "ok_step",
        "mock_echo",
        serde_json::json!({}),
    )];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "执行应成功: {:?}", result.err());

    let mut run_completed_status: Option<RunStatus> = None;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::RunCompleted { status, .. } = ev {
            run_completed_status = Some(status);
        }
    }
    let event_status = run_completed_status.expect("RunCompleted 事件应已发出");

    let wf_after = store.get(&wf_id).await.unwrap();
    let last = wf_after.last_run().unwrap();
    assert_eq!(event_status, RunStatus::Success);
    assert_eq!(last.status, RunStatus::Success);
    assert_eq!(event_status, last.status, "成功路径事件与落盘也必须一致");
    assert!(last.error.is_none(), "成功运行不得写入 error 字段");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B4/B5 回归（Cancelled 接通）：用户取消运行后，RunCompleted 事件与落盘记录
/// 都必须是 RunStatus::Cancelled（此前 Cancelled 变体全仓库无产生点，取消被
/// 谎报为 Error）。取消路径与生产一致：cancel flag 置位 → 工具重试间隔处的
/// check_cancel 报「已被取消」→ run 失败 → 定稿读取同一 flag。
/// tests.rs 是 executor 模块子模块，可直接访问私有 cancel_flags。
#[tokio::test]
async fn cancelled_run_finalizes_as_cancelled_in_event_and_record() {
    let tmp = std::env::temp_dir().join("nuphus_test_b45_cancelled");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    // 模拟用户在工具执行期间点「停止」：hud_stop → executor.cancel(id) →
    // flag 置位 + pause_notify 唤醒。工具本身成功返回，但步骤入口处的
    // check_cancel（下一个容器步/子步骤）会发现 flag → run 以 Err 收场。
    // 为保证有「下一步」，工作流配两个工具步骤。
    let mut wf = Workflow::new("b45_cancelled");
    wf.steps = vec![
        make_tool_step("cancel_step_1", "slow_tool", serde_json::json!({})),
        make_tool_step("cancel_step_2", "slow_tool", serde_json::json!({})),
    ];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    // 工具回调通过 &executor 引用在首次执行时置位取消（tests 是 executor
    // 子模块可访问 pub(super) 方法 cancel；&executor 不可变共享借用与
    // execute_v2(&self) 并存合法）。
    let tool_exec = {
        let executor_ref = &executor;
        let wf_id_ref = &wf_id;
        move |_tool: String, _params: serde_json::Value| {
            let executor_ref = executor_ref;
            let wf_id_ref = wf_id_ref;
            async move {
                // 仅第一次调用置位（第二次调用发生在置位后，
                // check_cancel 在步骤入口直接报错不再执行工具）
                executor_ref.cancel(wf_id_ref).await;
                Ok("tool ok but cancelled after".to_string())
            }
        }
    };
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_err(), "步骤 2 入口应因取消失败: {result:?}");

    let mut run_completed_status: Option<RunStatus> = None;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::RunCompleted { status, .. } = ev {
            run_completed_status = Some(status);
        }
    }
    let event_status = run_completed_status.expect("RunCompleted 事件应已发出");

    let wf_after = store.get(&wf_id).await.unwrap();
    let last = wf_after.last_run().unwrap();

    assert_eq!(
        event_status,
        RunStatus::Cancelled,
        "用户取消定稿必须是 Cancelled（事件侧）"
    );
    assert_eq!(
        last.status,
        RunStatus::Cancelled,
        "用户取消定稿必须是 Cancelled（落盘侧）"
    );
    assert_eq!(event_status, last.status, "取消路径事件与落盘也必须一致");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

// ── B13/B14：循环静默成功转显式 ──

/// B13 回归：for_each 引用了不存在的集合变量（变量名打错场景）时，运行仍合法跳过
/// （空集合语义不 breaking），但必须发出可观测警告（StepRunOutput，文本带 [警告]
/// 前缀与变量名）——此前静默返回 Ok 无任何提示。
#[tokio::test]
async fn for_each_missing_variable_emits_warning_but_stays_ok() {
    let tmp = std::env::temp_dir().join("nuphus_test_b13_missing_var");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let body = make_tool_step("body", "mock_echo", serde_json::json!({}));
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "foreach_typo".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: Some(ForEachDef {
                    // 变量名打错：正确名是 panels，这里写成 panles
                    items: VarRef::Var {
                        var: "panles.list".into(),
                    },
                    item_var: "item".into(),
                }),
                repeat: None,
                until: None,
                max: 100,
                steps: vec![body],
            },
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("b13_typo");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(
        result.is_ok(),
        "空集合跳过是合法语义，运行不得失败: {:?}",
        result.err()
    );

    // 可观测性断言：警告事件已发出，文本含变量名与 [警告] 标记
    let mut warning: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::StepRunOutput { text, .. } = ev {
            if text.contains("[警告]") {
                warning = Some(text);
            }
        }
    }
    let warning = warning.expect("变量名打错时必须发出 StepRunOutput 警告");
    assert!(
        warning.contains("panles.list"),
        "警告文本应含打错的变量名: {warning}"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B13 对照：真实存在的空集合（变量已声明但为空数组）不产生警告——
/// 空集合跳过是合法语义，只有「引用打错」才警告。
#[tokio::test]
async fn for_each_existing_empty_collection_no_warning() {
    let tmp = std::env::temp_dir().join("nuphus_test_b13_empty_ok");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    let body = make_tool_step("body", "mock_echo", serde_json::json!({}));
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "foreach_empty".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: Some(ForEachDef {
                    items: VarRef::Var {
                        var: "panels.list".into(),
                    },
                    item_var: "item".into(),
                }),
                repeat: None,
                until: None,
                max: 100,
                steps: vec![body],
            },
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("b13_empty_ok");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    // panels 声明为空数组（真实存在的空集合）
    let inputs: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::from([("panels".to_string(), serde_json::json!({"list": []}))]);

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id,
            &store,
            &events,
            tool_exec,
            None,
            None,
            None,
            Some(inputs),
            false,
        )
        .await;
    assert!(result.is_ok(), "空集合跳过应成功: {:?}", result.err());

    let mut saw_warning = false;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::StepRunOutput { text, .. } = ev {
            if text.contains("[警告]") {
                saw_warning = true;
            }
        }
    }
    assert!(!saw_warning, "真实空集合不得产生警告（空转是合法语义）");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B14 回归：until 条件写错（永假）跑满 max_iterations 时，运行保持 Ok（兜底
/// 退出是合法语义），但必须发出可观测警告（StepRunOutput，文本含 [警告] 与
/// 迭代上限）——此前静默返回 Ok 无任何提示。
#[tokio::test]
async fn until_max_iterations_emits_warning_but_stays_ok() {
    let tmp = std::env::temp_dir().join("nuphus_test_b14_until_max");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    // 循环体：捕获 attempts 变量，但 until 条件引用打错的变量名 → 求值恒 false
    //（eval_condition 对缺失变量返回空串，Equals("","x") 永假 → 必然跑满 max）
    let body = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "tick".into(),
        action: Action::Tool {
            tool: "mock_echo".into(),
            with: serde_json::json!({"n": "{{_index}}"}),
        },
        capture: Some("attempts".into()),
        ..Default::default()
    };
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "until_typo".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: None,
                repeat: None,
                until: Some(Condition::Equals {
                    equals: vec![
                        VarRef::Var {
                            var: "attempts_typo".into(),
                        },
                        VarRef::Lit("done".into()),
                    ],
                }),
                max: 3, // 小 max：跑满 3 次即触发警告路径
                steps: vec![body],
            },
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("b14_until_max");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(
        result.is_ok(),
        "until 跑满是兜底退出，运行保持 Ok: {:?}",
        result.err()
    );

    let mut warning: Option<String> = None;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::StepRunOutput { text, .. } = ev {
            if text.contains("[警告]") {
                warning = Some(text);
            }
        }
    }
    let warning = warning.expect("until 跑满 max 必须发出 StepRunOutput 警告");
    assert!(
        warning.contains("最大迭代次数") && warning.contains("3"),
        "警告文本应含迭代上限: {warning}"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B14 对照：until 条件满足时（未跑满）不得产生跑满警告。
#[tokio::test]
async fn until_condition_met_no_warning() {
    let tmp = std::env::temp_dir().join("nuphus_test_b14_until_met");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let mut rx = events.subscribe();
    let executor = Executor::new();

    // 循环体捕获 attempts；until 条件引用正确变量名 → 第 1 轮后满足退出
    let body = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "tick".into(),
        action: Action::Tool {
            tool: "mock_echo".into(),
            with: serde_json::json!({"n": "x"}),
        },
        capture: Some("attempts".into()),
        ..Default::default()
    };
    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "until_met".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: None,
                repeat: None,
                until: Some(Condition::NotEmpty {
                    not_empty: VarRef::Var {
                        var: "attempts".into(),
                    },
                }),
                max: 5,
                steps: vec![body],
            },
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("b14_until_met");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "until 满足应成功: {:?}", result.err());

    let mut saw_max_warning = false;
    while let Ok(ev) = rx.try_recv() {
        if let WorkflowEvent::StepRunOutput { text, .. } = ev {
            if text.contains("[警告]") && text.contains("最大迭代次数") {
                saw_max_warning = true;
            }
        }
    }
    assert!(!saw_max_warning, "条件满足退出不得产生跑满警告");
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

// ── B2/B3 回归：break/continue 控制流信号 ──

/// 构造 break 步骤
fn make_break_step(name: &str) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        action: Action::Break { _break: true },
        ..Default::default()
    }
}

/// 构造 continue 步骤
fn make_continue_step(name: &str) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        action: Action::Continue { _continue: true },
        ..Default::default()
    }
}

/// 构造捕获输出的工具步骤
fn make_capture_step(name: &str, var: &str, marker: &str) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        action: Action::Tool {
            tool: "mock_echo".into(),
            with: serde_json::json!({ "marker": marker }),
        },
        capture: Some(var.into()),
        ..Default::default()
    }
}

fn repeat_loop(count: u32, steps: Vec<Step>) -> Step {
    Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "loop".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: None,
                repeat: Some(count),
                until: None,
                max: 100,
                steps,
            },
        },
        ..Default::default()
    }
}

/// B2 回归①：Seq 容器内的 break 必须穿透到 loop——此前容器把哨兵字符串
/// 当普通结果吞掉，循环跑满 5 次（capture 被调用 5 次）。修复后第 1 轮即
/// break，capture 只执行 1 次。
#[tokio::test]
async fn break_inside_seq_container_escapes_to_loop() {
    let tmp = std::env::temp_dir().join("nuphus_test_b2_break_in_seq");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let seq_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "seq_container".into(),
        action: Action::Seq {
            seq: vec![
                make_capture_step("capture", "hit", "x"),
                make_break_step("bail"),
            ],
        },
        ..Default::default()
    };
    let wf_steps = vec![repeat_loop(5, vec![seq_step])];

    let mut wf = Workflow::new("b2_break_in_seq");
    wf.steps = wf_steps;
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_exec = calls.clone();
    let tool_exec = move |_tool: String, _params: serde_json::Value| {
        calls_exec.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move { Ok("ok".to_string()) }
    };
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "break 语义下运行应成功: {:?}", result.err());

    let wf = store.get(&wf_id).await.unwrap();
    let run = wf.run_history.first().expect("run record");
    assert!(
        matches!(run.status, RunStatus::Success),
        "break 不是失败: {:?}",
        run.status
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "第 1 轮 capture 后必须 break（若信号被容器吞掉会跑满 5 次调用）"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B2 回归②：If 容器 then 分支内的 break 穿透到 loop。
#[tokio::test]
async fn break_inside_if_container_escapes_to_loop() {
    let tmp = std::env::temp_dir().join("nuphus_test_b2_break_in_if");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let if_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "if_container".into(),
        action: Action::If {
            def: crate::workflow::types::IfDef {
                condition: Condition::Always { always: true },
                then: vec![make_break_step("bail")],
                else_branch: vec![],
            },
        },
        ..Default::default()
    };
    let wf_steps = vec![repeat_loop(5, vec![if_step])];

    let mut wf = Workflow::new("b2_break_in_if");
    wf.steps = wf_steps;
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let tool_exec = |tool: String, params: serde_json::Value| ok_tool_exec(tool, params);
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "break 语义下运行应成功: {:?}", result.err());

    // 第 1 轮就 break：循环不应产生第 2 轮的 capture/事件；
    // 该工作流无 capture 步骤，快照不含任何 marker
    let wf = store.get(&wf_id).await.unwrap();
    let run = wf.run_history.first().expect("run record");
    assert!(
        matches!(run.status, RunStatus::Success),
        "break 不是失败: {:?}",
        run.status
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B2 回归③：嵌套 loop 内层 break 不逃逸外层——外层应继续完整跑完
/// （外层 3 轮 × 每轮 outer_tick + inner_hit = 6 次工具调用；
/// 若内层 break 逃逸外层则只调 2 次）。
#[tokio::test]
async fn inner_loop_break_does_not_escape_outer_loop() {
    let tmp = std::env::temp_dir().join("nuphus_test_b2_nested_break");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // 外层 repeat 3：每轮 outer capture；内层 repeat 5：第 1 步 capture 后 break
    let inner = repeat_loop(
        5,
        vec![
            make_capture_step("inner_hit", "inner_marker", "y"),
            make_break_step("inner_bail"),
        ],
    );
    let outer_body = vec![make_capture_step("outer_tick", "outer_marker", "o"), inner];
    let wf_steps = vec![repeat_loop(3, outer_body)];

    let mut wf = Workflow::new("b2_nested_break");
    wf.steps = wf_steps;
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_exec = calls.clone();
    let tool_exec = move |_tool: String, _params: serde_json::Value| {
        calls_exec.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move { Ok("ok".to_string()) }
    };
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "嵌套 break 不应失败: {:?}", result.err());

    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        6,
        "内层 break 不得逃逸外层（外层必须完整跑 3 轮）"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B3 回归④：break 时末次迭代变量已提升——for_each 循环第 1 轮 capture 后
/// break，被捕获变量必须可见。此前提升逻辑在 break return 之后被跳过。
#[tokio::test]
async fn break_still_promotes_last_iteration_variables() {
    let tmp = std::env::temp_dir().join("nuphus_test_b3_break_promotes");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    let loop_step = Step {
        id: uuid::Uuid::new_v4().to_string(),
        name: "for_each_break".into(),
        action: Action::Loop {
            def: LoopDef {
                for_each: Some(crate::workflow::types::ForEachDef {
                    items: VarRef::Lit("[\"a\",\"b\",\"c\"]".into()),
                    item_var: "item".into(),
                }),
                repeat: None,
                until: None,
                max: 100,
                steps: vec![
                    make_capture_step("capture_item", "last_seen", "{{item}}"),
                    make_break_step("bail"),
                ],
            },
        },
        ..Default::default()
    };

    let mut wf = Workflow::new("b3_break_promotes");
    wf.steps = vec![loop_step];
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_exec = calls.clone();
    let tool_exec = move |_tool: String, _params: serde_json::Value| {
        calls_exec.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move { Ok("ok".to_string()) }
    };
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(result.is_ok(), "break 语义下运行应成功: {:?}", result.err());

    // B3 核心：break 前的 capture（第 1 次调用）必须已执行——若提升在 return
    // 之前缺失，此计数仍为 1 但变量不可见；配合下方 continue 测试形成互补。
    // 这里断言 break 发生在第 1 轮（只有 1 次工具调用，而非 3 轮 3 次）。
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "for_each 第 1 轮 capture 后必须 break"
    );
    // B3 变量提升验证：break 时 run_record 需含该 capture 步骤的 Success 记录
    let wf = store.get(&wf_id).await.unwrap();
    let run = wf.run_history.first().expect("run record");
    assert!(
        run.steps
            .iter()
            .any(|s| s.step_id != "bail" && s.status == StepRunStatus::Success),
        "break 前的 capture 步骤必须已成功执行并记录"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}

/// B2 回归⑤：continue 语义保持——跳过本轮剩余步骤，循环继续。
#[tokio::test]
async fn continue_skips_rest_of_iteration_and_loop_continues() {
    let tmp = std::env::temp_dir().join("nuphus_test_b2_continue");
    let _ = std::fs::create_dir_all(&tmp);
    let store = WorkflowStore::with_root(tmp.clone());
    let events = EventBus::new();
    let executor = Executor::new();

    // repeat 3：第 1 步 capture "seen"（应执行 3 次），第 2 步 continue，
    // 第 3 步 capture "never"（被 continue 跳过，永不应出现）
    let body = vec![
        make_capture_step("seen", "seen_marker", "s"),
        make_continue_step("skip_rest"),
        make_capture_step("never", "never_marker", "n"),
    ];
    let wf_steps = vec![repeat_loop(3, body)];

    let mut wf = Workflow::new("b2_continue");
    wf.steps = wf_steps;
    let wf_id = wf.id.clone();
    store.save(&wf).await.unwrap();

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_exec = calls.clone();
    let tool_exec = move |_tool: String, _params: serde_json::Value| {
        calls_exec.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async move { Ok("ok".to_string()) }
    };
    let result = executor
        .execute_v2(
            &wf_id, &store, &events, tool_exec, None, None, None, None, false,
        )
        .await;
    assert!(
        result.is_ok(),
        "continue 语义下运行应成功: {:?}",
        result.err()
    );

    // seen（continue 前）应执行 3 轮 = 3 次；never（continue 后）被跳过。
    // 旧语义下若 continue 破坏循环，seen 不会达到 3 次。
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "continue 跳过本轮剩余后循环必须继续（seen 执行 3 次，never 0 次）"
    );
    let _ = tokio::fs::remove_dir_all(&tmp).await;
}
