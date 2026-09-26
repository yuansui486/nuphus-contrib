//! Workbench persistence/coordinator; the native host supplies the upstream clock.
//! Claims are committed BEFORE execution. Recovery never replays claimed work.
use crate::{
    service::{string, Host, Service},
    store::{event, load_draft, now},
    ApiError, Draft, Result, WorkbenchStore,
};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Serialize, Deserialize)]
pub struct Binding {
    pub workflow_id: String,
    pub config: Value,
    pub generation: String,
    pub anchor_at: i64,
    pub next_at: i64,
    // Never serialize bindings directly in a public response.
    pub sealed_inputs: String,
    #[serde(default)]
    pub sensitive_inputs: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub workflow_id: String,
    pub workflow_title: String,
    pub due_at: i64,
    pub status: String,
    pub reason: Option<String>,
    pub run_id: Option<String>,
}

impl WorkbenchStore {
    pub fn schedules(&self, project: &str) -> Result<Vec<Binding>> {
        let conn = self.project_db(project)?;
        let mut query = conn.prepare("SELECT body FROM schedules ORDER BY workflow_id")?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|body| Ok(serde_json::from_str(&body)?))
            .collect()
    }

    pub fn schedule(&self, project: &str, workflow: &str) -> Result<Option<Binding>> {
        let body: Option<String> = self
            .project_db(project)?
            .query_row(
                "SELECT body FROM schedules WHERE workflow_id=?1",
                [workflow],
                |r| r.get(0),
            )
            .optional()?;
        body.map(|body| Ok(serde_json::from_str(&body)?))
            .transpose()
    }

    fn save_schedule(
        &self,
        project: &str,
        workflow: &str,
        revision: u64,
        binding: Option<&Binding>,
    ) -> Result<Draft> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut draft = load_draft(&tx, workflow)?;
        if draft.revision != revision {
            return Err(ApiError::new(
                "revision_conflict",
                "Reload the changed workflow before configuring its schedule",
            ));
        }
        draft.document["schedule"] = binding.map(|b| b.config.clone()).unwrap_or(Value::Null);
        draft.revision += 1;
        draft.updated_at = now();
        if let Some(binding) = binding {
            tx.execute("INSERT INTO schedules(workflow_id,body) VALUES(?1,?2) ON CONFLICT(workflow_id) DO UPDATE SET body=excluded.body", params![workflow, serde_json::to_string(binding)?])?;
        } else {
            tx.execute("DELETE FROM schedules WHERE workflow_id=?1", [workflow])?;
        }
        tx.execute(
            "UPDATE drafts SET revision=?1,body=?2 WHERE workflow_id=?3",
            params![draft.revision, serde_json::to_string(&draft)?, workflow],
        )?;
        event(
            &tx,
            project,
            workflow,
            None,
            "schedule.changed",
            &json!({"revision":draft.revision,"config":draft.document["schedule"]}),
        )?;
        tx.commit()?;
        Ok(draft)
    }

    fn claim_schedule(
        &self,
        project: &str,
        binding: &Binding,
        next: i64,
    ) -> Result<Option<Attempt>> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let body: Option<String> = tx
            .query_row(
                "SELECT body FROM schedules WHERE workflow_id=?1",
                [&binding.workflow_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(body) = body else { return Ok(None) };
        let mut current: Binding = serde_json::from_str(&body)?;
        if current.generation != binding.generation
            || current.next_at != binding.next_at
            || current.config["enabled"] != true
        {
            return Ok(None);
        }
        let draft = load_draft(&tx, &binding.workflow_id)?;
        let attempt = Attempt {
            id: format!("{}:{}", binding.generation, binding.next_at),
            workflow_id: binding.workflow_id.clone(),
            workflow_title: draft.document["name"]
                .as_str()
                .unwrap_or(&binding.workflow_id)
                .into(),
            due_at: binding.next_at,
            status: "starting".into(),
            reason: None,
            run_id: None,
        };
        current.next_at = next;
        tx.execute(
            "UPDATE schedules SET body=?1 WHERE workflow_id=?2",
            params![serde_json::to_string(&current)?, binding.workflow_id],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO schedule_attempts(id,workflow_id,body) VALUES(?1,?2,?3)",
            params![
                attempt.id,
                attempt.workflow_id,
                serde_json::to_string(&attempt)?
            ],
        )?;
        tx.commit()?;
        Ok((inserted > 0).then_some(attempt))
    }

    fn finish_attempt(&self, project: &str, attempt: &Attempt) -> Result<()> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE schedule_attempts SET body=?1 WHERE id=?2",
            params![serde_json::to_string(attempt)?, attempt.id],
        )?;
        event(
            &tx,
            project,
            &attempt.workflow_id,
            attempt.run_id.as_deref(),
            "schedule.triggered",
            &serde_json::to_value(attempt)?,
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn schedule_history(&self, project: &str) -> Result<Vec<Attempt>> {
        let conn = self.project_db(project)?;
        let mut query = conn.prepare("SELECT body FROM schedule_attempts")?;
        let rows = query
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut result = rows
            .into_iter()
            .map(|body| Ok(serde_json::from_str::<Attempt>(&body)?))
            .collect::<Result<Vec<_>>>()?;
        result.sort_by_key(|row| std::cmp::Reverse(row.due_at));
        Ok(result)
    }
}

impl<H: Host> Service<H> {
    pub(crate) async fn schedule_dispatch(&self, operation: &str, args: &Value) -> Result<Value> {
        let _guard = self.schedule_guard.lock().await;
        let project = string(args, "project_id")?;
        match operation {
            "workflow.schedule.list" => return Ok(json!(self.store.schedules(project)?.iter().map(|b| json!({"workflow_id":b.workflow_id,"config":b.config,"next_at": if b.config["enabled"] == true { Some(b.next_at) } else { None }})).collect::<Vec<_>>())),
            "workflow.schedule.preview" => {
                let anchor = now(); let mut after = anchor; let mut times = vec![];
                for _ in 0..3 { after = self.host.schedule_next(&args["config"],anchor,after)?; times.push(after); }
                return Ok(json!(times));
            }
            "workflow.schedule.history" => {
                let workflow = args.get("workflow_id").and_then(Value::as_str);
                let mut history = self.store.schedule_history(project)?;
                history.retain(|a| workflow.is_none_or(|w| a.workflow_id == w));
                for attempt in &mut history {
                    if let Some(run_id) = &attempt.run_id {
                        if let Ok(run) = self.store.run(project,run_id) {
                            // A refused dispatch remains a skipped occurrence, not an executed failure.
                            if attempt.status != "skipped" { attempt.status = run.status.as_str().into(); }
                        }
                    }
                }
                return Ok(json!(history));
            }
            "workflow.schedule.history_delete" => {
                // No active evidence is removed, and run/step evidence is never deleted here.
                let mut count = 0;
                for attempt in self.store.schedule_history(project)? {
                    if args.get("workflow_id").and_then(Value::as_str).is_some_and(|w| w != attempt.workflow_id) {continue}
                    if attempt.status == "starting" || attempt.run_id.as_ref().is_some_and(|id| self.store.run(project,id).is_ok_and(|r| !r.status.terminal())) {continue}
                    count += self.store.project_db(project)?.execute("DELETE FROM schedule_attempts WHERE id=?1",[attempt.id])?;
                }
                return Ok(json!(count));
            }
            _ => {}
        }
        let workflow = string(args, "workflow_id")?;
        let draft = self.store.draft(project, workflow)?;
        let old = self.store.schedule(project, workflow)?;
        if operation == "workflow.schedule.get" {
            let mut inputs = old
                .as_ref()
                .map(|b| self.host.open_schedule_inputs(&b.sealed_inputs))
                .transpose()?
                .unwrap_or(json!({}));
            let mut sensitive: Vec<String> = vec![];
            if let Some(binding) = &old {
                for name in &binding.sensitive_inputs {
                    if inputs
                        .as_object_mut()
                        .and_then(|m| m.remove(name))
                        .is_some()
                    {
                        sensitive.push(name.clone())
                    }
                }
            }
            for spec in draft.document["inputs"].as_array().into_iter().flatten() {
                if spec["sensitive"] == true {
                    if let Some(name) = spec["name"].as_str() {
                        if inputs
                            .as_object_mut()
                            .and_then(|m| m.remove(name))
                            .is_some()
                        {
                            sensitive.push(name.to_owned())
                        }
                    }
                }
            }
            let names: Vec<&str> = draft.document["inputs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["name"].as_str())
                .collect();
            if let Some(map) = inputs.as_object_mut() {
                map.retain(|name, _| names.contains(&name.as_str()));
            }
            sensitive.retain(|name| names.contains(&name.as_str()));
            return Ok(
                json!({"config":old.as_ref().map(|b| &b.config),"inputs":inputs,"sensitive_inputs":sensitive,"eligible":true,"revision":draft.revision,"next_at":old.as_ref().filter(|b| b.config["enabled"] == true).map(|b| b.next_at)}),
            );
        }
        let revision = args["revision"]
            .as_u64()
            .ok_or_else(|| ApiError::new("invalid_params", "revision is required"))?;
        if draft.revision != revision {
            return Err(ApiError::new(
                "revision_conflict",
                "Reload the changed workflow",
            ));
        }
        if operation == "workflow.schedule.remove" {
            return Ok(json!({"draft":self.store.save_schedule(project,workflow,revision,None)?}));
        }
        if operation != "workflow.schedule.set" {
            return Err(ApiError::new("unknown_operation", operation));
        }
        let config = &args["config"];
        if !config["enabled"].is_boolean() {
            return Err(ApiError::new(
                "invalid_params",
                "config.enabled is required",
            ));
        }
        let now = now();
        let unchanged_clock = old.as_ref().filter(|b| {
            b.config["enabled"] == true
                && config["enabled"] == true
                && b.config["cron"] == config["cron"]
                && b.config["timezone"] == config["timezone"]
                && b.config["interval_minutes"] == config["interval_minutes"]
        });
        let anchor = unchanged_clock.map(|b| b.anchor_at).unwrap_or(now);
        let next = self.host.schedule_next(config, anchor, now)?;
        let mut inputs = args.get("inputs").cloned().unwrap_or(json!({}));
        if !inputs.is_object() {
            return Err(ApiError::new("invalid_params", "inputs must be an object"));
        }
        let preserved: Vec<String> =
            serde_json::from_value(args.get("preserve_sensitive").cloned().unwrap_or(json!([])))?;
        if !preserved.is_empty() {
            let previous = old
                .as_ref()
                .map(|b| self.host.open_schedule_inputs(&b.sealed_inputs))
                .transpose()?
                .unwrap_or(json!({}));
            for name in preserved {
                let sensitive = draft.document["inputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["name"] == name && s["sensitive"] == true);
                if !sensitive || previous.get(&name).is_none() {
                    return Err(ApiError::new(
                        "invalid_inputs",
                        "Cannot preserve an absent sensitive input",
                    ));
                }
                if inputs.get(&name).is_none() {
                    inputs[&name] = previous[&name].clone()
                }
            }
        }
        // Disabling remains possible even when the workflow has become invalid.
        if config["enabled"] == true {
            let report = self.validate(project, &draft.document).await?;
            if report["passed"] != true {
                return Err(ApiError::new(
                    "validation_failed",
                    "Fix the workflow before enabling its schedule",
                )
                .details(report));
            }
            self.host.preflight(&draft.document, &inputs).await?;
        }
        let sensitive_inputs = draft.document["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["sensitive"] == true)
            .filter_map(|s| s["name"].as_str().map(str::to_owned))
            .collect();
        let binding = Binding {
            workflow_id: workflow.into(),
            config: config.clone(),
            generation: uuid::Uuid::new_v4().to_string(),
            anchor_at: anchor,
            next_at: unchanged_clock
                .filter(|b| b.next_at > now)
                .map(|b| b.next_at)
                .unwrap_or(next),
            sealed_inputs: self.host.seal_schedule_inputs(&inputs)?,
            sensitive_inputs,
        };
        Ok(json!({"draft":self.store.save_schedule(project,workflow,revision,Some(&binding))?}))
    }

    /// On host startup (and project reconnection), never execute missed occurrences.
    pub async fn recover_schedules(&self, project: &str, time: i64) -> Result<()> {
        let _guard = self.schedule_guard.lock().await;
        for mut binding in self.store.schedules(project)? {
            if binding.next_at <= time {
                binding.next_at =
                    self.host
                        .schedule_next(&binding.config, binding.anchor_at, time)?;
                self.store.project_db(project)?.execute(
                    "UPDATE schedules SET body=?1 WHERE workflow_id=?2",
                    params![serde_json::to_string(&binding)?, binding.workflow_id],
                )?;
            }
        }
        for mut attempt in self.store.schedule_history(project)? {
            if attempt.status == "starting" {
                attempt.status = "interrupted".into();
                attempt.reason = Some("host_restarted".into());
                self.store.finish_attempt(project, &attempt)?
            }
        }
        Ok(())
    }

    pub async fn tick_schedules(&self, project: &str, time: i64) -> Result<()> {
        let _guard = self.schedule_guard.lock().await;
        for binding in self.store.schedules(project)? {
            if binding.config["enabled"] != true || binding.next_at > time {
                continue;
            }
            let next = self
                .host
                .schedule_next(&binding.config, binding.anchor_at, time)?;
            let Some(mut attempt) = self.store.claim_schedule(project, &binding, next)? else {
                continue;
            };
            if time - binding.next_at > 30_000 {
                attempt.status = "skipped".into();
                attempt.reason = Some("missed_while_unavailable".into());
            } else if self
                .store
                .runs(project)?
                .iter()
                .any(|r| r.workflow_id == binding.workflow_id && !r.status.terminal())
            {
                attempt.status = "skipped".into();
                attempt.reason = Some("previous_run_active".into());
            } else {
                match self.start_scheduled(project, &binding, &attempt.id).await {
                    Ok(run_id) => {
                        attempt.status = "running".into();
                        attempt.run_id = Some(run_id)
                    }
                    Err(error) => {
                        attempt.status = if error.code == "automation_busy" {
                            "skipped"
                        } else {
                            "failed"
                        }
                        .into();
                        // Detailed native evidence stays on the run. Avoid echoing fixed secret values.
                        attempt.reason = Some(error.code);
                        attempt.run_id = error
                            .details
                            .as_ref()
                            .and_then(|d| d["run_id"].as_str())
                            .map(str::to_owned);
                    }
                }
            }
            self.store.finish_attempt(project, &attempt)?;
        }
        Ok(())
    }

    async fn start_scheduled(
        &self,
        project: &str,
        binding: &Binding,
        request: &str,
    ) -> Result<String> {
        let draft = self.store.draft(project, &binding.workflow_id)?;
        let definitions = self.definitions(project, &draft.document)?;
        let report = self.host.validate(&draft.document, &definitions).await?;
        if report["passed"] != true {
            return Err(ApiError::new(
                "validation_failed",
                "Scheduled workflow is invalid",
            ));
        }
        let inputs = self.host.open_schedule_inputs(&binding.sealed_inputs)?;
        // A later edit must not silently turn a stored secret into logged plain input.
        for name in &binding.sensitive_inputs {
            if inputs.get(name).is_some()
                && !draft.document["inputs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|spec| spec["name"] == *name && spec["sensitive"] == true)
            {
                return Err(ApiError::new(
                    "invalid_inputs",
                    "Sensitive input definitions changed; reconfigure the schedule inputs",
                ));
            }
        }
        let mut redacted = self.host.preflight(&draft.document, &inputs).await?;
        for name in &binding.sensitive_inputs {
            if redacted.get(name).is_some() {
                redacted[name] = json!("[REDACTED]")
            }
        }
        let version = self
            .store
            .publish_validated(project, &draft.workflow_id, draft.revision)?;
        let (run, created) = self.store.register_run(
            project,
            "schedule",
            request,
            request,
            &version,
            redacted,
            json!(definitions),
        )?;
        if created {
            if let Err(error) = self
                .host
                .start(self.store.clone(), run.clone(), inputs)
                .await
            {
                let status = if error.code == "automation_busy" {
                    crate::RunStatus::Skipped
                } else {
                    crate::RunStatus::Failed
                };
                self.store.transition(
                    project,
                    &run.run_id,
                    status,
                    Some(json!({"code":error.code,"message":error.message})),
                )?;
                return Err(error.details(json!({"run_id":run.run_id})));
            }
        }
        Ok(run.run_id)
    }
}
