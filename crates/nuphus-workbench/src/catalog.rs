//! One discovery contract shared by HTTP, MCP and the stdio bridge.
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Clone, Serialize)]
pub struct Operation {
    pub name: &'static str,
    pub description: &'static str,
    pub capability: &'static str,
    pub input_schema: Value,
}

pub fn operations() -> Vec<Operation> {
    let mut result = Vec::new();
    let mut add = |name, description, capability, required: &[&str], optional: &[&str]| {
        let mut properties = serde_json::Map::new();
        for key in required.iter().chain(optional) {
            let schema = match *key {
                "revision" | "layout_revision" | "after" | "invocation_id" => {
                    json!({"type":"integer","minimum":0})
                }
                "limit" => json!({"type":"integer","minimum":1,"maximum":1000}),
                "focus" => json!({"type":"boolean","default":false}),
                "document" | "inputs" | "layout" => json!({"type":"object"}),
                "authoring_mode" => json!({"type":"string","enum":["internal","external"]}),
                "operations" => json!({"type":"array","minItems":1}),
                _ => json!({"type":"string","minLength":1}),
            };
            properties.insert((*key).into(), schema);
        }
        let mut schema = json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
        if name == "canvas.update" {
            let mut edits = serde_json::to_value(schemars::schema_for!(crate::edit::Edit))
                .expect("static edit schema");
            if let Some(defs) = edits.as_object_mut().and_then(|s| s.remove("$defs")) {
                schema["$defs"] = defs;
            }
            schema["properties"]["operations"]["items"] = edits;
        }
        result.push(Operation {
            name,
            description,
            capability,
            input_schema: schema,
        });
    };
    add("system.capabilities", "Discover edition and native execution capabilities; no model is required for deterministic workflows.", "read", &[], &[]);
    add(
        "project.list",
        "List only projects authorized for this client.",
        "read",
        &[],
        &[],
    );
    add("project.register", "Register an existing absolute local directory as a project. Requires explicit project-management permission.", "projects", &["directory","name"], &[]);
    add(
        "project.get",
        "Read a registered project.",
        "read",
        &["project_id"],
        &[],
    );
    add(
        "canvas.create",
        "Create a draft. External clients default to external authoring (no internal chat panel).",
        "edit",
        &["project_id", "name"],
        &["authoring_mode"],
    );
    add("canvas.get", "Read native Workflow IR, content revision, layout and authoring mode. Edges are derived from nested IR, not a separate graph.", "read", &["project_id","workflow_id"], &[]);
    add("canvas.update", "Atomically edit the native IR using the exact last-read revision. A conflict never overwrites another editor. Move indexes are evaluated after removal; loop children are do.loop.do.", "edit", &["project_id","workflow_id","revision","operations"], &[]);
    add(
        "canvas.layout_update",
        "Save visual positions independently from content; requires last-read layout_revision.",
        "edit",
        &["project_id", "workflow_id", "layout_revision", "layout"],
        &[],
    );
    add(
        "canvas.open",
        "Ask the desktop UI to open this canvas; focus is opt-in. Does not start execution.",
        "edit",
        &["project_id", "workflow_id"],
        &["focus"],
    );
    add(
        "canvas.view_get",
        "Read live UI view/selection when open; closed canvases have no selection.",
        "read",
        &["project_id", "workflow_id"],
        &[],
    );
    add(
        "workflow.list",
        "List project workflow drafts.",
        "read",
        &["project_id"],
        &[],
    );
    add(
        "workflow.get",
        "Read a workflow draft and revisions.",
        "read",
        &["project_id", "workflow_id"],
        &[],
    );
    add("workflow.validate", "Use the same native compiler as the desktop editor; optionally validate an unsaved document.", "read", &["project_id","workflow_id"], &["document"]);
    add("workflow.save", "Validate the exact draft revision and publish an immutable version. Incomplete drafts remain editable but cannot run.", "edit", &["project_id","workflow_id","revision"], &[]);
    add(
        "workflow.versions",
        "List immutable published versions, newest first.",
        "read",
        &["project_id", "workflow_id"],
        &[],
    );
    add(
        "workflow.import",
        "Import native Workflow IR as a new draft with a new ID. Does not execute it.",
        "edit",
        &["project_id", "name", "document"],
        &["authoring_mode"],
    );
    add(
        "workflow.export",
        "Export the native workflow document (not model credentials or run inputs).",
        "read",
        &["project_id", "workflow_id"],
        &[],
    );
    add(
        "workflow.delete",
        "Delete this draft with revision check. Published versions and run evidence are retained.",
        "edit",
        &["project_id", "workflow_id", "revision"],
        &[],
    );
    add("workflow.run", "Start a published version locally. Return a durable run_id immediately. Reuse request_id with identical arguments for safe retry; use a new ID only for intentional re-execution.", "run", &["project_id","version_id","request_id"], &["inputs"]);
    add(
        "run.list",
        "List durable project runs, including interrupted runs after restart.",
        "read",
        &["project_id"],
        &[],
    );
    add(
        "run.get",
        "Read run status, redacted inputs, immutable snapshots and final result.",
        "read",
        &["project_id", "run_id"],
        &[],
    );
    add("run.events", "Replay durable ordered events after cursor (exclusive). Resume using the last received cursor after reconnect. Omit run_id for project-wide canvas and run events.", "read", &["project_id"], &["run_id","after","limit"]);
    add("run.steps", "Read step evidence; specify invocation_id for a particular step invocation (loops may invoke the same step repeatedly).", "read", &["project_id","run_id"], &["invocation_id"]);
    add("run.pause", "Request a pause at a safe execution boundary; does not interrupt an in-flight native action.", "run", &["project_id","run_id"], &[]);
    add(
        "run.resume",
        "Resume an explicitly paused run without starting a second run.",
        "run",
        &["project_id", "run_id"],
        &[],
    );
    add("run.cancel", "Request cancellation. Observe run events until terminal status; an in-flight action may finish first.", "run", &["project_id","run_id"], &[]);
    add("run.respond", "Respond to the current explicit wait using its request_id and decision (continue or cancel). Stale replies never approve a later wait. Requires respond permission.", "respond", &["project_id","run_id","request_id","decision"], &[]);
    add("automation.capabilities", "Discover native automation availability and restrictions before building automation steps.", "read", &["project_id"], &[]);
    result
}
