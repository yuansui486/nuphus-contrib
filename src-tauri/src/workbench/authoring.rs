use super::*;
use nuphus::{
    api::{FunctionDefinition, ToolDefinition},
    runtime::workflow_agent::WorkflowAuthoring,
};

struct Authoring {
    service: Arc<Service<NativeHost>>,
    project: String,
    workflow: String,
    epoch: String,
    cancel: Arc<AtomicBool>,
    runs: Arc<Mutex<Vec<String>>>,
    turn: String,
}

mod host;

pub(super) struct Generation {
    pub cancel: Arc<AtomicBool>,
    signals: nuphus::state::SharedSignals,
    pending: Arc<Mutex<HashMap<String, Value>>>,
    turn: String,
}

impl Generation {
    fn respond(&self, response: &Value) -> Result<Value> {
        if response["turn_id"] != self.turn || self.cancel.load(Ordering::SeqCst) {
            return Err(ApiError::new(
                "stale_prompt",
                "This prompt belongs to an inactive turn",
            ));
        }
        let id = response["action_id"]
            .as_str()
            .ok_or_else(|| ApiError::new("invalid_params", "action_id required"))?;
        let mut pending = self.pending.lock().map_err(native_error)?;
        let prompt = pending
            .get(id)
            .ok_or_else(|| ApiError::new("stale_prompt", "Prompt expired"))?;
        if prompt["type"] == "security_check" {
            let approved = response["approved"]
                .as_bool()
                .ok_or_else(|| ApiError::new("invalid_params", "approved required"))?;
            if nuphus::security::approval::get(&self.signals, id)
                .is_some_and(|p| p.kind == nuphus::tools::desktop_approval::KIND)
            {
                nuphus::tools::desktop_approval::resolve(&self.signals, id, approved)
                    .map_err(native_error)?;
            } else {
                if approved && response["session"] == true {
                    if let Some(tool) = prompt["tool"].as_str() {
                        nuphus::security::approve_session_tool(&self.signals, tool);
                    }
                }
                nuphus::security::set_security_result(&self.signals, id, approved);
            }
        } else if response["cancelled"] == true {
            nuphus::security::user_input::cancel(&self.signals, id);
        } else {
            nuphus::security::user_input::submit(
                &self.signals,
                id,
                response["value"].as_str().unwrap_or("").to_string(),
            );
        }
        pending.remove(id);
        Ok(json!({"responded":true}))
    }
}

struct AuthoringEmitter {
    app: AppHandle,
    epoch: String,
    project: String,
    workflow: String,
    turn: String,
    pending: Arc<Mutex<HashMap<String, Value>>>,
}
impl nuphus::agent::events::EventEmitter for AuthoringEmitter {
    fn emit(&self, event: nuphus::agent::events::NuphusEvent) {
        if self
            .app
            .state::<WorkbenchState>()
            .service
            .host
            .session_epoch()
            .ok()
            .flatten()
            .as_deref()
            != Some(&self.epoch)
        {
            return;
        }
        let Some(visible) = authoring_event(event) else {
            return;
        };
        if let Some(id) = visible["action_id"].as_str() {
            if let Ok(mut pending) = self.pending.lock() {
                if visible["type"] == "prompt_timeout" {
                    pending.remove(id);
                } else {
                    pending.insert(id.into(), visible.clone());
                }
            }
        }
        let _=self.app.emit("workbench-authoring",json!({"project_id":self.project,"workflow_id":self.workflow,"turn_id":self.turn,"event":visible}));
    }
}

fn authoring_event(event: nuphus::agent::events::NuphusEvent) -> Option<Value> {
    use nuphus::agent::events::NuphusEvent;
    // Only public text from this authoring agent enters the panel. Thinking,
    // subtask output and unrelated app events remain excluded.
    match event {
        NuphusEvent::LlmTextDelta {
            text,
            is_thinking: false,
            from_task: false,
        } => Some(json!({"type":"progress","text":text,"delta":true})),
        NuphusEvent::ToolCallStart { tool_name, .. } => {
            Some(json!({"type":"tool","name":tool_name}))
        }
        NuphusEvent::ToolCallEnd {
            success: false,
            error,
            ..
        } => Some(json!({"type":"error","text":error})),
        NuphusEvent::ToolCallEnd {
            tool_name,
            success,
            output_preview,
            ..
        } => Some(
            json!({"type":"tool_result","name":tool_name,"success":success,"text":output_preview}),
        ),
        event @ (NuphusEvent::SecurityCheck { .. }
        | NuphusEvent::UserInputRequest { .. }
        | NuphusEvent::PromptTimeout { .. }) => serde_json::to_value(event).ok(),
        _ => None,
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
    let epoch = state.service.host.session_epoch()?.unwrap_or_default();
    let key = format!("{project_id}/{workflow_id}");
    if action == "history" {
        let generations = state.generations.lock().map_err(native_error)?;
        let active = generations.get(&key);
        let pending = active
            .map(|g| {
                g.pending
                    .lock()
                    .map(|p| p.values().cloned().collect::<Vec<_>>())
                    .map_err(native_error)
            })
            .transpose()?
            .unwrap_or_default();
        return Ok(
            json!({"session":state.service.store.authoring_session(&project_id,&workflow_id)?,"busy":active.is_some(),"turn_id":active.map(|g| &g.turn),"pending":pending}),
        );
    }
    if action == "cancel" {
        if let Some(flag) = state.generations.lock().map_err(native_error)?.get(&key) {
            flag.cancel.store(true, Ordering::Relaxed);
        }
        return Ok(json!({"cancel_requested":true}));
    }
    if action == "respond" {
        let response: Value = serde_json::from_str(input.as_deref().unwrap_or("{}"))?;
        let generations = state.generations.lock().map_err(native_error)?;
        let active = generations
            .get(&key)
            .ok_or_else(|| ApiError::new("stale_prompt", "This authoring turn has ended"))?;
        return active.respond(&response);
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
    let cancel = Arc::new(AtomicBool::new(false));
    let signals = nuphus::state::new_shared_signals();
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let runs = Arc::new(Mutex::new(Vec::new()));
    let mut tools = nuphus::ToolRegistry::work_agent();
    tools.set_signals(signals.clone());
    tools.set_cancel_flag(cancel.clone());
    if let Some(gate) = native.tools.automation_gate() {
        tools.set_automation_gate(gate);
    }
    tools.set_enhanced_mode(native.workflow_enhanced_mode.load(Ordering::Relaxed));
    let emitter = Arc::new(AuthoringEmitter {
        app: app.clone(),
        epoch: epoch.clone(),
        project: project_id.clone(),
        workflow: workflow_id.clone(),
        turn: uuid::Uuid::new_v4().to_string(),
        pending: pending.clone(),
    });
    let turn = emitter.turn.clone();
    nuphus::tools::desktop_approval::install_host(&signals, emitter.clone(), cancel.clone());
    let mut agent = nuphus::runtime::WorkflowAgent::new(
        llm,
        tools,
        Some(emitter),
        None,
        binding.1,
        "用户".into(),
        "Nuphus Workbench".into(),
        *native.tool_permissions_ref.lock().map_err(native_error)?,
        0.5,
    );
    if let Some(session) = state
        .service
        .store
        .authoring_session(&project_id, &workflow_id)?
    {
        *agent.session_mut() = serde_json::from_value(session)?;
    }
    let authoring = Arc::new(Authoring {
        service: state.service.clone(),
        project: project_id.clone(),
        workflow: workflow_id.clone(),
        epoch: epoch.clone(),
        cancel: cancel.clone(),
        runs: runs.clone(),
        turn: turn.clone(),
    });
    agent.set_authoring(authoring.clone());
    let store = state.service.store.pinned()?;
    let project = store.project(&project_id)?;
    let data_dir = store.project_data_dir(&project_id)?.join("author");
    let context = Arc::new(RunContext {
        run_id: turn.clone(),
        project_dir: PathBuf::from(&project.directory),
        cancelled: cancel.clone(),
        store: WorkflowStore::frozen(data_dir.join("workflow-snapshots"), vec![]),
        event_sink: None,
    });
    {
        let mut generations = state.generations.lock().map_err(native_error)?;
        if generations.contains_key(&key) {
            return Err(ApiError::new(
                "generation_busy",
                "This workflow already has a generation in progress",
            ));
        }
        generations.insert(
            key.clone(),
            Generation {
                cancel: cancel.clone(),
                signals,
                pending,
                turn: turn.clone(),
            },
        );
    }
    let generations = state.generations.clone();
    let returned = turn.clone();
    tauri::async_runtime::spawn(nuphus::profile::PRODUCT_EPOCH.scope(epoch, async move {
        let result = nuphus::automation_gate::with_execution_owner(format!("authoring:{turn}"), nuphus::profile::AUTHOR_DATA_DIR.scope(data_dir, CURRENT.scope(context, async {
        let run = agent.run(&input, &None, &cancel);
        tokio::pin!(run);
        tokio::select! {
            result=&mut run=>result,
            _=async {
                loop {
                    if authoring.check_context().is_err() || nuphus::profile::require_product().is_err() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }=>{ cancel.store(true, Ordering::SeqCst); run.await },
        }
        }))).await;
        if cancel.load(Ordering::SeqCst) {
            let run_ids = runs.lock().map(|r| r.clone()).unwrap_or_default();
            for id in run_ids {
                if let Ok(run) = store.run(&project_id, &id) {
                    if !run.status.terminal() {
                        let _ = authoring.service.host.control(&store, &run, "run.cancel", &json!({})).await;
                    }
                }
            }
        }
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
        if nuphus::profile::require_product().is_ok() {
        let _ = app.emit(
            "workbench-authoring",
            json!({"project_id":project_id,"workflow_id":workflow_id,"turn_id":turn,"event":event}),
        );
        }
    }));
    Ok(json!({"turn_id":returned}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuphus::agent::events::NuphusEvent;

    #[test]
    fn authoring_replies_reach_only_the_active_upstream_signals() {
        let generation = Generation {
            cancel: Arc::new(AtomicBool::new(false)),
            signals: nuphus::state::new_shared_signals(),
            pending: Arc::new(Mutex::new(HashMap::new())),
            turn: "active".into(),
        };
        let id = nuphus::security::user_input::add(
            &generation.signals,
            "Details",
            "Steps",
            false,
            "step_form",
            None,
            None,
            None,
            None,
            None,
            None,
            Some("Prepare"),
        );
        generation
            .pending
            .lock()
            .unwrap()
            .insert(id.clone(), json!({"type":"user_input_request"}));
        let value = json!({"stage":"Prepare","steps":["Open app"]}).to_string();
        assert_eq!(
            generation
                .respond(&json!({"turn_id":"old","action_id":id,"value":value}))
                .unwrap_err()
                .code,
            "stale_prompt"
        );
        assert!(nuphus::security::user_input::poll_response(&generation.signals, &id).is_none());
        generation
            .respond(&json!({"turn_id":"active","action_id":id,"value":value}))
            .unwrap();
        assert_eq!(
            nuphus::security::user_input::poll_response(&generation.signals, &id),
            Some(value)
        );
        assert!(generation.pending.lock().unwrap().is_empty());

        generation.pending.lock().unwrap().insert(
            "permission".into(),
            json!({"type":"security_check","tool":"system_shell"}),
        );
        generation.respond(&json!({"turn_id":"active","action_id":"permission","approved":true,"session":true})).unwrap();
        assert!(nuphus::security::is_session_approved(
            &generation.signals,
            "system_shell"
        ));
        assert!(!nuphus::security::is_session_approved(
            &nuphus::state::new_shared_signals(),
            "system_shell"
        ));
        generation.cancel.store(true, Ordering::SeqCst);
        assert_eq!(
            generation
                .respond(&json!({"turn_id":"active","action_id":"permission","approved":true}))
                .unwrap_err()
                .code,
            "stale_prompt"
        );
    }

    #[test]
    fn authoring_preserves_upstream_prompt_fields_and_timeout() {
        let prompt = json!({"type":"user_input_request","action_id":"input","title":"Details","prompt":"Steps","sensitive":false,"input_type":"step_form","default_stage":"Prepare"});
        assert_eq!(
            authoring_event(serde_json::from_value(prompt.clone()).unwrap()),
            Some(prompt)
        );
        assert_eq!(
            authoring_event(NuphusEvent::PromptTimeout {
                action_id: "input".into()
            }),
            Some(json!({"type":"prompt_timeout","action_id":"input"}))
        );
    }

    #[test]
    fn authoring_stream_exposes_public_text_but_not_thinking_or_subtasks() {
        for (is_thinking, from_task) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let event = authoring_event(NuphusEvent::LlmTextDelta {
                text: "chunk".into(),
                is_thinking,
                from_task,
            });
            if !is_thinking && !from_task {
                assert_eq!(
                    event,
                    Some(json!({"type":"progress","text":"chunk","delta":true}))
                );
            } else {
                assert!(event.is_none());
            }
        }
        assert!(authoring_event(NuphusEvent::DirectResponse {
            message: "unrelated".into()
        })
        .is_none());
    }
}
