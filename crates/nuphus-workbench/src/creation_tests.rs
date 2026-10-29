//! Creation retry and iterative authoring regressions, using the service host fixture.
use super::*;

#[tokio::test]
async fn creation_retries_return_current_draft_and_never_resurrect_deleted_workflows() {
    let (dir, service, p, _) = fixture();
    let args = json!({"project_id":p.project_id,"name":" Task ","request_id":"create-once"});
    let first = service
        .dispatch(&Principal::LocalExternal, "canvas.create", args.clone())
        .await
        .unwrap();
    assert_eq!(first["created"], true);
    let id = first["workflow_id"].as_str().unwrap();
    service
        .store
        .update(
            &p.project_id,
            id,
            1,
            &[edit::Edit::Rename {
                name: "User edit".into(),
            }],
        )
        .unwrap();
    let restarted = Service::new(
        WorkbenchStore::open(dir.path().join("app")).unwrap(),
        FakeHost::default(),
    );
    let mut retry_args = args.clone();
    retry_args["name"] = json!("Task");
    retry_args["authoring_mode"] = json!("external");
    let retry = restarted
        .dispatch(&Principal::LocalExternal, "canvas.create", retry_args)
        .await
        .unwrap();
    assert_eq!(retry["workflow_id"], first["workflow_id"]);
    assert_eq!(retry["created"], false);
    assert_eq!(retry["revision"], 2);
    assert_eq!(retry["document"]["name"], "User edit");
    assert_eq!(
        service
            .store
            .events(&p.project_id, None, 0, 100)
            .unwrap()
            .len(),
        2
    );
    for (key, value) in [
        ("name", json!("Other")),
        ("authoring_mode", json!("internal")),
    ] {
        let mut conflict = args.clone();
        conflict[key] = value;
        assert_eq!(
            restarted
                .dispatch(&Principal::LocalExternal, "canvas.create", conflict)
                .await
                .unwrap_err()
                .code,
            "idempotency_conflict"
        );
    }
    let mut conflict = args.clone();
    conflict["document"] = json!({"name":"Task","steps":[]});
    assert_eq!(
        restarted
            .dispatch(&Principal::LocalExternal, "workflow.import", conflict)
            .await
            .unwrap_err()
            .code,
        "idempotency_conflict"
    );
    service.store.delete(&p.project_id, id, 2).unwrap();
    let error = restarted
        .dispatch(&Principal::LocalExternal, "canvas.create", args)
        .await
        .unwrap_err();
    assert_eq!(error.code, "not_found");
    assert_eq!(error.details.unwrap()["workflow_id"], id);
    assert!(service.store.drafts(&p.project_id).unwrap().is_empty());
}

#[tokio::test]
async fn imports_are_atomic_and_retries_ignore_only_rewritten_fields_and_key_order() {
    let (_dir, service, p, _) = fixture();
    let mut args = json!({"project_id":p.project_id,"name":"Import","request_id":"import-once","document":{"name":"Import","steps":[{"id":"a","name":"Wait","do":{"sleep":0.01}},{"id":"a","name":"Duplicate","do":{"sleep":0.01}}]}});
    assert_eq!(
        service
            .dispatch(&Principal::LocalExternal, "workflow.import", args.clone())
            .await
            .unwrap_err()
            .code,
        "invalid_edit"
    );
    let conn = service.store.project_db(&p.project_id).unwrap();
    for table in ["drafts", "creation_requests", "events"] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    args["document"] = serde_json::from_str(r#"{"name":"Import","steps":[{"id":"a","name":"Wait","do":{"sleep":0.01}}],"schedule":{"enabled":true},"id":"source","run_history":[{"old":true}]}"#).unwrap();
    let first = service
        .dispatch(&Principal::LocalExternal, "workflow.import", args.clone())
        .await
        .unwrap();
    assert_eq!(first["created"], true);
    assert_eq!(first["revision"], 2);
    assert_eq!(first["document"]["id"], first["workflow_id"]);
    assert_eq!(first["document"]["run_history"], json!([]));
    assert!(first["document"]["schedule"].is_null());
    args["document"] = serde_json::from_str(r#"{"run_history":[],"id":"another-source","schedule":{"enabled":true},"steps":[{"do":{"sleep":0.01},"name":"Wait","id":"a"}],"name":"Import"}"#).unwrap();
    let retry = service
        .dispatch(&Principal::LocalExternal, "workflow.import", args.clone())
        .await
        .unwrap();
    assert_eq!(retry["created"], false);
    assert_eq!(retry["workflow_id"], first["workflow_id"]);
    assert_eq!(retry["revision"], 2);
    assert_eq!(
        service
            .store
            .events(&p.project_id, None, 0, 100)
            .unwrap()
            .len(),
        2
    );
    args["document"]["steps"][0]["do"]["sleep"] = json!(1);
    assert_eq!(
        service
            .dispatch(&Principal::LocalExternal, "workflow.import", args)
            .await
            .unwrap_err()
            .code,
        "idempotency_conflict"
    );
}

#[tokio::test]
async fn creation_keys_are_optional_validated_and_scoped_to_project_and_caller() {
    let (dir, service, p, token) = fixture();
    let base = json!({"project_id":p.project_id,"name":"Same name"});
    for operation in ["canvas.create", "workflow.import"] {
        for invalid in [
            Value::Null,
            json!(7),
            json!(true),
            json!({}),
            json!([]),
            json!(""),
            json!(" \n\t"),
        ] {
            let mut args = base.clone();
            args["document"] = json!({"name":"Same name","steps":[]});
            args["request_id"] = invalid;
            assert_eq!(
                service
                    .dispatch(&Principal::LocalExternal, operation, args)
                    .await
                    .unwrap_err()
                    .code,
                "invalid_params"
            );
        }
    }
    assert!(service.store.drafts(&p.project_id).unwrap().is_empty());
    // Older callers can still deliberately create two workflows with the same name.
    for _ in 0..2 {
        assert_eq!(
            service
                .dispatch(&Principal::LocalExternal, "canvas.create", base.clone())
                .await
                .unwrap()["created"],
            true
        );
    }
    let mut args = base;
    args["request_id"] = json!("shared-key");
    let first = service
        .dispatch(&Principal::LocalExternal, "canvas.create", args.clone())
        .await
        .unwrap();
    let legacy = service.store.authenticate(&token).unwrap();
    let other_client = service
        .dispatch(&legacy, "canvas.create", args.clone())
        .await
        .unwrap();
    assert_ne!(first["workflow_id"], other_client["workflow_id"]);
    let second_path = dir.path().join("other-project");
    std::fs::create_dir(&second_path).unwrap();
    let second = service
        .store
        .register_project(&second_path, "Other")
        .unwrap();
    args["project_id"] = json!(second.project_id);
    let other_project = service
        .dispatch(&Principal::LocalExternal, "canvas.create", args.clone())
        .await
        .unwrap();
    assert_ne!(first["workflow_id"], other_project["workflow_id"]);
    args["project_id"] = json!(p.project_id);
    args["request_id"] = json!("new-key");
    let separate = service
        .dispatch(&Principal::LocalExternal, "canvas.create", args)
        .await
        .unwrap();
    assert_ne!(first["workflow_id"], separate["workflow_id"]);
    assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 5);
}

#[test]
fn concurrent_creation_and_import_retries_across_store_handles_commit_once() {
    for import in [false, true] {
        let (dir, service, p, _) = fixture();
        let barrier = Arc::new(std::sync::Barrier::new(4));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let store = WorkbenchStore::open(dir.path().join("app")).unwrap();
                let project = p.project_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .create_requested(
                            &project,
                            "Concurrent",
                            AuthoringMode::External,
                            import.then(|| json!({"name":"Concurrent","steps":[]})),
                            Some(("local-external", "same-request")),
                        )
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
        assert!(results
            .iter()
            .all(|(draft, _)| draft.workflow_id == results[0].0.workflow_id));
        assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 1);
        assert_eq!(
            service
                .store
                .events(&p.project_id, None, 0, 100)
                .unwrap()
                .len(),
            if import { 2 } else { 1 }
        );
    }
}

#[test]
fn additive_creation_table_preserves_v2_and_v3_history() {
    for version in [2, 3] {
        let (dir, service, p, _) = fixture();
        let draft = service
            .store
            .create(&p.project_id, "History", AuthoringMode::External)
            .unwrap();
        let conn = service.store.project_db(&p.project_id).unwrap();
        conn.execute_batch("DROP TABLE creation_requests").unwrap();
        if version == 3 {
            conn.execute(
                "INSERT INTO metadata(key,value) VALUES('tenant_id','historical-account')",
                [],
            )
            .unwrap();
        }
        conn.pragma_update(None, "user_version", version).unwrap();
        drop(conn);
        let store = WorkbenchStore::open(dir.path().join("app")).unwrap();
        assert_eq!(
            serde_json::to_value(store.draft(&p.project_id, &draft.workflow_id).unwrap()).unwrap(),
            serde_json::to_value(&draft).unwrap()
        );
        let conn = store.project_db(&p.project_id).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            version
        );
        assert_eq!(store.events(&p.project_id, None, 0, 100).unwrap().len(), 1);
        store
            .create_requested(
                &p.project_id,
                "New",
                AuthoringMode::External,
                None,
                Some(("local-external", "new")),
            )
            .unwrap();
        assert_eq!(store.drafts(&p.project_id).unwrap().len(), 2);
    }
}

#[tokio::test]
async fn three_edit_debug_cycles_keep_one_draft_and_distinct_retry_safe_runs() {
    let (_dir, service, p, _) = fixture();
    let creation = json!({"project_id":p.project_id,"name":"Iterate","request_id":"task-creation"});
    let initial = service
        .dispatch(&Principal::LocalExternal, "canvas.create", creation.clone())
        .await
        .unwrap();
    let id = initial["workflow_id"].as_str().unwrap();
    let mut run_ids = std::collections::HashSet::new();
    for iteration in 0..3 {
        let current = service
            .dispatch(&Principal::LocalExternal, "canvas.create", creation.clone())
            .await
            .unwrap();
        assert_eq!(current["created"], false);
        let edited = service.dispatch(&Principal::LocalExternal, "canvas.update", json!({"project_id":p.project_id,"workflow_id":id,"revision":current["revision"],"operations":[{"op":"rename","name":format!("Edit {iteration}")}]})).await.unwrap();
        let before = edited["draft"].clone();
        let args = json!({"project_id":p.project_id,"workflow_id":id,"revision":before["revision"],"request_id":format!("test-{iteration}"),"mode":if iteration == 1 { "through" } else { "node" },"selected_step_id":"wait","steps":[{"id":"wait","name":"Unsaved wait","do":{"sleep":0.01}}],"input_specs":[],"variables":{"iteration":iteration}});
        let run = service
            .dispatch(&Principal::LocalExternal, "workflow.debug", args.clone())
            .await
            .unwrap();
        assert!(run_ids.insert(run["run_id"].as_str().unwrap().to_owned()));
        let retry = service
            .dispatch(&Principal::LocalExternal, "workflow.debug", args)
            .await
            .unwrap();
        assert_eq!(retry["run_id"], run["run_id"]);
        assert_eq!(retry["created"], false);
        let after = service
            .dispatch(
                &Principal::LocalExternal,
                "canvas.get",
                json!({"project_id":p.project_id,"workflow_id":id}),
            )
            .await
            .unwrap();
        assert_eq!(after, before);
        let evidence = service
            .store
            .run(&p.project_id, run["run_id"].as_str().unwrap())
            .unwrap();
        assert_eq!(evidence.snapshots[0]["steps"][0]["id"], "wait");
        service
            .store
            .transition(&p.project_id, &evidence.run_id, RunStatus::Completed, None)
            .unwrap();
    }
    assert_eq!(service.host.starts.load(Ordering::SeqCst), 3);
    assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 1);
    assert!(service
        .store
        .versions(&p.project_id, id)
        .unwrap()
        .is_empty());
    let events = service.store.events(&p.project_id, None, 0, 100).unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "canvas.created").count(),
        1
    );
    assert_eq!(
        events.iter().filter(|e| e.kind == "canvas.changed").count(),
        3
    );
}

#[cfg(feature = "gateway")]
#[tokio::test]
async fn mcp_sessions_http_and_ipc_share_durable_creation_retries() {
    let (dir, service, p, _) = fixture();
    let listener = crate::gateway::bind(0).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = crate::gateway::local_router(service.clone());
    let http_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let root = dir.path().join("app");
    let server = crate::local::Server::bind(&root).unwrap();
    let ipc_task = tokio::spawn(server.serve(service.clone()));
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap();
    let args = json!({"project_id":p.project_id,"name":"Transport retry","request_id":"transport-creation"});
    let mut original = Value::Null;
    for session_number in 0..2 {
        let url = format!("{base}/mcp");
        let (status, session, init) = rpc(&http, &url, "", None, json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}})).await;
        assert!(status.is_success(), "{init}");
        assert_eq!(init["result"]["serverInfo"]["name"], "lingque");
        assert_eq!(init["result"]["serverInfo"]["title"], "灵雀 Lingque");
        assert!(init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("workflow_debug"));
        let session = session.unwrap();
        rpc(
            &http,
            &url,
            "",
            Some(&session),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        let (_, _, discovery) = rpc(
            &http,
            &url,
            "",
            Some(&session),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        )
        .await;
        for name in ["canvas_create", "workflow_import"] {
            let tool = discovery["result"]["tools"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["name"] == name)
                .unwrap();
            assert_eq!(
                tool["inputSchema"]["properties"]["request_id"]["type"],
                "string"
            );
            assert!(!tool["inputSchema"]["required"]
                .as_array()
                .unwrap()
                .contains(&json!("request_id")));
        }
        let (_, _, body) = rpc(&http, &url, "", Some(&session), json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"canvas_create","arguments":args}})).await;
        let draft = &body["result"]["structuredContent"]["result"];
        assert_eq!(draft["created"], session_number == 0, "{body}");
        if session_number == 0 {
            original = draft["workflow_id"].clone();
        }
        assert_eq!(draft["workflow_id"], original);
    }
    let response: Value = http
        .post(format!("{base}/api/v1/canvas.create"))
        .json(&args)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["result"]["workflow_id"], original);
    assert_eq!(response["result"]["created"], false);
    let ipc = crate::local::Client::new(root.clone(), root.join("absent-host"), None);
    let retry = ipc.operation("canvas.create", args.clone()).await.unwrap();
    assert_eq!(retry["workflow_id"], original);
    assert_eq!(retry["created"], false);
    let mut conflict = args;
    conflict["name"] = json!("Changed");
    assert_eq!(
        http.post(format!("{base}/api/v1/canvas.create"))
            .json(&conflict)
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 1);
    assert_eq!(
        service
            .store
            .events(&p.project_id, None, 0, 100)
            .unwrap()
            .len(),
        1
    );
    let url = format!("{base}/mcp");
    let (_, session, _) = rpc(&http, &url, "", None, json!({"jsonrpc":"2.0","id":4,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"delete-test","version":"1"}}})).await;
    let session = session.unwrap();
    rpc(
        &http,
        &url,
        "",
        Some(&session),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    let delete_args = json!({"project_id":p.project_id,"workflow_id":original,"revision":0});
    let (_, _, stale) = rpc(&http, &url, "", Some(&session), json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"workflow_delete","arguments":delete_args}})).await;
    assert_eq!(stale["result"]["isError"], true);
    assert_eq!(service.store.drafts(&p.project_id).unwrap().len(), 1);
    let mut delete_args = delete_args;
    delete_args["revision"] = json!(1);
    let (_, _, deleted) = rpc(&http, &url, "", Some(&session), json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"workflow_delete","arguments":delete_args}})).await;
    assert_eq!(
        deleted["result"]["structuredContent"]["result"]["deleted"], true,
        "{deleted}"
    );
    assert!(service.store.drafts(&p.project_id).unwrap().is_empty());
    assert_eq!(
        service
            .store
            .events(&p.project_id, None, 0, 100)
            .unwrap()
            .last()
            .unwrap()
            .kind,
        "canvas.deleted"
    );
    ipc_task.abort();
    http_task.abort();
    let _ = ipc_task.await;
    let _ = http_task.await;
}
