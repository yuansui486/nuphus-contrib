//! Optional execution context for hosts with versioned workflow storage.
//! Ordinary Nuphus runs retain their existing store and lifecycle.
use super::store::WorkflowStore;
use std::{path::PathBuf, sync::Arc};

pub struct RunContext {
    pub run_id: String,
    pub project_dir: PathBuf,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub store: WorkflowStore,
    pub event_sink: Option<Arc<dyn Fn(&super::events::WorkflowEvent) + Send + Sync>>,
}

tokio::task_local! { pub static CURRENT: Arc<RunContext>; }

pub fn current() -> Option<Arc<RunContext>> {
    CURRENT.try_with(Arc::clone).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{
        types::{Step, Workflow},
        WorkflowEngine, WorkflowRunSource,
    };
    use std::sync::Mutex;

    #[tokio::test]
    async fn versioned_context_uses_frozen_definitions_and_one_real_run_identity() {
        let root = std::env::temp_dir().join(format!("nuphus-context-{}", uuid::Uuid::new_v4()));
        let mut workflow = Workflow::new("Frozen version");
        workflow.steps = vec![Step::new_seq("group", "Group", vec![])];
        let id = workflow.id.clone();
        let run_id = uuid::Uuid::new_v4().to_string();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = events.clone();
        let context = Arc::new(RunContext {
            run_id: run_id.clone(),
            project_dir: root.join("project"),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            store: WorkflowStore::frozen(root.clone(), vec![workflow]),
            event_sink: Some(Arc::new(move |event| {
                captured.lock().unwrap().push(event.clone())
            })),
        });
        let engine = WorkflowEngine::new();
        assert!(engine.store.get(&id).await.is_none());
        CURRENT
            .scope(context.clone(), async {
                assert_eq!(crate::utils::work_root(), context.project_dir);
                engine
                    .execute_workflow(
                        &id,
                        |_, _| async { panic!("Empty group must not execute tools") },
                        None,
                        None,
                        None,
                        true,
                        WorkflowRunSource::External,
                    )
                    .await
                    .unwrap();
            })
            .await;
        assert!(current().is_none());
        let traces = crate::workflow::trace::list(&root, &id, false)
            .await
            .unwrap();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].run_id, run_id);
        for event in events.lock().unwrap().iter() {
            if let super::super::events::WorkflowEvent::RunStarted { run_id: id, .. }
            | super::super::events::WorkflowEvent::RunCompleted { run_id: id, .. } = event
            {
                assert_eq!(id, &run_id);
            }
        }
        assert!(engine.store.get(&id).await.is_none());
        // Only this test's fresh UUID directory is removed.
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn blocking_tools_inherit_project_context_without_changing_process_directory() {
        let before = std::env::current_dir().unwrap();
        let project = before.join(format!("test-context-{}", uuid::Uuid::new_v4()));
        let context = Arc::new(RunContext {
            run_id: uuid::Uuid::new_v4().to_string(),
            project_dir: project.clone(),
            store: WorkflowStore::frozen(project.clone(), vec![]),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            event_sink: None,
        });
        let mut tools = crate::ToolRegistry::new();
        tools.register(crate::tools::registry::ToolDef {
            name: "test_project_context".into(),
            description: String::new(),
            parameters: serde_json::json!({}),
            category: crate::permissions::ToolCategory::FileAccess,
            depends_on: vec![],
            executor: |_, _| {
                Ok(crate::ToolResult::success(
                    crate::utils::work_root().to_string_lossy().to_string(),
                ))
            },
        });
        let result = CURRENT
            .scope(
                context,
                tools.execute("test_project_context", &serde_json::json!({})),
            )
            .await
            .unwrap();
        assert_eq!(
            result.output.as_deref(),
            Some(project.to_string_lossy().as_ref())
        );
        assert_eq!(std::env::current_dir().unwrap(), before);
        assert!(current().is_none());
    }
}
