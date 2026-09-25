use crate::{
    edit, ApiError, AuthoringMode, Draft, Event, Project, Result, Run, RunStatus, Version,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Only registry metadata lives in the app directory. Project documents and
/// evidence belong to the registered project, never to the process cwd.
#[derive(Clone)]
pub struct WorkbenchStore {
    root: PathBuf,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version > 1 {
        return Err(ApiError::new(
            "schema_too_new",
            "This project was written by a newer Workbench; update the application",
        ));
    }
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
    Ok(conn)
}

fn decode<T: DeserializeOwned>(text: String) -> Result<T> {
    Ok(serde_json::from_str(&text)?)
}

fn missing(kind: &str) -> ApiError {
    ApiError::new("not_found", format!("{kind} not found"))
}

fn conflict(actual: u64) -> ApiError {
    ApiError::new(
        "revision_conflict",
        "The draft changed; reload and review your changes before applying them",
    )
    .details(json!({"current_revision": actual}))
}

fn event(
    conn: &Connection,
    project: &str,
    workflow: &str,
    run: Option<&str>,
    kind: &str,
    data: &Value,
) -> Result<()> {
    conn.execute("INSERT INTO events(project_id,workflow_id,run_id,kind,data,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
        params![project, workflow, run, kind, data.to_string(), now()])?;
    Ok(())
}

impl WorkbenchStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        let store = Self { root };
        let conn = store.registry()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS projects(
            project_id TEXT PRIMARY KEY, name TEXT NOT NULL, directory TEXT NOT NULL UNIQUE);
            CREATE TABLE IF NOT EXISTS clients(
            client_id TEXT PRIMARY KEY, name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE,
            projects TEXT NOT NULL, capabilities TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
            PRAGMA user_version=1;",
        )?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn registry(&self) -> Result<Connection> {
        connect(&self.root.join("registry.sqlite"))
    }

    pub fn register_project(&self, directory: &Path, name: &str) -> Result<Project> {
        if !directory.is_absolute() || !directory.is_dir() {
            return Err(ApiError::new(
                "invalid_project",
                "Select an existing absolute project directory",
            ));
        }
        if name.trim().is_empty() {
            return Err(ApiError::new("invalid_project", "Project name is required"));
        }
        let directory = directory.canonicalize()?.to_string_lossy().to_string();
        let conn = self.registry()?;
        let id = uuid::Uuid::new_v4().to_string();
        let inserted = conn.execute("INSERT INTO projects(project_id,name,directory) VALUES(?1,?2,?3) ON CONFLICT(directory) DO NOTHING",
            params![id, name.trim(), directory])?;
        let project = conn.query_row(
            "SELECT project_id,name,directory FROM projects WHERE directory=?1",
            [&directory],
            |r| {
                Ok(Project {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    directory: r.get(2)?,
                })
            },
        )?;
        if let Err(error) = self.project_db(&project.project_id) {
            if inserted != 0 {
                conn.execute(
                    "DELETE FROM projects WHERE project_id=?1",
                    [&project.project_id],
                )?;
            }
            return Err(error);
        }
        Ok(project)
    }

    pub fn projects(&self) -> Result<Vec<Project>> {
        let conn = self.registry()?;
        let mut stmt = conn
            .prepare("SELECT project_id,name,directory FROM projects ORDER BY name,project_id")?;
        let items = stmt
            .query_map([], |r| {
                Ok(Project {
                    project_id: r.get(0)?,
                    name: r.get(1)?,
                    directory: r.get(2)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(items)
    }

    pub fn project(&self, project_id: &str) -> Result<Project> {
        self.registry()?
            .query_row(
                "SELECT project_id,name,directory FROM projects WHERE project_id=?1",
                [project_id],
                |r| {
                    Ok(Project {
                        project_id: r.get(0)?,
                        name: r.get(1)?,
                        directory: r.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| missing("Project"))
    }

    pub fn project_data_dir(&self, project_id: &str) -> Result<PathBuf> {
        let project = self.project(project_id)?;
        let root = PathBuf::from(project.directory);
        if !root.is_dir() {
            return Err(ApiError::new(
                "project_unavailable",
                "Project directory is unavailable",
            ));
        }
        Ok(root.join(".nuphus-workbench"))
    }

    fn project_db(&self, project_id: &str) -> Result<Connection> {
        let root = self.project_data_dir(project_id)?;
        std::fs::create_dir_all(&root)?;
        let conn = connect(&root.join("workbench.sqlite"))?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS drafts(
            workflow_id TEXT PRIMARY KEY, revision INTEGER NOT NULL, body TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS versions(
            version_id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, revision INTEGER NOT NULL, body TEXT NOT NULL,
            UNIQUE(workflow_id,revision));
            CREATE TABLE IF NOT EXISTS runs(
            run_id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, client_id TEXT NOT NULL,
            request_id TEXT NOT NULL, fingerprint TEXT NOT NULL, body TEXT NOT NULL,
            UNIQUE(client_id,request_id));
            CREATE TABLE IF NOT EXISTS events(
            cursor INTEGER PRIMARY KEY AUTOINCREMENT, project_id TEXT NOT NULL,
            workflow_id TEXT NOT NULL, run_id TEXT, kind TEXT NOT NULL, data TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS events_run ON events(run_id,cursor);
            PRAGMA user_version=1;")?;
        Ok(conn)
    }

    pub fn create(&self, project: &str, name: &str, mode: AuthoringMode) -> Result<Draft> {
        if name.trim().is_empty() {
            return Err(ApiError::new("invalid_params", "Workflow name is required"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let draft = Draft {
            project_id: project.into(),
            workflow_id: id.clone(),
            revision: 1,
            authoring_mode: mode,
            document: json!({"id": id, "name": name.trim(), "status":"Draft", "steps":[],
            "doc":null,"schedule":null,"run_history":[],"inputs":[],"dry_run":false}),
            layout: json!({}),
            layout_revision: 0,
            updated_at: now(),
        };
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO drafts(workflow_id,revision,body) VALUES(?1,?2,?3)",
            params![id, draft.revision, serde_json::to_string(&draft)?],
        )?;
        event(
            &tx,
            project,
            &id,
            None,
            "canvas.created",
            &json!({"revision":1}),
        )?;
        tx.commit()?;
        Ok(draft)
    }

    pub fn drafts(&self, project: &str) -> Result<Vec<Draft>> {
        let conn = self.project_db(project)?;
        let mut stmt = conn.prepare("SELECT body FROM drafts ORDER BY workflow_id")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(decode).collect()
    }

    pub fn draft(&self, project: &str, id: &str) -> Result<Draft> {
        load_draft(&self.project_db(project)?, id)
    }

    /// The entire edit batch is atomic. Incomplete but structurally valid IR can
    /// be saved; semantic compiler diagnostics are supplied separately by host.
    pub fn update(
        &self,
        project: &str,
        id: &str,
        revision: u64,
        operations: &[edit::Edit],
    ) -> Result<Draft> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut draft = load_draft(&tx, id)?;
        if draft.revision != revision {
            return Err(conflict(draft.revision));
        }
        for operation in operations {
            edit::apply(&mut draft, operation)?;
        }
        edit::check_document(&draft.document, id)?;
        draft.revision += 1;
        draft.updated_at = now();
        tx.execute(
            "UPDATE drafts SET revision=?1,body=?2 WHERE workflow_id=?3",
            params![draft.revision, serde_json::to_string(&draft)?, id],
        )?;
        event(
            &tx,
            project,
            id,
            None,
            "canvas.changed",
            &json!({"revision":draft.revision}),
        )?;
        tx.commit()?;
        Ok(draft)
    }

    /// Caller must validate this exact draft revision with the native compiler.
    /// CAS below prevents publishing another document after validation.
    pub fn publish_validated(&self, project: &str, id: &str, revision: u64) -> Result<Version> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let draft = load_draft(&tx, id)?;
        if draft.revision != revision {
            return Err(conflict(draft.revision));
        }
        if let Some(body) = tx
            .query_row(
                "SELECT body FROM versions WHERE workflow_id=?1 AND revision=?2",
                params![id, revision],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return decode(body);
        }
        let version = Version {
            version_id: uuid::Uuid::new_v4().to_string(),
            workflow_id: id.into(),
            revision,
            document: draft.document,
            created_at: now(),
        };
        tx.execute(
            "INSERT INTO versions(version_id,workflow_id,revision,body) VALUES(?1,?2,?3,?4)",
            params![
                version.version_id,
                id,
                revision,
                serde_json::to_string(&version)?
            ],
        )?;
        event(
            &tx,
            project,
            id,
            None,
            "workflow.saved",
            &json!({"version_id":version.version_id,"revision":revision}),
        )?;
        tx.commit()?;
        Ok(version)
    }

    pub fn versions(&self, project: &str, id: &str) -> Result<Vec<Version>> {
        let conn = self.project_db(project)?;
        let mut stmt =
            conn.prepare("SELECT body FROM versions WHERE workflow_id=?1 ORDER BY revision DESC")?;
        let rows = stmt
            .query_map([id], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(decode).collect()
    }

    pub fn version(&self, project: &str, id: &str) -> Result<Version> {
        let body = self
            .project_db(project)?
            .query_row("SELECT body FROM versions WHERE version_id=?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?;
        decode(body.ok_or_else(|| missing("Version"))?)
    }

    pub fn update_layout(
        &self,
        project: &str,
        id: &str,
        revision: u64,
        layout: Value,
    ) -> Result<Draft> {
        if !layout.is_object() {
            return Err(ApiError::new("invalid_params", "layout must be an object"));
        }
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut draft = load_draft(&tx, id)?;
        if draft.layout_revision != revision {
            return Err(conflict(draft.layout_revision));
        }
        draft.layout = layout;
        draft.layout_revision += 1;
        tx.execute(
            "UPDATE drafts SET body=?1 WHERE workflow_id=?2",
            params![serde_json::to_string(&draft)?, id],
        )?;
        event(
            &tx,
            project,
            id,
            None,
            "canvas.layout_changed",
            &json!({"layout_revision":draft.layout_revision}),
        )?;
        tx.commit()?;
        Ok(draft)
    }

    pub fn request_run(
        &self,
        project: &str,
        client: &str,
        request: &str,
        fingerprint: &str,
    ) -> Result<Option<Run>> {
        let conn = self.project_db(project)?;
        let old: Option<(String, String)> = conn
            .query_row(
                "SELECT fingerprint,body FROM runs WHERE client_id=?1 AND request_id=?2",
                params![client, request],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match old {
            Some((previous, _)) if previous != fingerprint => Err(ApiError::new(
                "idempotency_conflict",
                "request_id was already used for a different request",
            )),
            Some((_, body)) => Ok(Some(decode(body)?)),
            None => Ok(None),
        }
    }

    /// Idempotency is scoped by authenticated caller, not by caller-supplied
    /// identity. Inputs in the record must already have sensitive values masked.
    pub fn register_run(
        &self,
        project: &str,
        client: &str,
        request_id: &str,
        fingerprint: &str,
        version: &Version,
        redacted_inputs: Value,
        snapshots: Value,
    ) -> Result<(Run, bool)> {
        if request_id.trim().is_empty() {
            return Err(ApiError::new("invalid_params", "request_id is required"));
        }
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT fingerprint,body FROM runs WHERE client_id=?1 AND request_id=?2",
                params![client, request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_fingerprint, body)) = old {
            if old_fingerprint != fingerprint {
                return Err(ApiError::new(
                    "idempotency_conflict",
                    "request_id was already used for a different request",
                ));
            }
            return Ok((decode(body)?, false));
        }
        let run = Run {
            run_id: uuid::Uuid::new_v4().to_string(),
            project_id: project.into(),
            workflow_id: version.workflow_id.clone(),
            version_id: version.version_id.clone(),
            status: RunStatus::Starting,
            inputs: redacted_inputs,
            snapshots,
            created_at: now(),
            updated_at: now(),
            result: None,
            pending_request: None,
        };
        tx.execute("INSERT INTO runs(run_id,workflow_id,client_id,request_id,fingerprint,body) VALUES(?1,?2,?3,?4,?5,?6)",
            params![run.run_id,run.workflow_id,client,request_id,fingerprint,serde_json::to_string(&run)?])?;
        event(
            &tx,
            project,
            &run.workflow_id,
            Some(&run.run_id),
            "run.starting",
            &json!({"version_id":run.version_id}),
        )?;
        tx.commit()?;
        Ok((run, true))
    }

    pub fn run(&self, project: &str, id: &str) -> Result<Run> {
        load_run(&self.project_db(project)?, id)
    }

    pub fn runs(&self, project: &str) -> Result<Vec<Run>> {
        let conn = self.project_db(project)?;
        let mut stmt = conn.prepare("SELECT body FROM runs ORDER BY rowid DESC")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter().map(decode).collect()
    }

    pub fn transition(
        &self,
        project: &str,
        id: &str,
        status: RunStatus,
        result: Option<Value>,
    ) -> Result<Run> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = load_run(&tx, id)?;
        if !run.status.allows(status) {
            return Err(ApiError::new(
                "invalid_run_state",
                format!(
                    "Cannot change {} to {}",
                    run.status.as_str(),
                    status.as_str()
                ),
            ));
        }
        if run.status == status {
            return Ok(run);
        }
        run.status = status;
        if status != RunStatus::AwaitingHuman {
            run.pending_request = None;
        }
        run.updated_at = now();
        run.result = result;
        tx.execute(
            "UPDATE runs SET body=?1 WHERE run_id=?2",
            params![serde_json::to_string(&run)?, id],
        )?;
        event(
            &tx,
            project,
            &run.workflow_id,
            Some(id),
            &format!("run.{}", status.as_str()),
            &json!({"status":status,"result":run.result}),
        )?;
        tx.commit()?;
        Ok(run)
    }

    pub fn await_human(&self, project: &str, id: &str, step_id: &str, prompt: &str) -> Result<Run> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = load_run(&tx, id)?;
        if run.status.terminal() {
            return Err(ApiError::new("invalid_run_state", "Run already finished"));
        }
        run.status = RunStatus::AwaitingHuman;
        run.updated_at = now();
        run.pending_request = Some(crate::HumanRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            step_id: step_id.into(),
            prompt: prompt.into(),
        });
        tx.execute(
            "UPDATE runs SET body=?1 WHERE run_id=?2",
            params![serde_json::to_string(&run)?, id],
        )?;
        event(
            &tx,
            project,
            &run.workflow_id,
            Some(id),
            "run.awaiting_human",
            &json!({"request":run.pending_request}),
        )?;
        tx.commit()?;
        Ok(run)
    }

    /// Claim a specific pending request once. A stale reply cannot resume a later wait.
    pub fn respond_human(
        &self,
        project: &str,
        id: &str,
        request: &str,
        decision: &str,
    ) -> Result<Run> {
        if !["continue", "cancel"].contains(&decision) {
            return Err(ApiError::new(
                "invalid_params",
                "decision must be continue or cancel",
            ));
        }
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut run = load_run(&tx, id)?;
        if run.status != RunStatus::AwaitingHuman
            || run
                .pending_request
                .as_ref()
                .is_none_or(|r| r.request_id != request)
        {
            return Err(ApiError::new(
                "stale_human_request",
                "This request is no longer awaiting a response; refresh run status",
            ));
        }
        run.pending_request = None;
        run.status = RunStatus::Running;
        run.updated_at = now();
        tx.execute(
            "UPDATE runs SET body=?1 WHERE run_id=?2",
            params![serde_json::to_string(&run)?, id],
        )?;
        event(
            &tx,
            project,
            &run.workflow_id,
            Some(id),
            "run.response",
            &json!({"request_id":request,"decision":decision}),
        )?;
        tx.commit()?;
        Ok(run)
    }

    /// Call once when the host starts, before accepting clients. Reconnecting a
    /// client must not call this: connections do not own execution lifetimes.
    pub fn recover_interrupted(&self) -> Result<usize> {
        let mut recovered = 0;
        for project in self.projects()? {
            for run in self.runs(&project.project_id)? {
                if !run.status.terminal() {
                    self.transition(
                        &project.project_id,
                        &run.run_id,
                        RunStatus::Interrupted,
                        Some(json!({"code":"host_restarted"})),
                    )?;
                    recovered += 1;
                }
            }
        }
        Ok(recovered)
    }

    pub fn append_event(
        &self,
        project: &str,
        run_id: &str,
        kind: &str,
        data: &Value,
    ) -> Result<()> {
        let conn = self.project_db(project)?;
        let run = load_run(&conn, run_id)?;
        event(&conn, project, &run.workflow_id, Some(run_id), kind, data)
    }

    pub fn events(
        &self,
        project: &str,
        run_id: Option<&str>,
        after: u64,
        limit: u32,
    ) -> Result<Vec<Event>> {
        let conn = self.project_db(project)?;
        let mut stmt=conn.prepare("SELECT cursor,workflow_id,run_id,kind,data,created_at FROM events WHERE cursor>?1 AND (?2 IS NULL OR run_id=?2) ORDER BY cursor LIMIT ?3")?;
        let rows = stmt
            .query_map(params![after, run_id, limit.clamp(1, 1000)], |r| {
                Ok((
                    r.get::<_, u64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(cursor, workflow_id, run_id, kind, data, created_at)| {
                Ok(Event {
                    cursor,
                    project_id: project.into(),
                    workflow_id,
                    run_id,
                    kind,
                    data: decode(data)?,
                    created_at,
                })
            })
            .collect()
    }

    pub fn delete(&self, project: &str, id: &str, revision: u64) -> Result<()> {
        let mut conn = self.project_db(project)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let draft = load_draft(&tx, id)?;
        if draft.revision != revision {
            return Err(conflict(draft.revision));
        }
        // Immutable versions and run evidence remain available after deletion.
        tx.execute("DELETE FROM drafts WHERE workflow_id=?1", [id])?;
        event(
            &tx,
            project,
            id,
            None,
            "canvas.deleted",
            &json!({"revision":revision}),
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub fn fingerprint(value: &Value) -> String {
    let mut canonical = value.clone();
    canonical.sort_all_objects();
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

fn load_draft(conn: &Connection, id: &str) -> Result<Draft> {
    let body = conn
        .query_row("SELECT body FROM drafts WHERE workflow_id=?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    decode(body.ok_or_else(|| missing("Draft"))?)
}

fn load_run(conn: &Connection, id: &str) -> Result<Run> {
    let body = conn
        .query_row("SELECT body FROM runs WHERE run_id=?1", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?;
    decode(body.ok_or_else(|| missing("Run"))?)
}
