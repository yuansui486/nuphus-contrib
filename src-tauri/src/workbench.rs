//! Thin Tauri/native adapter. Public workflow state and operations live in the
//! transport-independent nuphus-workbench crate.
pub mod authoring;
use async_trait::async_trait;
use nuphus::workflow::{
    compiler::Compiler,
    events::WorkflowEvent,
    run_context::{RunContext, CURRENT},
    store::WorkflowStore,
    types::Workflow,
    WorkflowRunSource,
};
use nuphus_workbench::{
    auth::Principal,
    service::{Host, Service},
    ApiError, Result, Run, RunStatus, WorkbenchStore,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};

pub struct WorkbenchState {
    pub service: Arc<Service<NativeHost>>,
    pub endpoint: Arc<Mutex<Value>>,
    generations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    // Retained for the host lifetime: a second instance must not mark the first
    // instance's live runs interrupted during startup recovery.
    _instance_lock: std::fs::File,
}

pub struct NativeHost {
    app: AppHandle,
    cancelled: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    views: Mutex<HashMap<String, Value>>,
}

fn native_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::new("native_error", error.to_string())
}

pub fn install(app: &AppHandle) -> Result<()> {
    let root = nuphus::profile::workbench_data_dir();
    std::fs::create_dir_all(&root)?;
    let instance_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("host.lock"))?;
    instance_lock.try_lock().map_err(|_| {
        ApiError::new(
            "already_running",
            "Nuphus Workbench is already running; open it from the system tray",
        )
    })?;
    if let Some(window) = app.get_webview_window("main") {
        window.set_title("Nuphus Workbench").map_err(native_error)?;
        if std::env::args().any(|arg| arg == "--background") {
            window.hide().map_err(native_error)?;
        }
    }
    let store = WorkbenchStore::open(nuphus::profile::workbench_data_dir())?;
    if store.projects()?.is_empty() {
        let directory = store.root().join("workspace");
        std::fs::create_dir_all(&directory)?;
        store.register_project(&directory, "Workspace")?;
    }
    let (_, recovery_errors) = store.recover_available()?;
    for (project, error) in &recovery_errors {
        tracing::warn!(%project, %error, "Workbench project recovery deferred");
    }
    let service = Arc::new(Service::new(
        store,
        NativeHost {
            app: app.clone(),
            cancelled: Arc::new(Mutex::new(HashMap::new())),
            views: Mutex::new(HashMap::new()),
        },
    ));
    let endpoint = Arc::new(Mutex::new(json!({"status":"starting"})));
    #[cfg(feature = "workbench")]
    {
        let service = service.clone();
        let endpoint = endpoint.clone();
        tauri::async_runtime::spawn(async move {
            let port = std::env::var("NUPHUS_WORKBENCH_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(47731);
            match nuphus_workbench::gateway::bind(port).await {
                Ok(listener) => {
                    let address = listener.local_addr().expect("bound loopback listener");
                    if let Ok(mut status) = endpoint.lock() {
                        *status = json!({"status":"listening","url":format!("http://{address}"),"mcp_url":format!("http://{address}/mcp")});
                    }
                    if let Err(error) =
                        axum::serve(listener, nuphus_workbench::gateway::router(service)).await
                    {
                        if let Ok(mut status) = endpoint.lock() {
                            *status = json!({"status":"failed","message":error.to_string()});
                        }
                    }
                }
                Err(error) => {
                    if let Ok(mut status) = endpoint.lock() {
                        *status = json!({"status":"failed","message":error.to_string()});
                    }
                }
            }
        });
    }
    app.manage(WorkbenchState {
        service,
        endpoint,
        generations: Arc::new(Mutex::new(HashMap::new())),
        _instance_lock: instance_lock,
    });
    Ok(())
}

#[tauri::command]
pub async fn workbench_call(app: AppHandle, operation: String, args: Value) -> Result<Value> {
    let state = app
        .try_state::<WorkbenchState>()
        .ok_or_else(|| ApiError::new("edition_unavailable", "Start the Workbench edition"))?;
    state
        .service
        .dispatch(&Principal::LocalUi, &operation, args)
        .await
}

#[tauri::command]
pub fn workbench_clients(app: AppHandle, action: String, args: Value) -> Result<Value> {
    let state = app
        .try_state::<WorkbenchState>()
        .ok_or_else(|| ApiError::new("edition_unavailable", "Start the Workbench edition"))?;
    let store = &state.service.store;
    match action.as_str() {
        "status" => Ok(state.endpoint.lock().map_err(native_error)?.clone()),
        "list" => Ok(serde_json::to_value(store.clients()?)?),
        "create" => {
            let name = nuphus_workbench::service::string(&args, "name")?;
            let projects =
                serde_json::from_value(args.get("projects").cloned().unwrap_or_else(|| json!([])))?;
            let capabilities = serde_json::from_value(
                args.get("capabilities")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            )?;
            let (client, token) = store.create_client(name, projects, capabilities)?;
            Ok(json!({"client":client,"token":token}))
        }
        "revoke" => {
            store.revoke_client(nuphus_workbench::service::string(&args, "client_id")?)?;
            Ok(json!({"revoked":true}))
        }
        _ => Err(ApiError::new("unknown_operation", action)),
    }
}

#[tauri::command]
pub fn workbench_view_state(
    app: AppHandle,
    project_id: String,
    workflow_id: String,
    view: Value,
) -> Result<()> {
    let state = app
        .try_state::<WorkbenchState>()
        .ok_or_else(|| ApiError::new("edition_unavailable", "Start the Workbench edition"))?;
    state.service.store.draft(&project_id, &workflow_id)?;
    state
        .service
        .host
        .views
        .lock()
        .map_err(native_error)?
        .insert(format!("{project_id}/{workflow_id}"), view);
    Ok(())
}

#[async_trait]
impl Host for NativeHost {
    async fn validate(&self, document: &Value, definitions: &[Value]) -> Result<Value> {
        let workflow: Workflow = match serde_json::from_value(document.clone()) {
            Ok(workflow) => workflow,
            Err(error) => {
                return Ok(
                    json!({"passed":false,"errors":[error.to_string()],"warnings":[],"diagnostics":[]}),
                )
            }
        };
        let definitions: Vec<Workflow> = definitions
            .iter()
            .cloned()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        let frozen = WorkflowStore::frozen(PathBuf::new(), definitions);
        let state = self.app.state::<crate::state::AppState>();
        let mut report =
            Compiler::validate_workflow_with_tools(&workflow, &state.tools.get_schemas());
        let calls = Compiler::validate_call_report(&workflow, &frozen).await;
        report.errors.extend(calls.errors);
        report.diagnostics.extend(calls.diagnostics);
        report.passed = report.errors.is_empty();
        Ok(serde_json::to_value(report)?)
    }

    async fn preflight(&self, document: &Value, inputs: &Value) -> Result<Value> {
        let workflow: Workflow = serde_json::from_value(document.clone())?;
        let provided = serde_json::from_value(inputs.clone())?;
        let mut resolved =
            nuphus::workflow::inputs::resolve_declared_inputs(&workflow.inputs, &provided)
                .map_err(|e| ApiError::new("invalid_inputs", e.to_string()))?;
        for spec in &workflow.inputs {
            if spec.sensitive && resolved.contains_key(&spec.name) {
                resolved.insert(spec.name.clone(), json!("[REDACTED]"));
            }
        }
        let mut visible = inputs.clone();
        for (key, value) in resolved {
            visible[&key] = value;
        }
        Ok(visible)
    }

    async fn start(&self, store: WorkbenchStore, run: Run, inputs: Value) -> Result<()> {
        let state = self.app.state::<crate::state::AppState>();
        let (lease, owner) = crate::resource_gate::acquire_execution_body_with_owner(
            &state.automation_gate,
            "workbench.run",
        )
        .map_err(|e| ApiError::new("automation_busy", e))?;
        let engine = state.workflow_engine.clone();
        {
            let mut engine = engine.write().await;
            crate::commands::workflow::inject_workflow_runtime(&state, &mut engine);
        }
        let definitions: Vec<Workflow> = serde_json::from_value(run.snapshots.clone())?;
        let root = definitions
            .iter()
            .find(|w| w.id == run.workflow_id)
            .ok_or_else(|| ApiError::new("invalid_snapshot", "Root workflow is absent"))?;
        let provided: HashMap<String, Value> = serde_json::from_value(if run.debug.is_some() {
            inputs
                .get("runtime_inputs")
                .cloned()
                .unwrap_or_else(|| json!({}))
        } else {
            inputs.clone()
        })?;
        let resolved = if run.debug.is_some() {
            provided.clone().into_iter().collect()
        } else {
            nuphus::workflow::inputs::resolve_declared_inputs(&root.inputs, &provided)
                .map_err(native_error)?
        };
        let debug_request =
            run.debug
                .as_ref()
                .map(|options| nuphus::workflow::debug::DebugRequest {
                    workflow_id: root.id.clone(),
                    steps: root.steps.clone(),
                    inputs: Some(root.inputs.clone()),
                    selected_step_id: options["selected_step_id"]
                        .as_str()
                        .unwrap_or_default()
                        .into(),
                    mode: if options["mode"] == "through" {
                        nuphus::workflow::debug::DebugMode::Through
                    } else {
                        nuphus::workflow::debug::DebugMode::Node
                    },
                    variables: serde_json::from_value(
                        inputs
                            .get("variables")
                            .cloned()
                            .unwrap_or_else(|| json!({})),
                    )
                    .unwrap_or_default(),
                    runtime_inputs: provided.clone(),
                    use_retry_policy: options["use_retry_policy"].as_bool().unwrap_or(false),
                    source: options.get("source").cloned(),
                });
        let secrets: Vec<String> = root
            .inputs
            .iter()
            .filter(|spec| spec.sensitive)
            .filter_map(|spec| resolved.get(&spec.name))
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string())
            })
            .filter(|s| !s.is_empty())
            .collect();
        let event_store = store.clone();
        let event_run = run.clone();
        let event_app = self.app.clone();
        let sink_secrets = secrets.clone();
        let storage_error = Arc::new(Mutex::new(None::<String>));
        let sink_error = storage_error.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let context = Arc::new(RunContext {
            run_id: run.run_id.clone(),
            cancelled: cancelled.clone(),
            project_dir: PathBuf::from(store.project(&run.project_id)?.directory),
            store: WorkflowStore::frozen(
                store.project_data_dir(&run.project_id)?.join("artifacts"),
                definitions,
            ),
            event_sink: Some(Arc::new(move |event| {
                let data = redact_event(event, &sink_secrets);
                let result = event_store.append_event(
                    &event_run.project_id,
                    &event_run.run_id,
                    "workflow.step",
                    &data,
                );
                if let Err(error) = result {
                    if let Ok(mut slot) = sink_error.lock() {
                        *slot = Some(error.to_string());
                    }
                }
                if let WorkflowEvent::StepRunPaused {
                    step_id, reason, ..
                } = event
                {
                    let result = if reason == "用户暂停" || reason == "debug_after_step" {
                        event_store.transition(
                            &event_run.project_id,
                            &event_run.run_id,
                            RunStatus::Paused,
                            None,
                        )
                    } else {
                        event_store.await_human(
                            &event_run.project_id,
                            &event_run.run_id,
                            step_id,
                            redact(json!(reason), &sink_secrets)
                                .as_str()
                                .unwrap_or("Confirmation required"),
                        )
                    };
                    if let Err(e) = result {
                        if let Ok(mut slot) = sink_error.lock() {
                            *slot = Some(e.to_string());
                        }
                    }
                } else if matches!(event, WorkflowEvent::StepRunStarted { .. }) {
                    if let Ok(current) = event_store.run(&event_run.project_id, &event_run.run_id) {
                        if matches!(current.status, RunStatus::Paused | RunStatus::AwaitingHuman) {
                            let _ = event_store.transition(
                                &event_run.project_id,
                                &event_run.run_id,
                                RunStatus::Running,
                                None,
                            );
                        }
                    }
                }
                let _ = event_app.emit(
                    "workbench-event",
                    json!({"project_id":event_run.project_id,"run_id":event_run.run_id}),
                );
            })),
        });
        let debug_session = if let Some(request) = debug_request {
            let engine = engine.read().await;
            Some(
                CURRENT
                    .scope(context.clone(), engine.prepare_debug(request))
                    .await
                    .map_err(native_error)?,
            )
        } else {
            None
        };
        // Preflight only reachable AI nodes, using the narrowed debug store.
        let execution_store = debug_session
            .as_ref()
            .map(|s| &s.store)
            .unwrap_or(&context.store);
        if let Err(error) = configure_run_models(&state, &run.workflow_id, execution_store).await {
            if let Some(session) = &debug_session {
                engine
                    .read()
                    .await
                    .debug_sessions
                    .write()
                    .await
                    .remove(&session.run_id);
                session.recorder.error(error.to_string()).await;
                session.recorder.complete("error").await;
            }
            return Err(error);
        }
        let tools = state.tools.clone();
        let permissions = state.tool_permissions_ref.clone();
        let emitter = crate::emitter::CompoundEmitter::new(self.app.clone(), &state);
        self.cancelled
            .lock()
            .map_err(native_error)?
            .insert(run.run_id.clone(), cancelled.clone());
        let cancellations = self.cancelled.clone();
        store.transition(&run.project_id, &run.run_id, RunStatus::Running, None)?;
        // Run ownership outlives the request and any external connection.
        tauri::async_runtime::spawn(async move {
            let _lease = lease;
            let result = nuphus::automation_gate::with_execution_owner(
                owner,
                CURRENT.scope(context, async {
                    let engine = engine.read().await;
                    let tool_exec = move |tool: String, params: Value| {
                        let tools = tools.clone();
                        let permissions = permissions.clone();
                        async move {
                            let configured = *permissions
                                .lock()
                                .map_err(|_| "Tool permissions unavailable".to_string())?;
                            let policy = nuphus::permissions::PermissionPolicy::new(configured)
                                .with_categories(tools.all_tool_categories());
                            let (allowed, message) = policy.authorize_with_message(&tool);
                            if !allowed {
                                return Err(message);
                            }
                            let result = if tool.starts_with("browser_") {
                                tools.execute_browser_tool(&tool, &params).await
                            } else {
                                tools.execute(&tool, &params).await
                            }
                            .map_err(|e| e.to_string())?;
                            result.into_exec_result()
                        }
                    };
                    let execute = engine.execute_workflow(
                        &run.workflow_id,
                        tool_exec,
                        None,
                        Some(&emitter),
                        Some(
                            debug_session
                                .as_ref()
                                .map(|session| session.runtime_inputs.clone())
                                .unwrap_or(provided),
                        ),
                        true,
                        WorkflowRunSource::External,
                    );
                    if let Some(session) = &debug_session {
                        let result = nuphus::workflow::debug::CURRENT
                            .scope(session.clone(), execute)
                            .await;
                        if session.cancelled.load(Ordering::Relaxed) {
                            cancelled.store(true, Ordering::Relaxed);
                        }
                        if let Err(error) = &result {
                            session.recorder.error(error.to_string()).await;
                        }
                        session
                            .recorder
                            .complete(if cancelled.load(Ordering::Relaxed) {
                                "cancelled"
                            } else if result.is_ok() {
                                "success"
                            } else {
                                "error"
                            })
                            .await;
                        engine.debug_sessions.write().await.remove(&session.run_id);
                        result
                    } else {
                        execute.await
                    }
                }),
            )
            .await;
            let (status, result) = if cancelled.load(Ordering::Relaxed) {
                (RunStatus::Cancelled, json!({"cancelled":true}))
            } else {
                match result {
                    Ok(output) => (RunStatus::Completed, json!({"output":output})),
                    Err(error) => (RunStatus::Failed, json!({"error":error.to_string()})),
                }
            };
            let failure = storage_error.lock().ok().and_then(|slot| slot.clone());
            let (status, result) = if let Some(error) = failure {
                (
                    RunStatus::Failed,
                    json!({"code":"evidence_storage_failed","error":error,"execution_result":result}),
                )
            } else {
                (status, result)
            };
            if let Err(error) = store.transition(
                &run.project_id,
                &run.run_id,
                status,
                Some(redact(result, &secrets)),
            ) {
                tracing::error!("Workbench run finalization failed: {}", error);
            }
            if let Ok(mut active) = cancellations.lock() {
                active.remove(&run.run_id);
            }
        });
        Ok(())
    }

    async fn control(
        &self,
        store: &WorkbenchStore,
        run: &Run,
        action: &str,
        args: &Value,
    ) -> Result<Value> {
        if action == "run.steps" {
            let root = store.project_data_dir(&run.project_id)?.join("artifacts");
            if let Some(id) = args.get("invocation_id").and_then(Value::as_u64) {
                return Ok(serde_json::to_value(
                    nuphus::workflow::trace::read(
                        &root,
                        &run.workflow_id,
                        &run.run_id,
                        run.debug.is_some(),
                        id,
                    )
                    .await
                    .map_err(native_error)?,
                )?);
            }
            let traces =
                nuphus::workflow::trace::list(&root, &run.workflow_id, run.debug.is_some())
                    .await
                    .map_err(native_error)?;
            return Ok(json!(traces.into_iter().find(|t| t.run_id == run.run_id)));
        }
        if run.status.terminal() {
            return Err(ApiError::new(
                "invalid_run_state",
                "Run has finished; create a new run explicitly",
            ));
        }
        let state = self.app.state::<crate::state::AppState>();
        let engine = state.workflow_engine.read().await;
        match action {
            "run.pause" => {
                if run.status != RunStatus::Running {
                    return Err(ApiError::new(
                        "invalid_run_state",
                        "Only running workflows can pause",
                    ));
                }
                engine.pause_workflow(&run.workflow_id).await;
                store.append_event(
                    &run.project_id,
                    &run.run_id,
                    "run.pause_requested",
                    &json!({}),
                )?;
            }
            "run.resume" => {
                if run.status != RunStatus::Paused {
                    return Err(ApiError::new(
                        "invalid_run_state",
                        "Only explicitly paused runs can resume",
                    ));
                }
                engine.resume_workflow(&run.workflow_id).await;
                store.transition(&run.project_id, &run.run_id, RunStatus::Running, None)?;
            }
            "run.cancel" => {
                if let Some(flag) = self
                    .cancelled
                    .lock()
                    .map_err(native_error)?
                    .get(&run.run_id)
                {
                    flag.store(true, Ordering::Relaxed);
                }
                engine.cancel_workflow(&run.workflow_id).await;
                store.append_event(
                    &run.project_id,
                    &run.run_id,
                    "run.cancel_requested",
                    &json!({}),
                )?;
            }
            "run.respond" => {
                let request = nuphus_workbench::service::string(args, "request_id")?;
                let decision = nuphus_workbench::service::string(args, "decision")?;
                store.respond_human(&run.project_id, &run.run_id, request, decision)?;
                if decision == "cancel" {
                    if let Some(flag) = self
                        .cancelled
                        .lock()
                        .map_err(native_error)?
                        .get(&run.run_id)
                    {
                        flag.store(true, Ordering::Relaxed);
                    }
                    engine.cancel_workflow(&run.workflow_id).await;
                } else {
                    engine.resume_workflow(&run.workflow_id).await;
                }
            }
            _ => return Err(ApiError::new("unknown_operation", action)),
        }
        Ok(
            json!({"run_id":run.run_id,"status":store.run(&run.project_id,&run.run_id)?.status,"cancel_requested":action=="run.cancel"}),
        )
    }

    async fn automation(
        &self,
        _project: &nuphus_workbench::Project,
        action: &str,
        args: &Value,
    ) -> Result<Value> {
        let state = self.app.state::<crate::state::AppState>();
        if action == "workflow.schema" {
            return Ok(serde_json::from_str(include_str!(
                "../../src/workflow/step_schema.json"
            ))?);
        }
        if action == "workflow.tools" {
            let filter = args
                .get("tool_filter")
                .and_then(Value::as_str)
                .unwrap_or("");
            return Ok(json!(state
                .tools
                .get_schemas()
                .into_iter()
                .filter(
                    |s| nuphus::tools::registry::is_workflow_step_tool(&s.function.name)
                        && s.function.name.contains(filter)
                )
                .collect::<Vec<_>>()));
        }
        let schemas = state
            .tools
            .get_schemas()
            .into_iter()
            .filter(|schema| {
                schema.function.name.starts_with("desktop_")
                    || schema.function.name.starts_with("browser_")
            })
            .collect::<Vec<_>>();
        if action == "automation.capabilities" {
            return Ok(
                json!({"host":self.capabilities(),"tools":schemas,"observation_tools":OBSERVATION_TOOLS,"results":"run.steps","execution_location":"local"}),
            );
        }
        let tool = nuphus_workbench::service::string(args, "tool")?;
        if !schemas.iter().any(|schema| schema.function.name == tool) {
            return Err(ApiError::new(
                "unknown_tool",
                "Choose an available desktop/browser tool from automation.capabilities",
            ));
        }
        if action == "automation.observe" && !OBSERVATION_TOOLS.contains(&tool) {
            return Err(ApiError::new(
                "invalid_observation",
                "This tool may change state; use automation.execute explicitly",
            ));
        }
        let permissions = *state.tool_permissions_ref.lock().map_err(native_error)?;
        let (allowed, message) = nuphus::permissions::PermissionPolicy::new(permissions)
            .with_categories(state.tools.all_tool_categories())
            .authorize_with_message(tool);
        if !allowed {
            return Err(ApiError::new("permission_denied", message));
        }
        Ok(json!({"validated":true}))
    }

    async fn view(
        &self,
        project: &str,
        workflow: &str,
        action: &str,
        args: &Value,
    ) -> Result<Value> {
        if action == "canvas.open" {
            self.app
                .emit(
                    "workbench-open",
                    json!({"project_id":project,"workflow_id":workflow}),
                )
                .map_err(native_error)?;
            if args.get("focus").and_then(Value::as_bool) == Some(true) {
                if let Some(window) = self.app.get_webview_window("main") {
                    window.show().map_err(native_error)?;
                    window.set_focus().map_err(native_error)?;
                }
            }
        }
        Ok(self
            .views
            .lock()
            .map_err(native_error)?
            .get(&format!("{project}/{workflow}"))
            .cloned()
            .unwrap_or_else(|| json!({"open":false,"selection":null})))
    }

    fn capabilities(&self) -> Value {
        json!({"edition":"workbench","platform":std::env::consts::OS,"workflow_execution":true,
            "direct_automation":true,"concurrent_automation":false,"model_required_for_deterministic_workflows":false})
    }
}

const OBSERVATION_TOOLS: &[&str] = &[
    "desktop_targets_list",
    "desktop_semantic_observe",
    "desktop_semantic_candidate",
    "desktop_verify_state",
    "desktop_screenshot",
    "desktop_window_screenshot",
    "browser_snapshot",
    "browser_extract",
    "browser_list_tabs",
    "browser_cookies_get",
    "browser_screenshot",
    "browser_list_downloads",
];

fn redact_event(event: &WorkflowEvent, secrets: &[String]) -> Value {
    let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
    // Payload redaction must not corrupt event types, IDs, depth or counters.
    for field in ["step_name", "workflow_name", "text", "reason", "message"] {
        if let Some(payload) = value.get_mut(field) {
            *payload = redact(payload.take(), secrets);
        }
    }
    if let Some(status) = value.get_mut("status") {
        if status.is_object() {
            *status = redact(status.take(), secrets);
        }
    }
    value
}

async fn configure_run_models(
    state: &crate::state::AppState,
    root: &str,
    store: &WorkflowStore,
) -> Result<()> {
    use nuphus::workflow::types::{Action, ChatOpts, Step};
    fn collect(steps: &[Step], options: &mut Vec<ChatOpts>, calls: &mut Vec<String>) {
        for step in steps {
            match &step.action {
                Action::Chat { with, .. } => options.push(with.clone()),
                Action::Call { call, .. } => calls.push(call.clone()),
                Action::Seq { seq } => collect(seq, options, calls),
                Action::Loop { def } => collect(&def.steps, options, calls),
                Action::If { def } => {
                    collect(&def.then, options, calls);
                    collect(&def.else_branch, options, calls);
                }
                Action::Wait { auto, .. } => collect(auto, options, calls),
                _ => {}
            }
        }
    }
    let mut options = Vec::new();
    let mut pending = vec![root.to_owned()];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(workflow) = store.get(&id).await {
            if !workflow.dry_run {
                collect(&workflow.steps, &mut options, &mut pending);
            }
        }
    }
    if options.is_empty() {
        return Ok(());
    }
    let failure = |error: String| {
        ApiError::new(
            "model_not_configured",
            format!("Configure the model required by AI steps before running: {error}"),
        )
    };
    let factory = nuphus::llm::ClientFactory::live();
    let registry = factory.registry().map_err(|e| failure(e.to_string()))?;
    let default = crate::commands::config::llm::effective_model_binding(
        &state.llm_config_path,
        &registry,
        "workflow",
    );
    let mut base = None;
    for opts in options {
        let binding = if let Some(model) = &opts.model {
            if let Some(provider) = &opts.provider {
                Some((provider.clone(), model.clone()))
            } else {
                let candidates = registry.find_model_candidates(model);
                match candidates.as_slice() {
                    [] => None,
                    [(provider, model)] => Some((provider.name.clone(), model.id.clone())),
                    _ => {
                        return Err(failure(format!(
                            "Model '{model}' has multiple providers; select one"
                        )))
                    }
                }
            }
        } else {
            None
        };
        let binding = binding
            .or_else(|| default.as_ref().ok().cloned())
            .ok_or_else(|| failure("No workflow model binding".into()))?;
        let client = factory
            .create_client_for(&binding.0, &binding.1)
            .map_err(|e| failure(e.to_string()))?;
        if base.is_none() || opts.model.is_none() {
            base = Some(client);
        }
    }
    let mut engine = state.workflow_engine.write().await;
    engine.set_client_factory(factory);
    if let Some(client) = base {
        engine.set_llm_client(client);
    }
    Ok(())
}

fn redact(value: Value, secrets: &[String]) -> Value {
    if !value.is_string() && secrets.iter().any(|s| *s == value.to_string()) {
        return json!("[REDACTED]");
    }
    match value {
        Value::String(mut text) => {
            for secret in secrets {
                text = text.replace(secret, "[REDACTED]");
            }
            json!(text)
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(|v| redact(v, secrets)).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact(value, secrets)))
                .collect(),
        ),
        other => other,
    }
}
