use crate::{
    auth::Principal, edit::Edit, store::fingerprint, ApiError, AuthoringMode, Result, Run,
    RunStatus, WorkbenchStore, API_VERSION,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};

/// Native capabilities stay with the application. Transports never execute a
/// workflow, access model credentials or operate a desktop by themselves.
#[async_trait]
pub trait Host: Send + Sync + 'static {
    async fn validate(&self, document: &Value, definitions: &[Value]) -> Result<Value>;
    async fn preflight(&self, document: &Value, inputs: &Value) -> Result<Value>;
    async fn start(&self, store: WorkbenchStore, run: Run, inputs: Value) -> Result<()>;
    async fn control(
        &self,
        store: &WorkbenchStore,
        run: &Run,
        action: &str,
        args: &Value,
    ) -> Result<Value>;
    async fn automation(
        &self,
        project: &crate::Project,
        action: &str,
        args: &Value,
    ) -> Result<Value>;
    async fn view(
        &self,
        project: &str,
        workflow: &str,
        action: &str,
        args: &Value,
    ) -> Result<Value>;
    fn capabilities(&self) -> Value;
}

#[derive(Clone)]
pub struct Service<H: Host> {
    pub store: WorkbenchStore,
    pub host: Arc<H>,
}

pub fn string<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::new("invalid_params", format!("{key} must be a nonempty string")))
}

fn revision(args: &Value) -> Result<u64> {
    args.get("revision")
        .and_then(Value::as_u64)
        .ok_or_else(|| ApiError::new("invalid_params", "revision is required"))
}

fn encode(value: impl serde::Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

impl<H: Host> Service<H> {
    pub fn new(store: WorkbenchStore, host: H) -> Self {
        Self {
            store,
            host: Arc::new(host),
        }
    }

    pub async fn dispatch(
        &self,
        principal: &Principal,
        operation: &str,
        args: Value,
    ) -> Result<Value> {
        if !args.is_object() {
            return Err(ApiError::new(
                "invalid_params",
                "Arguments must be an object",
            ));
        }
        let capability = match operation {
            "system.capabilities"
            | "project.list"
            | "project.get"
            | "canvas.get"
            | "canvas.view_get"
            | "workflow.list"
            | "workflow.get"
            | "workflow.validate"
            | "workflow.versions"
            | "workflow.export"
            | "run.list"
            | "run.get"
            | "run.events"
            | "run.steps"
            | "automation.capabilities" => "read",
            "canvas.create"
            | "canvas.update"
            | "canvas.layout_update"
            | "canvas.open"
            | "workflow.save"
            | "workflow.import"
            | "workflow.delete" => "edit",
            "workflow.run" | "workflow.debug" | "run.pause" | "run.resume" | "run.cancel" => "run",
            "run.respond" => "respond",
            "project.register" => "projects",
            "automation.observe" | "automation.execute" => "automation",
            _ => return Err(ApiError::new("unknown_operation", operation)),
        };
        // Project-less methods are explicitly enumerated. Omitting project_id
        // must never turn a project operation into a global capability check.
        if ["system.capabilities", "project.list", "project.register"].contains(&operation) {
            principal.authorize(capability, None)?;
            return match operation {
                "system.capabilities" => {
                    Ok(json!({"api_version":API_VERSION,"host":self.host.capabilities()}))
                }
                "project.list" => encode(
                    self.store
                        .projects()?
                        .into_iter()
                        .filter(|p| principal.authorize("read", Some(&p.project_id)).is_ok())
                        .collect::<Vec<_>>(),
                ),
                "project.register" => encode(self.store.register_project(
                    Path::new(string(&args, "directory")?),
                    string(&args, "name")?,
                )?),
                _ => unreachable!(),
            };
        }
        let project = string(&args, "project_id")?;
        principal.authorize(capability, Some(project))?;
        let project_info = self.store.project(project)?;
        match operation {
            "project.get" => encode(project_info),
            "workflow.list" => encode(self.store.drafts(project)?),
            "canvas.create" | "workflow.import" => {
                let imported_document = if operation == "workflow.import" {
                    Some(
                        args.get("document")
                            .filter(|v| v.is_object())
                            .cloned()
                            .ok_or_else(|| {
                                ApiError::new("invalid_params", "document is required")
                            })?,
                    )
                } else {
                    None
                };
                let mode = args
                    .get("authoring_mode")
                    .map(|v| serde_json::from_value(v.clone()))
                    .transpose()?
                    .unwrap_or(if matches!(principal, Principal::LocalUi) {
                        AuthoringMode::Internal
                    } else {
                        AuthoringMode::External
                    });
                let draft = self.store.create(project, string(&args, "name")?, mode)?;
                if let Some(mut document) = imported_document {
                    document["id"] = json!(draft.workflow_id);
                    document["run_history"] = json!([]);
                    match self.store.update(
                        project,
                        &draft.workflow_id,
                        draft.revision,
                        &[Edit::ReplaceDocument { document }],
                    ) {
                        Ok(imported) => encode(imported),
                        Err(error) => {
                            self.store
                                .delete(project, &draft.workflow_id, draft.revision)?;
                            Err(error)
                        }
                    }
                } else {
                    encode(draft)
                }
            }
            "canvas.get" | "workflow.get" => {
                encode(self.store.draft(project, string(&args, "workflow_id")?)?)
            }
            "canvas.update" => {
                let operations: Vec<Edit> =
                    serde_json::from_value(args.get("operations").cloned().ok_or_else(|| {
                        ApiError::new("invalid_params", "operations is required")
                    })?)?;
                let draft = self.store.update(
                    project,
                    string(&args, "workflow_id")?,
                    revision(&args)?,
                    &operations,
                )?;
                // A diagnostic failure must not conceal an already committed edit.
                let diagnostics = self.validate(project, &draft.document).await.unwrap_or_else(|e|json!({"passed":false,"errors":[e.message],"diagnostics":[],"unavailable":true}));
                Ok(json!({"draft":draft,"diagnostics":diagnostics}))
            }
            "workflow.validate" => {
                let draft = self.store.draft(project, string(&args, "workflow_id")?)?;
                self.validate(project, args.get("document").unwrap_or(&draft.document))
                    .await
            }
            "canvas.layout_update" => encode(
                self.store.update_layout(
                    project,
                    string(&args, "workflow_id")?,
                    args.get("layout_revision")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| {
                            ApiError::new("invalid_params", "layout_revision is required")
                        })?,
                    args.get("layout")
                        .cloned()
                        .ok_or_else(|| ApiError::new("invalid_params", "layout is required"))?,
                )?,
            ),
            "workflow.save" => {
                let id = string(&args, "workflow_id")?;
                let draft = self.store.draft(project, id)?;
                if draft.revision != revision(&args)? {
                    return Err(
                        ApiError::new("revision_conflict", "Reload the changed draft")
                            .details(json!({"current_revision":draft.revision})),
                    );
                }
                let report = self.validate(project, &draft.document).await?;
                require_passed(&report)?;
                encode(self.store.publish_validated(project, id, draft.revision)?)
            }
            "workflow.versions" => encode(
                self.store
                    .versions(project, string(&args, "workflow_id")?)?,
            ),
            "workflow.export" => Ok(self
                .store
                .draft(project, string(&args, "workflow_id")?)?
                .document),
            "workflow.delete" => {
                self.store
                    .delete(project, string(&args, "workflow_id")?, revision(&args)?)?;
                Ok(json!({"deleted":true,"run_evidence_preserved":true}))
            }
            "workflow.run" => self.start(principal, &args).await,
            "run.list" => encode(self.store.runs(project)?),
            "run.get" => encode(self.store.run(project, string(&args, "run_id")?)?),
            "run.events" => encode(
                self.store.events(
                    project,
                    args.get("run_id").and_then(Value::as_str),
                    args.get("after").and_then(Value::as_u64).unwrap_or(0),
                    args.get("limit")
                        .and_then(Value::as_u64)
                        .unwrap_or(100)
                        .min(1000) as u32,
                )?,
            ),
            "run.pause" | "run.resume" | "run.cancel" | "run.respond" | "run.steps" => {
                let run = self.store.run(project, string(&args, "run_id")?)?;
                self.host.control(&self.store, &run, operation, &args).await
            }
            "canvas.open" | "canvas.view_get" => {
                let id = string(&args, "workflow_id")?;
                self.store.draft(project, id)?;
                self.host.view(project, id, operation, &args).await
            }
            "automation.capabilities" | "automation.observe" | "automation.execute" => {
                self.host.automation(&project_info, operation, &args).await
            }
            "workflow.debug" => Err(ApiError::new(
                "not_supported",
                "Node debugging is not yet connected to the public service",
            )),
            _ => Err(ApiError::new("unknown_operation", operation)),
        }
    }

    pub fn definitions(&self, project: &str, document: &Value) -> Result<Vec<Value>> {
        let mut definitions = vec![document.clone()];
        for draft in self.store.drafts(project)? {
            if Some(draft.workflow_id.as_str()) == document.get("id").and_then(Value::as_str) {
                continue;
            }
            if let Some(version) = self.store.versions(project, &draft.workflow_id)?.first() {
                definitions.push(version.document.clone());
            }
        }
        Ok(definitions)
    }

    pub async fn validate(&self, project: &str, document: &Value) -> Result<Value> {
        self.host
            .validate(document, &self.definitions(project, document)?)
            .await
    }

    async fn start(&self, principal: &Principal, args: &Value) -> Result<Value> {
        let project = string(args, "project_id")?;
        if let Some(run) = self.store.request_run(
            project,
            principal.id(),
            string(args, "request_id")?,
            &fingerprint(args),
        )? {
            return Ok(json!({"run_id":run.run_id,"created":false,"status":run.status}));
        }
        let version = self.store.version(project, string(args, "version_id")?)?;
        let inputs = args.get("inputs").cloned().unwrap_or_else(|| json!({}));
        if !inputs.is_object() {
            return Err(ApiError::new("invalid_params", "inputs must be an object"));
        }
        let definitions = self.definitions(project, &version.document)?;
        require_passed(&self.host.validate(&version.document, &definitions).await?)?;
        let redacted_inputs = self.host.preflight(&version.document, &inputs).await?;
        let (run, created) = self.store.register_run(
            project,
            principal.id(),
            string(args, "request_id")?,
            &fingerprint(args),
            &version,
            redacted_inputs,
            json!(definitions),
        )?;
        if created {
            if let Err(error) = self
                .host
                .start(self.store.clone(), run.clone(), inputs)
                .await
            {
                self.store.transition(
                    project,
                    &run.run_id,
                    RunStatus::Failed,
                    Some(json!({"code":error.code,"message":error.message})),
                )?;
                return Err(error.details(json!({"run_id":run.run_id})));
            }
        }
        Ok(
            json!({"run_id":run.run_id,"created":created,"status":self.store.run(project,&run.run_id)?.status}),
        )
    }
}

fn require_passed(report: &Value) -> Result<()> {
    if report.get("passed").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(ApiError::new(
            "validation_failed",
            "Fix the workflow diagnostics before running or publishing",
        )
        .details(report.clone()))
    }
}
