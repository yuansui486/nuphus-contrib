use super::*;
use crate::api::{
    AssistantEvent, FunctionDefinition, MessageRequest, ProviderKind, ToolDefinition,
};
use serde_json::{json, Value};
use std::sync::Mutex;

struct SchemaHost;

fn schema_result() -> Value {
    json!({"step_schema": serde_json::from_str::<Value>(include_str!("../../workflow/step_schema.json")).unwrap(), "tools": []})
}

#[async_trait::async_trait]
impl WorkflowAuthoring for SchemaHost {
    fn authorize(&self) -> Result<crate::permissions::ToolPermissions, String> {
        Ok(crate::permissions::ToolPermissions::default())
    }
    fn schemas(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            tool_type: "function".into(),
            function: FunctionDefinition {
                name: "workbench_schema".into(),
                description: Some("Read the native schema".into()),
                parameters: json!({"type":"object"}),
                permission: None,
            },
        }]
    }

    fn system_prompt(&self) -> String {
        "Read workbench_schema before designing a workflow.".into()
    }

    async fn execute(&self, _: &str, _: &Value) -> ToolResult {
        ToolResult::success(schema_result().to_string())
    }
}

struct CapturingModel(Mutex<Vec<MessageRequest>>);

#[async_trait::async_trait]
impl ApiClient for CapturingModel {
    async fn stream(&self, request: MessageRequest) -> crate::Result<Vec<AssistantEvent>> {
        let mut requests = self.0.lock().unwrap();
        requests.push(request);
        Ok(if requests.len() == 1 {
            vec![
                AssistantEvent::ToolUse {
                    id: "schema-call".into(),
                    name: "workbench_schema".into(),
                    input: "{}".into(),
                },
                AssistantEvent::MessageStop,
            ]
        } else {
            vec![
                AssistantEvent::TextDelta("Schema received.".into()),
                AssistantEvent::MessageStop,
            ]
        })
    }
    fn model_name(&self) -> &str {
        "mock-model"
    }
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::MiniMax
    }
    fn provider_name(&self) -> &str {
        "mock"
    }
}

#[tokio::test]
async fn authoring_schema_reaches_next_model_request_without_truncation() {
    let model = Arc::new(CapturingModel(Mutex::new(vec![])));
    let mut agent = WorkflowAgent::new(
        model.clone(),
        ToolRegistry::work_agent(),
        None,
        None,
        "mock-model".into(),
        "用户".into(),
        "Lingque".into(),
        crate::permissions::ToolPermissions::default(),
        0.5,
    );
    agent.set_authoring(Arc::new(SchemaHost));
    let root = tempfile::tempdir().unwrap();
    crate::profile::AUTHOR_DATA_DIR
        .scope(
            root.path().to_path_buf(),
            agent.run("Read the schema", &None, &AtomicBool::new(false)),
        )
        .await
        .unwrap();
    let requests = model.0.lock().unwrap();
    assert!(requests.len() >= 2);
    let expected = schema_result().to_string();
    assert!(expected.chars().count() > 8000);
    // Check the actual transport request, not merely the tool producer.
    fn contains(value: &Value, expected: &str) -> bool {
        match value {
            Value::String(text) => text == expected,
            Value::Array(values) => values.iter().any(|value| contains(value, expected)),
            Value::Object(values) => values.values().any(|value| contains(value, expected)),
            _ => false,
        }
    }
    assert!(requests[1]
        .messages
        .iter()
        .any(|message| contains(message, &expected)));
}

#[test]
fn authoring_schema_exception_does_not_disable_normal_output_limits() {
    assert!(
        crate::utils::truncate_tool_output(&"x".repeat(9000), 8000, "workbench_schema")
            .contains("截断")
    );
    let output = json!({"text":"x".repeat(9000)}).to_string();
    assert!(crate::utils::truncate_tool_output(&output, 8000, "ordinary_tool").contains("截断"));
}

#[test]
fn authoring_inherits_upstream_tools_and_methodology() {
    let model = Arc::new(CapturingModel(Mutex::new(vec![])));
    let mut agent = WorkflowAgent::new(
        model,
        ToolRegistry::work_agent(),
        None,
        None,
        "mock-model".into(),
        "用户".into(),
        "Lingque".into(),
        crate::permissions::ToolPermissions::all(),
        0.5,
    );
    let upstream = agent.get_filtered_schemas();
    let original = agent.build_system_prompt();
    agent.set_authoring(Arc::new(SchemaHost));
    let hosted = agent.get_filtered_schemas();
    for tool in &upstream {
        assert!(
            hosted.iter().any(|candidate| candidate == tool),
            "upstream tool lost: {}",
            tool.function.name
        );
    }
    let prompt = agent.build_system_prompt();
    for marker in [
        "L0 WorkAgent Constitution",
        "skill_read workflow-design",
        "skill_read workflow-interaction",
        "skill_read workflow-recovery",
        "Explore → Solidify → Design → Verify → Decide",
    ] {
        assert!(original.contains(marker));
        assert!(
            prompt.contains(marker),
            "upstream methodology lost: {marker}"
        );
    }
    assert!(original.contains("plugin/workflows/{id}/"));
    assert!(!prompt.contains("plugin/workflows/{id}/"));
    assert!(prompt.contains("Read workbench_schema"));
    assert!(!original.contains("{workflow_delivery}"));
}

struct MutableHost(std::sync::atomic::AtomicU8);
#[async_trait::async_trait]
impl WorkflowAuthoring for MutableHost {
    fn schemas(&self) -> Vec<ToolDefinition> {
        SchemaHost.schemas()
    }
    fn system_prompt(&self) -> String {
        String::new()
    }
    fn authorize(&self) -> Result<crate::permissions::ToolPermissions, String> {
        match self.0.load(Ordering::SeqCst) {
            0 => Ok(crate::permissions::ToolPermissions::all()),
            1 => Ok(crate::permissions::ToolPermissions::none()),
            _ => Err("session revoked".into()),
        }
    }
    async fn execute(&self, tool: &str, params: &Value) -> ToolResult {
        SchemaHost.execute(tool, params).await
    }
}

#[tokio::test]
async fn authoring_refreshes_permissions_and_checks_host_session_each_call() {
    let mut agent = WorkflowAgent::new(
        Arc::new(CapturingModel(Mutex::new(vec![]))),
        ToolRegistry::work_agent(),
        None,
        None,
        "mock-model".into(),
        "用户".into(),
        "Lingque".into(),
        crate::permissions::ToolPermissions::none(),
        0.5,
    );
    let host = Arc::new(MutableHost(std::sync::atomic::AtomicU8::new(0)));
    agent.set_authoring(host.clone());
    let call = ToolCall {
        id: "1".into(),
        tool: "workbench_schema".into(),
        params: json!({}),
    };
    let cancel = AtomicBool::new(false);
    assert!(agent.check_tool_safety(&call, &cancel).await.is_none());
    assert!(agent.tool_permissions.file_access);
    host.0.store(1, Ordering::SeqCst);
    assert!(agent.check_tool_safety(&call, &cancel).await.is_none());
    assert!(!agent.tool_permissions.file_access);
    host.0.store(2, Ordering::SeqCst);
    assert!(agent
        .check_tool_safety(&call, &cancel)
        .await
        .unwrap()
        .error
        .unwrap()
        .contains("revoked"));
}

#[tokio::test]
async fn authoring_data_stays_in_project_through_blocking_tools() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let mut tools = ToolRegistry::new();
    tools.register(crate::tools::registry::ToolDef {
        name: "authoring_test_data".into(),
        description: String::new(),
        parameters: json!({}),
        category: crate::permissions::ToolCategory::Core,
        depends_on: vec![],
        executor: |params, _| {
            let conn = crate::store::db::acquire().map_err(|e| e.to_string())?;
            conn.execute_batch("CREATE TABLE IF NOT EXISTS author_test(value TEXT)")
                .map_err(|e| e.to_string())?;
            if let Some(value) = params["value"].as_str() {
                conn.execute("INSERT INTO author_test VALUES (?1)", [value])
                    .map_err(|e| e.to_string())?;
            }
            let count: i64 = conn
                .query_row("SELECT count(*) FROM author_test", [], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            Ok(ToolResult::success(
                json!({"count":count,"root":crate::utils::nuphus_data_dir(),
                    "project":crate::utils::active_project(),
                    "tag":crate::utils::active_project_tag(),
                    "configured":crate::utils::has_configured_project_dir(),
                    "file":crate::utils::resolve_user_path("params.json")})
                .to_string(),
            ))
        },
    });
    for (root, value, expected) in [
        (first.path(), json!({"value":"first"}), 1),
        (second.path(), json!({}), 0),
        (first.path(), json!({}), 1),
    ] {
        let context = Arc::new(crate::workflow::run_context::RunContext {
            run_id: "author-test".into(),
            project_dir: root.to_path_buf(),
            cancelled: Arc::new(AtomicBool::new(false)),
            store: crate::workflow::store::WorkflowStore::frozen(root.join("snapshots"), vec![]),
            event_sink: None,
        });
        let result = crate::profile::AUTHOR_DATA_DIR
            .scope(
                root.to_path_buf(),
                crate::workflow::run_context::CURRENT
                    .scope(context, tools.execute("authoring_test_data", &value)),
            )
            .await
            .unwrap();
        let parsed: Value = serde_json::from_str(&result.output.unwrap()).unwrap();
        assert_eq!(parsed["count"], expected);
        assert_eq!(parsed["root"], root.to_string_lossy().as_ref());
        assert_eq!(parsed["project"][1], parsed["root"]);
        assert_eq!(parsed["project"][0], parsed["tag"]);
        assert_eq!(parsed["configured"], true);
        assert_eq!(
            parsed["file"],
            root.join("params.json").to_string_lossy().as_ref()
        );
    }
}
