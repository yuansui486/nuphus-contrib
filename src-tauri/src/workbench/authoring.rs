use super::*;
use nuphus::{
    api::{FunctionDefinition, ToolDefinition},
    runtime::workflow_agent::WorkflowAuthoring,
};

struct Authoring {
    service: Arc<Service<NativeHost>>,
    project: String,
    workflow: String,
}

#[async_trait]
impl WorkflowAuthoring for Authoring {
    fn schemas(&self) -> Vec<ToolDefinition> {
        let mut tools: Vec<_> = nuphus_workbench::catalog::operations()
            .into_iter()
            .filter(|op| {
                matches!(
                    op.name,
                    "canvas.get" | "canvas.update" | "workflow.validate" | "workflow.save"
                )
            })
            .map(|op| {
                let mut schema = op.input_schema;
                if let Some(props) = schema["properties"].as_object_mut() {
                    props.remove("project_id");
                    props.remove("workflow_id");
                }
                if let Some(required) = schema["required"].as_array_mut() {
                    required.retain(|v| v != "project_id" && v != "workflow_id");
                }
                ToolDefinition {
                    tool_type: "function".into(),
                    function: FunctionDefinition {
                        name: op.name.replace('.', "_"),
                        description: Some(op.description.into()),
                        parameters: schema,
                        permission: None,
                    },
                }
            })
            .collect();
        for (name,description,parameters) in [
            ("workbench_schema","Read the authoritative native step schema and workflow tool schemas. Filter tools by name fragment to limit context.",json!({"type":"object","properties":{"tool_filter":{"type":"string"}},"additionalProperties":false})),
            ("workflow_report_progress","Send a short user-facing progress update while continuing work. Never expose private model reasoning.",json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"]})),
        ] { tools.push(ToolDefinition {tool_type:"function".into(),function:FunctionDefinition{name:name.into(),description:Some(description.into()),parameters,permission:None}}); }
        tools
    }
    fn system_prompt(&self) -> String {
        "You are Nuphus Workbench's workflow author, using the existing WorkflowAgent conversation runtime. Respond in the user's language. The active canvas is already bound to your tools; do not request project IDs. First briefly tell the user your approach using workflow_report_progress, then read canvas_get and workbench_schema. Create or modify the native Workflow IR using canvas_update with its exact current revision. Preserve existing user edits, IDs and fields not implicated by the request. On revision_conflict reread and reconcile, never blindly overwrite. Native validation is authoritative: validate, repair actionable errors, and publish with workflow_save when valid. Incomplete drafts can be stored without claiming they are runnable. UI canvas renders changes automatically. Do not write workflow files, invoke a shell, invent tools, or claim a run/test happened: this authoring panel edits but does not execute; the user starts a published version with Run. Deterministic tools, waits, conditions, loops, child workflows, inputs and Agent nodes use the existing step schema. Prefer semantic desktop targets, with mouse/OCR as supported alternatives when needed; don't fabricate live UI IDs or coordinates. If necessary live target information is missing, declare runtime inputs/observation steps or ask the user a concise question. A configured model is needed only for this generation and AI nodes, not deterministic execution. Give concise progress during longer work, finish with what changed and any unresolved validation issue. Never send private unrelated app content to a model.".into()
    }
    async fn execute(&self, tool: &str, params: &Value) -> nuphus::ToolResult {
        let result: Result<Value> = async {
            let draft=self.service.store.draft(&self.project,&self.workflow)?;
            if draft.authoring_mode != nuphus_workbench::AuthoringMode::Internal {
                return Err(ApiError::new("authoring_mode_changed","This canvas switched to external authoring; stop this generation"));
            }
            if tool=="workbench_schema" {
                let filter=params.get("tool_filter").and_then(Value::as_str).unwrap_or("");
                let state=self.service.host.app.state::<crate::state::AppState>();
                let schemas=state.tools.get_schemas().into_iter().filter(|s|nuphus::tools::registry::is_workflow_step_tool(&s.function.name) && s.function.name.contains(filter)).collect::<Vec<_>>();
                return Ok(json!({"step_schema":serde_json::from_str::<Value>(include_str!("../../../src/workflow/step_schema.json"))?,"tools":schemas}));
            }
            let operation=match tool {
                "canvas_get"=>"canvas.get", "canvas_update"=>"canvas.update",
                "workflow_validate"=>"workflow.validate", "workflow_save"=>"workflow.save",
                _=>return Err(ApiError::new("unknown_operation","Only canvas authoring tools are available here")),
            };
            let mut args=params.clone();
            if !args.is_object() { return Err(ApiError::new("invalid_params","Object required")); }
            args["project_id"]=json!(self.project);
            args["workflow_id"]=json!(self.workflow);
            self.service.dispatch(&Principal::LocalUi,operation,args).await
        }.await;
        match result {
            Ok(value) => nuphus::ToolResult::success(value.to_string()),
            Err(error) => nuphus::ToolResult::failure(error.to_string()),
        }
    }
}

struct AuthoringEmitter {
    app: AppHandle,
    project: String,
    workflow: String,
    turn: String,
}
impl nuphus::agent::events::EventEmitter for AuthoringEmitter {
    fn emit(&self, event: nuphus::agent::events::NuphusEvent) {
        use nuphus::agent::events::NuphusEvent;
        // No reasoning chunks, model credentials or unrelated app events enter
        // this workflow-bound panel.
        let visible = match event {
            NuphusEvent::AssistantProgress { text, .. } => json!({"type":"progress","text":text}),
            NuphusEvent::ToolCallStart { tool_name, .. } => json!({"type":"tool","name":tool_name}),
            NuphusEvent::ToolCallEnd { success, error, .. } if !success => {
                json!({"type":"error","text":error})
            }
            _ => return,
        };
        let _=self.app.emit("workbench-authoring",json!({"project_id":self.project,"workflow_id":self.workflow,"turn_id":self.turn,"event":visible}));
    }
}

#[tauri::command]
pub async fn workbench_generate(
    app: AppHandle,
    project_id: String,
    workflow_id: String,
    action: String,
    input: Option<String>,
) -> Result<Value> {
    let state = app
        .try_state::<WorkbenchState>()
        .ok_or_else(|| ApiError::new("edition_unavailable", "Start Workbench"))?;
    let draft = state.service.store.draft(&project_id, &workflow_id)?;
    let key = format!("{project_id}/{workflow_id}");
    if action == "history" {
        return Ok(
            json!({"session":state.service.store.authoring_session(&project_id,&workflow_id)?,"busy":state.generations.lock().map_err(native_error)?.contains_key(&key)}),
        );
    }
    if action == "cancel" {
        if let Some(flag) = state.generations.lock().map_err(native_error)?.get(&key) {
            flag.store(true, Ordering::Relaxed);
        }
        return Ok(json!({"cancel_requested":true}));
    }
    if action != "start" {
        return Err(ApiError::new("unknown_operation", action));
    }
    if draft.authoring_mode != nuphus_workbench::AuthoringMode::Internal {
        return Err(ApiError::new(
            "external_authoring",
            "Switch this canvas to internal generation first",
        ));
    }
    let input = input
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ApiError::new("invalid_params", "Describe the workflow changes"))?;
    let factory = nuphus::llm::ClientFactory::live();
    let registry = factory.registry().map_err(|_| {
        ApiError::new(
            "model_not_configured",
            "Configure a workflow model in Models before using internal generation",
        )
    })?;
    let native = app.state::<crate::state::AppState>();
    let binding = crate::commands::config::llm::effective_model_binding(
        &native.llm_config_path,
        &registry,
        "workflow",
    )
    .map_err(native_error)?;
    let llm = factory
        .create_client_for(&binding.0, &binding.1)
        .map_err(native_error)?;
    let tools = nuphus::ToolRegistry::work_agent(); // isolated generation signals; no native execution tools are exposed
    let emitter = Arc::new(AuthoringEmitter {
        app: app.clone(),
        project: project_id.clone(),
        workflow: workflow_id.clone(),
        turn: uuid::Uuid::new_v4().to_string(),
    });
    let turn = emitter.turn.clone();
    let mut agent = nuphus::runtime::WorkflowAgent::new(
        llm,
        tools,
        Some(emitter),
        None,
        binding.1,
        "用户".into(),
        "Nuphus Workbench".into(),
        nuphus::permissions::ToolPermissions::default(),
        0.5,
    );
    if let Some(session) = state
        .service
        .store
        .authoring_session(&project_id, &workflow_id)?
    {
        *agent.session_mut() = serde_json::from_value(session)?;
    }
    agent.set_authoring(Arc::new(Authoring {
        service: state.service.clone(),
        project: project_id.clone(),
        workflow: workflow_id.clone(),
    }));
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut generations = state.generations.lock().map_err(native_error)?;
        if generations.contains_key(&key) {
            return Err(ApiError::new(
                "generation_busy",
                "This workflow already has a generation in progress",
            ));
        }
        generations.insert(key.clone(), cancel.clone());
    }
    let store = state.service.store.clone();
    let generations = state.generations.clone();
    let returned = turn.clone();
    tauri::async_runtime::spawn(async move {
        let result = agent.run(&input, &None, &cancel).await;
        let session_result = serde_json::to_value(agent.session())
            .map_err(ApiError::from)
            .and_then(|session| store.save_authoring_session(&project_id, &workflow_id, &session));
        let event = match (result, session_result) {
            (Ok(output), Ok(())) => {
                json!({"type":"completed","success":output.success,"text":output.message})
            }
            (Err(error), _) => json!({"type":"completed","success":false,"text":error.to_string()}),
            (_, Err(error)) => json!({"type":"completed","success":false,"text":error.to_string()}),
        };
        if let Ok(mut active) = generations.lock() {
            active.remove(&key);
        }
        let _ = app.emit(
            "workbench-authoring",
            json!({"project_id":project_id,"workflow_id":workflow_id,"turn_id":turn,"event":event}),
        );
    });
    Ok(json!({"turn_id":returned}))
}
