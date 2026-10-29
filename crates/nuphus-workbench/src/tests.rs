use crate::{
    edit::{Edit, Lane},
    *,
};
use serde_json::json;

#[test]
fn authorized_accounts_share_legacy_projects_without_rewriting_ownership() {
    let root = tempfile::tempdir().unwrap();
    let store = WorkbenchStore::open(root.path().join("registry")).unwrap();
    let project = store.register_project(root.path(), "Legacy").unwrap();
    let old = store
        .create(&project.project_id, "Preserved", AuthoringMode::External)
        .unwrap();
    let db =
        rusqlite::Connection::open(root.path().join(".nuphus-workbench/workbench.sqlite")).unwrap();
    db.execute(
        "INSERT INTO metadata(key,value) VALUES('tenant_id','tenant-a')",
        [],
    )
    .unwrap();
    db.pragma_update(None, "user_version", 3).unwrap();
    let second_dir = root.path().join("tenants/old-tenant-b/workspace");
    std::fs::create_dir_all(&second_dir).unwrap();
    let second = store.register_project(&second_dir, "Old tenant B").unwrap();
    let version = store
        .publish_validated(&project.project_id, &old.workflow_id, old.revision)
        .unwrap();
    let (run, _) = store
        .register_run(
            &project.project_id,
            "local",
            "history",
            "fingerprint",
            &version,
            json!({}),
            json!({}),
        )
        .unwrap();
    let binding = json!({"workflow_id":old.workflow_id,"config":{"kind":"interval","interval_minutes":1,"timezone":"UTC"},"generation":"legacy","anchor_at":0,"next_at":60000,"sealed_inputs":"sealed"});
    db.execute(
        "INSERT INTO schedules(workflow_id,body) VALUES(?1,?2)",
        rusqlite::params![old.workflow_id, binding.to_string()],
    )
    .unwrap();
    store
        .save_authoring_session(
            &project.project_id,
            &old.workflow_id,
            &json!({"messages":["history"]}),
        )
        .unwrap();
    drop(store);

    // Each login, including a different tenant after restart, uses the same registry.
    for _account in ["tenant-a/user-1", "tenant-a/user-2", "tenant-b/user-1"] {
        let account = WorkbenchStore::open(root.path().join("registry"))
            .unwrap()
            .with_authorization_guard(std::sync::Arc::new(|| Ok(())));
        assert_eq!(account.projects().unwrap().len(), 2);
        assert_eq!(account.project(&second.project_id).unwrap(), second);
        assert_eq!(
            account.register_project(root.path(), "Same data").unwrap(),
            project
        );
        assert_eq!(
            account
                .draft(&project.project_id, &old.workflow_id)
                .unwrap()
                .document,
            old.document
        );
        assert_eq!(
            account
                .versions(&project.project_id, &old.workflow_id)
                .unwrap()[0]
                .version_id,
            version.version_id
        );
        assert_eq!(
            account
                .authoring_session(&project.project_id, &old.workflow_id)
                .unwrap(),
            Some(json!({"messages":["history"]}))
        );
        assert!(!account
            .events(&project.project_id, None, 0, 20)
            .unwrap()
            .is_empty());
        assert!(account.unclaimed_projects().unwrap().is_empty());
        assert_eq!(
            account.runs(&project.project_id).unwrap()[0].run_id,
            run.run_id
        );
        assert_eq!(
            account.schedules(&project.project_id).unwrap()[0].generation,
            "legacy"
        );
        assert!(account.runs(&second.project_id).unwrap().is_empty());
        assert!(account.schedules(&second.project_id).unwrap().is_empty());
        assert!(account.draft(&second.project_id, &old.workflow_id).is_err());
        account.claim_project(&project.project_id).unwrap();
        assert_eq!(
            db.query_row(
                "SELECT value FROM metadata WHERE key='tenant_id'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "tenant-a"
        );
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            account
                .project_db(&second.project_id)
                .unwrap()
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
    }
}

#[test]
fn logout_blocks_shared_data_but_preserves_pinned_run_evidence() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let (_root, store, project, draft) = fixture();
    let authorized = Arc::new(AtomicBool::new(true));
    let current = authorized.clone();
    let store = store.with_authorization_guard(Arc::new(move || {
        if current.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(ApiError::new("product_auth_required", "login"))
        }
    }));
    let version = store
        .publish_validated(&project.project_id, &draft.workflow_id, draft.revision)
        .unwrap();
    let (run, _) = store
        .register_run(
            &project.project_id,
            "local",
            "logout",
            "fingerprint",
            &version,
            json!({}),
            json!({}),
        )
        .unwrap();
    let pinned = store.pinned().unwrap();
    authorized.store(false, Ordering::SeqCst);
    assert!(store.projects().is_err());
    assert!(store.project(&project.project_id).is_err());
    assert!(store
        .draft(&project.project_id, &draft.workflow_id)
        .is_err());
    assert!(store
        .register_project(std::path::Path::new(&project.directory), "Denied")
        .is_err());
    assert!(store.claim_project(&project.project_id).is_err());
    assert!(store.unclaimed_projects().is_err());
    assert!(store.pinned().is_err());
    assert!(store.runs(&project.project_id).is_err());
    assert!(store.schedules(&project.project_id).is_err());
    pinned
        .transition(&project.project_id, &run.run_id, RunStatus::Cancelled, None)
        .unwrap();
    authorized.store(true, Ordering::SeqCst);
    assert_eq!(
        store.run(&project.project_id, &run.run_id).unwrap().status,
        RunStatus::Cancelled
    );
}

fn fixture() -> (tempfile::TempDir, WorkbenchStore, Project, Draft) {
    let dir = tempfile::tempdir().unwrap();
    let store = WorkbenchStore::open(dir.path().join("app")).unwrap();
    let project = store.register_project(dir.path(), "Tests").unwrap();
    let draft = store
        .create(&project.project_id, "Example", AuthoringMode::External)
        .unwrap();
    (dir, store, project, draft)
}

fn step(id: &str) -> serde_json::Value {
    json!({"id":id,"name":id,"do":{"sleep":0.01}})
}

#[test]
fn projects_are_stable_and_relative_paths_are_rejected() {
    let (dir, store, project, _) = fixture();
    assert_eq!(
        store
            .register_project(dir.path(), "Different label")
            .unwrap(),
        project
    );
    assert_eq!(
        store
            .register_project(std::path::Path::new("."), "Bad")
            .unwrap_err()
            .code,
        "invalid_project"
    );
    assert_eq!(store.project("missing").unwrap_err().code, "not_found");
}

#[test]
fn edit_batches_rollback_and_revisions_do_not_silently_overwrite() {
    let (_dir, store, p, d) = fixture();
    let edits = [
        Edit::Rename {
            name: "Changed".into(),
        },
        Edit::RemoveStep {
            step_id: "absent".into(),
        },
    ];
    assert!(store
        .update(&p.project_id, &d.workflow_id, 1, &edits)
        .is_err());
    assert_eq!(store.draft(&p.project_id, &d.workflow_id).unwrap(), d);
    let next = store
        .update(
            &p.project_id,
            &d.workflow_id,
            1,
            &[Edit::Rename {
                name: "Changed".into(),
            }],
        )
        .unwrap();
    assert_eq!(next.revision, 2);
    assert_eq!(
        store
            .update(&p.project_id, &d.workflow_id, 1, &[])
            .unwrap_err()
            .code,
        "revision_conflict"
    );
}

#[test]
fn two_store_instances_share_atomic_compare_and_swap() {
    let (_dir, store, p, d) = fixture();
    let other = WorkbenchStore::open(store.root()).unwrap();
    store
        .update(
            &p.project_id,
            &d.workflow_id,
            1,
            &[Edit::Rename {
                name: "First".into(),
            }],
        )
        .unwrap();
    assert_eq!(
        other
            .update(
                &p.project_id,
                &d.workflow_id,
                1,
                &[Edit::Rename {
                    name: "Second".into()
                }]
            )
            .unwrap_err()
            .code,
        "revision_conflict"
    );
}

#[test]
fn incomplete_draft_can_be_saved_without_inventing_types() {
    let (_dir, store, p, d) = fixture();
    let d=store.update(&p.project_id,&d.workflow_id,1,&[Edit::AddStep {parent_id:None,lane:Lane::Main,index:0,
        step:json!({"id":"tool","name":"Incomplete tool","do":{"tool":"desktop_find_window","with":{}}})}]).unwrap();
    assert_eq!(d.document["steps"][0]["do"]["with"], json!({}));
}

#[test]
fn containers_can_be_empty_and_cannot_move_into_themselves() {
    let (_dir, store, p, d) = fixture();
    let d = store
        .update(
            &p.project_id,
            &d.workflow_id,
            1,
            &[Edit::AddStep {
                parent_id: None,
                lane: Lane::Main,
                index: 0,
                step: json!({"id":"container","name":"Sequence","do":{"seq":[step("child")]}}),
            }],
        )
        .unwrap();
    assert!(store
        .update(
            &p.project_id,
            &d.workflow_id,
            d.revision,
            &[Edit::MoveStep {
                step_id: "container".into(),
                parent_id: Some("child".into()),
                lane: Lane::Main,
                index: 0
            }]
        )
        .is_err());
    let d = store
        .update(
            &p.project_id,
            &d.workflow_id,
            d.revision,
            &[Edit::RemoveStep {
                step_id: "child".into(),
            }],
        )
        .unwrap();
    assert_eq!(d.document["steps"][0]["do"]["seq"], json!([]));
}

#[test]
fn move_and_optional_else_preserve_ir_semantics() {
    let (_dir, store, p, d) = fixture();
    let d=store.update(&p.project_id,&d.workflow_id,1,&[
        Edit::AddStep {parent_id:None,lane:Lane::Main,index:0,step:json!({"id":"if","name":"Condition","do":{"if":{"condition":true,"then":[]}}})},
        Edit::AddStep {parent_id:None,lane:Lane::Main,index:1,step:step("child")},
        Edit::MoveStep {step_id:"child".into(),parent_id:Some("if".into()),lane:Lane::Else,index:0},
    ]).unwrap();
    assert_eq!(d.document["steps"].as_array().unwrap().len(), 1);
    assert_eq!(d.document["steps"][0]["do"]["if"]["else"][0]["id"], "child");
}

#[test]
fn duplicate_ids_and_identity_changes_are_rejected() {
    let (_dir, store, p, d) = fixture();
    let op = Edit::AddStep {
        parent_id: None,
        lane: Lane::Main,
        index: 0,
        step: step("same"),
    };
    assert!(store
        .update(&p.project_id, &d.workflow_id, 1, &[op.clone(), op])
        .is_err());
    let mut document = d.document.clone();
    document["id"] = json!("another");
    assert!(store
        .update(
            &p.project_id,
            &d.workflow_id,
            1,
            &[Edit::ReplaceDocument { document }]
        )
        .is_err());
}

#[test]
fn tool_arguments_are_not_mistaken_for_steps() {
    let (_dir, store, p, d) = fixture();
    let d=store.update(&p.project_id,&d.workflow_id,1,&[Edit::AddStep {parent_id:None,lane:Lane::Main,index:0,
        step:json!({"id":"tool","name":"Tool","do":{"tool":"example","with":{"steps":[{"id":"tool"}]}}})}]).unwrap();
    assert_eq!(d.revision, 2);
}

#[test]
fn published_versions_are_immutable_and_publish_is_idempotent() {
    let (_dir, store, p, d) = fixture();
    let version = store
        .publish_validated(&p.project_id, &d.workflow_id, 1)
        .unwrap();
    assert_eq!(
        version,
        store
            .publish_validated(&p.project_id, &d.workflow_id, 1)
            .unwrap()
    );
    store
        .update(
            &p.project_id,
            &d.workflow_id,
            1,
            &[Edit::Rename { name: "New".into() }],
        )
        .unwrap();
    assert_eq!(
        store
            .version(&p.project_id, &version.version_id)
            .unwrap()
            .document["name"],
        "Example"
    );
    assert_eq!(
        store
            .publish_validated(&p.project_id, &d.workflow_id, 1)
            .unwrap_err()
            .code,
        "revision_conflict"
    );
}

#[test]
fn idempotency_belongs_to_client_and_rejects_changed_payload() {
    let (_dir, store, p, d) = fixture();
    let v = store
        .publish_validated(&p.project_id, &d.workflow_id, 1)
        .unwrap();
    let (run, created) = store
        .register_run(
            &p.project_id,
            "client1",
            "request1",
            "hash",
            &v,
            json!({}),
            json!([v.document]),
        )
        .unwrap();
    assert!(created);
    let (same, created) = store
        .register_run(
            &p.project_id,
            "client1",
            "request1",
            "hash",
            &v,
            json!({}),
            json!([v.document]),
        )
        .unwrap();
    assert!(!created);
    assert_eq!(run, same);
    assert_eq!(
        store
            .register_run(
                &p.project_id,
                "client1",
                "request1",
                "different",
                &v,
                json!({}),
                json!([])
            )
            .unwrap_err()
            .code,
        "idempotency_conflict"
    );
    assert!(
        store
            .register_run(
                &p.project_id,
                "client2",
                "request1",
                "hash",
                &v,
                json!({}),
                json!([])
            )
            .unwrap()
            .1
    );
}

#[test]
fn events_replay_after_reopen_and_terminal_runs_cannot_resume() {
    let (_dir, store, p, d) = fixture();
    let v = store
        .publish_validated(&p.project_id, &d.workflow_id, 1)
        .unwrap();
    let (run, _) = store
        .register_run(
            &p.project_id,
            "client",
            "request",
            "hash",
            &v,
            json!({}),
            json!([]),
        )
        .unwrap();
    store
        .transition(&p.project_id, &run.run_id, RunStatus::Running, None)
        .unwrap();
    store
        .transition(&p.project_id, &run.run_id, RunStatus::Paused, None)
        .unwrap();
    let first = store
        .events(&p.project_id, Some(&run.run_id), 0, 2)
        .unwrap();
    let reopened = WorkbenchStore::open(store.root()).unwrap();
    let rest = reopened
        .events(
            &p.project_id,
            Some(&run.run_id),
            first.last().unwrap().cursor,
            100,
        )
        .unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].kind, "run.paused");
    assert_eq!(reopened.recover_interrupted().unwrap(), 1);
    assert_eq!(reopened.recover_interrupted().unwrap(), 0);
    assert_eq!(
        reopened
            .transition(&p.project_id, &run.run_id, RunStatus::Running, None)
            .unwrap_err()
            .code,
        "invalid_run_state"
    );
}

#[test]
fn deletion_preserves_versions_and_execution_evidence() {
    let (_dir, store, p, d) = fixture();
    let v = store
        .publish_validated(&p.project_id, &d.workflow_id, 1)
        .unwrap();
    let (run, _) = store
        .register_run(
            &p.project_id,
            "client",
            "request",
            "hash",
            &v,
            json!({}),
            json!([]),
        )
        .unwrap();
    store.delete(&p.project_id, &d.workflow_id, 1).unwrap();
    assert_eq!(
        store.draft(&p.project_id, &d.workflow_id).unwrap_err().code,
        "not_found"
    );
    assert_eq!(store.version(&p.project_id, &v.version_id).unwrap(), v);
    assert_eq!(store.run(&p.project_id, &run.run_id).unwrap(), run);
}
