use crate::{
    auth::Principal,
    service::{Host, Service},
    *,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[derive(Default)]
struct FakeHost {
    starts: AtomicUsize,
}

#[async_trait]
impl Host for FakeHost {
    async fn validate(&self, document: &Value, _: &[Value]) -> Result<Value> {
        Ok(json!({"passed":document["steps"].as_array().is_some_and(|s|!s.is_empty())}))
    }
    async fn preflight(&self, _: &Value, inputs: &Value) -> Result<Value> {
        Ok(inputs.clone())
    }
    async fn start(&self, store: WorkbenchStore, run: Run, _: Value) -> Result<()> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        store.transition(&run.project_id, &run.run_id, RunStatus::Running, None)?;
        Ok(())
    }
    async fn control(&self, _: &WorkbenchStore, _: &Run, _: &str, _: &Value) -> Result<Value> {
        Ok(json!({}))
    }
    async fn automation(&self, _: &Project, _: &str, _: &Value) -> Result<Value> {
        Ok(json!({}))
    }
    async fn view(&self, _: &str, _: &str, _: &str, _: &Value) -> Result<Value> {
        Ok(json!({}))
    }
    fn capabilities(&self) -> Value {
        json!({"test":true})
    }
}

fn fixture() -> (tempfile::TempDir, Arc<Service<FakeHost>>, Project, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkbenchStore::open(dir.path().join("app")).unwrap();
    let project = store.register_project(dir.path(), "Test").unwrap();
    let (_, token) = store
        .create_client(
            "Test",
            vec![project.project_id.clone()],
            vec!["read".into(), "edit".into(), "run".into()],
        )
        .unwrap();
    (
        dir,
        Arc::new(Service::new(store, FakeHost::default())),
        project,
        token,
    )
}

#[tokio::test]
async fn authorization_precedes_any_project_access_or_mutation() {
    let (_dir, service, project, token) = fixture();
    let client = service.store.authenticate(&token).unwrap();
    assert_eq!(
        service
            .dispatch(
                &client,
                "canvas.create",
                json!({"project_id":"different","name":"No"})
            )
            .await
            .unwrap_err()
            .code,
        "permission_denied"
    );
    assert_eq!(
        service
            .dispatch(&client, "canvas.get", json!({"workflow_id":"missing"}))
            .await
            .unwrap_err()
            .code,
        "invalid_params"
    );
    assert!(service
        .store
        .drafts(&project.project_id)
        .unwrap()
        .is_empty());
    let id = client.id().to_owned();
    service.store.revoke_client(&id).unwrap();
    assert_eq!(
        service.store.authenticate(&token).err().unwrap().code,
        "unauthorized"
    );
    let clients = service.store.clients().unwrap();
    assert!(!serde_json::to_string(&clients).unwrap().contains(&token));
}

#[tokio::test]
async fn authoring_modes_and_invalid_import_do_not_leave_orphans() {
    let (_dir, service, p, token) = fixture();
    let client = service.store.authenticate(&token).unwrap();
    let args = json!({"project_id":p.project_id,"name":"New"});
    assert!(service
        .dispatch(&client, "workflow.import", args.clone())
        .await
        .is_err());
    assert!(service.store.drafts(&p.project_id).unwrap().is_empty());
    let external = service
        .dispatch(&client, "canvas.create", args.clone())
        .await
        .unwrap();
    let internal = service
        .dispatch(&Principal::LocalUi, "canvas.create", args)
        .await
        .unwrap();
    assert_eq!(external["authoring_mode"], "external");
    assert_eq!(internal["authoring_mode"], "internal");
    let listed = service
        .dispatch(&client, "project.list", json!({}))
        .await
        .unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn published_run_retries_do_not_start_another_execution() {
    let (_dir, service, p, token) = fixture();
    let client = service.store.authenticate(&token).unwrap();
    let draft = service
        .dispatch(
            &client,
            "canvas.create",
            json!({"project_id":p.project_id,"name":"Run"}),
        )
        .await
        .unwrap();
    let id = draft["workflow_id"].as_str().unwrap();
    assert_eq!(
        service
            .dispatch(
                &client,
                "workflow.save",
                json!({"project_id":p.project_id,"workflow_id":id,"revision":1})
            )
            .await
            .unwrap_err()
            .code,
        "validation_failed"
    );
    service.dispatch(&client,"canvas.update",json!({"project_id":p.project_id,"workflow_id":id,"revision":1,"operations":[{"op":"add_step","parent_id":null,"lane":"main","index":0,"step":{"id":"wait","name":"Wait","do":{"sleep":0.01}}}]})).await.unwrap();
    let version = service
        .dispatch(
            &client,
            "workflow.save",
            json!({"project_id":p.project_id,"workflow_id":id,"revision":2}),
        )
        .await
        .unwrap();
    let args = json!({"project_id":p.project_id,"version_id":version["version_id"],"request_id":"once","inputs":{"a":1,"b":2}});
    let first = service
        .dispatch(&client, "workflow.run", args.clone())
        .await
        .unwrap();
    let second = service
        .dispatch(&client, "workflow.run", args.clone())
        .await
        .unwrap();
    assert_eq!(first["run_id"], second["run_id"]);
    assert_eq!(second["created"], false);
    assert_eq!(service.host.starts.load(Ordering::SeqCst), 1);
    let mut changed = args;
    changed["inputs"]["a"] = json!(2);
    assert_eq!(
        service
            .dispatch(&client, "workflow.run", changed)
            .await
            .unwrap_err()
            .code,
        "idempotency_conflict"
    );
    let events = service
        .store
        .events(&p.project_id, first["run_id"].as_str(), 0, 100)
        .unwrap();
    assert!(events.iter().any(|e| e.kind == "run.running"));
}

#[test]
fn layout_changes_do_not_invalidate_content_revision() {
    let (_dir, service, p, _) = fixture();
    let draft = service
        .store
        .create(&p.project_id, "Layout", AuthoringMode::External)
        .unwrap();
    let updated = service
        .store
        .update_layout(
            &p.project_id,
            &draft.workflow_id,
            0,
            json!({"nodes":{"a":{"x":10,"y":20}}}),
        )
        .unwrap();
    assert_eq!(updated.revision, draft.revision);
    assert_eq!(updated.layout_revision, 1);
    assert_eq!(
        service
            .store
            .update_layout(&p.project_id, &draft.workflow_id, 0, json!({}))
            .unwrap_err()
            .code,
        "revision_conflict"
    );
    let changed = service
        .store
        .update(
            &p.project_id,
            &draft.workflow_id,
            1,
            &[edit::Edit::Rename {
                name: "Still editable".into(),
            }],
        )
        .unwrap();
    assert_eq!(changed.layout, updated.layout);
}

#[test]
fn loop_children_follow_the_native_ir_do_field() {
    let (_dir, service, p, _) = fixture();
    let draft = service
        .store
        .create(&p.project_id, "Loop", AuthoringMode::External)
        .unwrap();
    let mut document = draft.document;
    document["steps"] = json!([{"id":"loop","name":"Loop","do":{"loop":{"count":2,"do":[{"id":"wait","name":"Wait","do":{"sleep":1}}]}}}]);
    let changed = service
        .store
        .update(
            &p.project_id,
            &draft.workflow_id,
            1,
            &[
                edit::Edit::ReplaceDocument { document },
                edit::Edit::RemoveStep {
                    step_id: "wait".into(),
                },
            ],
        )
        .unwrap();
    assert_eq!(
        changed.document.pointer("/steps/0/do/loop/do"),
        Some(&json!([]))
    );
}

#[test]
fn refuses_future_schema_without_downgrading_it() {
    let (_dir, service, _, _) = fixture();
    let conn = rusqlite::Connection::open(service.store.root().join("registry.sqlite")).unwrap();
    conn.execute_batch("PRAGMA user_version=999").unwrap();
    assert_eq!(
        WorkbenchStore::open(service.store.root())
            .err()
            .unwrap()
            .code,
        "schema_too_new"
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        999
    );
}

#[test]
fn stale_human_replies_cannot_resume_the_next_wait() {
    let (_dir, service, p, _) = fixture();
    let draft = service
        .store
        .create(&p.project_id, "Wait", AuthoringMode::External)
        .unwrap();
    let version = service
        .store
        .publish_validated(&p.project_id, &draft.workflow_id, 1)
        .unwrap();
    let (run, _) = service
        .store
        .register_run(
            &p.project_id,
            "test",
            "once",
            "digest",
            &version,
            json!({}),
            json!([]),
        )
        .unwrap();
    service
        .store
        .transition(&p.project_id, &run.run_id, RunStatus::Running, None)
        .unwrap();
    let waiting = service
        .store
        .await_human(&p.project_id, &run.run_id, "step", "Review output")
        .unwrap();
    let request = waiting.pending_request.unwrap().request_id;
    service
        .store
        .respond_human(&p.project_id, &run.run_id, &request, "continue")
        .unwrap();
    let next = service
        .store
        .await_human(&p.project_id, &run.run_id, "step", "Second loop iteration")
        .unwrap();
    assert_ne!(next.pending_request.unwrap().request_id, request);
    assert_eq!(
        service
            .store
            .respond_human(&p.project_id, &run.run_id, &request, "continue")
            .unwrap_err()
            .code,
        "stale_human_request"
    );
}

#[cfg(feature = "gateway")]
async fn rpc(
    http: &reqwest::Client,
    url: &str,
    token: &str,
    session: Option<&str>,
    body: Value,
) -> (reqwest::StatusCode, Option<String>, Value) {
    let mut request = http
        .post(url)
        .bearer_auth(token)
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25")
        .json(&body);
    if let Some(session) = session {
        request = request.header("mcp-session-id", session);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let session = response
        .headers()
        .get("mcp-session-id")
        .map(|h| h.to_str().unwrap().to_owned());
    let body = response.text().await.unwrap();
    let parsed = serde_json::from_str(&body).unwrap_or_else(|_| {
        body.lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .find_map(|v| serde_json::from_str(v).ok())
            .unwrap_or(Value::Null)
    });
    (status, session, parsed)
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn mcp_discovery_and_calls_use_per_request_auth_and_session_ownership() {
    let (_dir, service, p, token) = fixture();
    let listener = crate::gateway::bind(0).await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let app = crate::gateway::router(service.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let (status,session,body)=rpc(&http,&url,&token,None,json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).await;
    assert!(status.is_success(), "{body}");
    assert_eq!(body["result"]["protocolVersion"], "2025-11-25");
    let session = session.unwrap();
    rpc(
        &http,
        &url,
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let (status, _, body) = rpc(
        &http,
        &url,
        &token,
        Some(&session),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await;
    assert!(status.is_success(), "{body}");
    assert!(body["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "canvas_create"));
    let (_,_,body)=rpc(&http,&url,&token,Some(&session),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"canvas_create","arguments":{"project_id":p.project_id,"name":"MCP"}}})).await;
    assert_eq!(
        body["result"]["structuredContent"]["result"]["authoring_mode"], "external",
        "{body}"
    );
    let (_, other) = service
        .store
        .create_client("Other", vec![p.project_id.clone()], vec!["read".into()])
        .unwrap();
    assert_eq!(
        rpc(
            &http,
            &url,
            &other,
            Some(&session),
            json!({"jsonrpc":"2.0","id":4,"method":"tools/list"})
        )
        .await
        .0,
        403
    );
    let principal = service.store.authenticate(&token).unwrap();
    service.store.revoke_client(principal.id()).unwrap();
    assert_eq!(
        rpc(
            &http,
            &url,
            &token,
            Some(&session),
            json!({"jsonrpc":"2.0","id":5,"method":"tools/list"})
        )
        .await
        .0,
        401
    );
    task.abort();
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn loopback_http_enforces_auth_origin_scope_and_revocation() {
    let (_dir, service, p, token) = fixture();
    let listener = crate::gateway::bind(0).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = crate::gateway::router(service.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let path = format!("{base}/api/v1/canvas.create");
    let args = json!({"project_id":p.project_id,"name":"HTTP"});
    assert_eq!(
        http.post(&path).json(&args).send().await.unwrap().status(),
        401
    );
    assert_eq!(
        http.post(&path)
            .bearer_auth(&token)
            .header("origin", "https://unrelated.example")
            .json(&args)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        http.post(&path)
            .bearer_auth(&token)
            .header("host", "unrelated.example")
            .json(&args)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let response: Value = http
        .post(&path)
        .bearer_auth(&token)
        .json(&args)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["result"]["authoring_mode"], "external");
    let catalog: Value = http
        .get(format!("{base}/api/v1/discover"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!catalog["operations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|o| o["name"] == "project.register"));
    let client = service.store.authenticate(&token).unwrap();
    service.store.revoke_client(client.id()).unwrap();
    assert_eq!(
        http.post(&path)
            .bearer_auth(&token)
            .json(&args)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 1);
    task.abort();
}
