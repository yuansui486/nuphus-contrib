use crate::{
    edit::{Edit, Lane},
    *,
};
use serde_json::json;

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
