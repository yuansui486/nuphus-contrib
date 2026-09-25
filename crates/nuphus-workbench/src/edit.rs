//! Structured editing of the existing nested Workflow IR, not a second graph.
use crate::{ApiError, AuthoringMode, Draft, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    AddStep {
        parent_id: Option<String>,
        lane: Lane,
        index: usize,
        step: Value,
    },
    RemoveStep {
        step_id: String,
    },
    MoveStep {
        step_id: String,
        parent_id: Option<String>,
        lane: Lane,
        index: usize,
    },
    UpdateFields {
        step_id: String,
        fields: Map<String, Value>,
    },
    SetInputs {
        inputs: Vec<Value>,
    },
    Rename {
        name: String,
    },
    SetAuthoringMode {
        mode: AuthoringMode,
    },
    /// Imports/editor saves are also revision-checked by the store.
    ReplaceDocument {
        document: Value,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    Main,
    Then,
    Else,
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::new("invalid_edit", message)
}

/// Only known container fields are traversed. A tool parameter containing an
/// unrelated `id` or `steps` object must never be mistaken for an IR node.
fn child_paths(step: &Value, path: &str) -> Vec<String> {
    [
        "/do/seq",
        "/do/loop/body",
        "/do/if/then",
        "/do/if/else",
        "/do/auto",
    ]
    .iter()
    .filter(|suffix| step.pointer(suffix).is_some_and(Value::is_array))
    .map(|suffix| format!("{path}{suffix}"))
    .collect()
}

fn visit(
    document: &Value,
    list_path: &str,
    entries: &mut Vec<(String, String)>,
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(invalid("Workflow nesting exceeds 128 levels"));
    }
    let list = document
        .pointer(list_path)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("Container steps must be an array"))?;
    for (index, step) in list.iter().enumerate() {
        let id = step
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && !id.contains("::"))
            .ok_or_else(|| invalid("Each step needs a nonempty, non-synthetic id"))?;
        if !step.get("name").is_some_and(Value::is_string)
            || !step.get("do").is_some_and(Value::is_object)
        {
            return Err(invalid(format!("Step '{id}' needs a name and a do object")));
        }
        let path = format!("{list_path}/{index}");
        entries.push((id.into(), path.clone()));
        for child in child_paths(step, &path) {
            visit(document, &child, entries, depth + 1)?;
        }
    }
    Ok(())
}

pub fn check_document(document: &Value, id: &str) -> Result<()> {
    if document.get("id").and_then(Value::as_str) != Some(id) {
        return Err(invalid("Workflow id cannot be changed"));
    }
    if !document.get("name").is_some_and(Value::is_string) {
        return Err(invalid("Workflow name must be a string"));
    }
    let mut entries = Vec::new();
    visit(document, "/steps", &mut entries, 0)?;
    let mut ids = HashSet::new();
    for (id, _) in entries {
        if !ids.insert(id.clone()) {
            return Err(invalid(format!("Duplicate step id '{id}'")));
        }
    }
    Ok(())
}

fn locate(document: &Value, id: &str) -> Result<String> {
    let mut entries = Vec::new();
    visit(document, "/steps", &mut entries, 0)?;
    entries
        .into_iter()
        .find(|(key, _)| key == id)
        .map(|(_, path)| path)
        .ok_or_else(|| invalid(format!("Step '{id}' does not exist")))
}

fn lane_path(document: &Value, parent: Option<&str>, lane: Lane) -> Result<String> {
    let Some(parent) = parent else {
        return if lane == Lane::Main {
            Ok("/steps".into())
        } else {
            Err(invalid("Root only has a main lane"))
        };
    };
    let path = locate(document, parent)?;
    let step = document
        .pointer(&path)
        .ok_or_else(|| invalid("Parent does not exist"))?;
    let suffix = if step.pointer("/do/if").is_some() {
        match lane {
            Lane::Then => "/do/if/then",
            Lane::Else => "/do/if/else",
            Lane::Main => return Err(invalid("Condition requires then or else lane")),
        }
    } else if lane != Lane::Main {
        return Err(invalid("Container only has a main lane"));
    } else if step.pointer("/do/seq").is_some() {
        "/do/seq"
    } else if step.pointer("/do/loop").is_some() {
        "/do/loop/body"
    } else if step.pointer("/do/wait").is_some() {
        "/do/auto"
    } else {
        return Err(invalid("Target step is not a supported container"));
    };
    Ok(format!("{path}{suffix}"))
}

fn insert(
    document: &mut Value,
    parent: Option<&str>,
    lane: Lane,
    index: usize,
    step: Value,
) -> Result<()> {
    let path = lane_path(document, parent, lane)?;
    // Optional empty else/auto/body arrays may be absent in upstream JSON.
    if document.pointer(&path).is_none() {
        let (owner, key) = path
            .rsplit_once('/')
            .ok_or_else(|| invalid("Invalid container"))?;
        document
            .pointer_mut(owner)
            .and_then(Value::as_object_mut)
            .ok_or_else(|| invalid("Invalid container"))?
            .insert(key.into(), Value::Array(Vec::new()));
    }
    let list = document
        .pointer_mut(&path)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("Target lane is not an array"))?;
    if index > list.len() {
        return Err(invalid("Insertion index is out of bounds"));
    }
    list.insert(index, step);
    Ok(())
}

fn remove(document: &mut Value, id: &str) -> Result<Value> {
    let path = locate(document, id)?;
    let (owner, index) = path
        .rsplit_once('/')
        .ok_or_else(|| invalid("Invalid step location"))?;
    let index = index
        .parse::<usize>()
        .map_err(|_| invalid("Invalid step index"))?;
    let list = document
        .pointer_mut(owner)
        .and_then(Value::as_array_mut)
        .ok_or_else(|| invalid("Invalid parent"))?;
    Ok(list.remove(index))
}

pub fn apply(draft: &mut Draft, operation: &Edit) -> Result<()> {
    match operation {
        Edit::AddStep {
            parent_id,
            lane,
            index,
            step,
        } => insert(
            &mut draft.document,
            parent_id.as_deref(),
            *lane,
            *index,
            step.clone(),
        )?,
        Edit::RemoveStep { step_id } => {
            remove(&mut draft.document, step_id)?;
        }
        Edit::MoveStep {
            step_id,
            parent_id,
            lane,
            index,
        } => {
            if let Some(parent) = parent_id {
                let source = locate(&draft.document, step_id)?;
                let target = locate(&draft.document, parent)?;
                if target == source || target.starts_with(&format!("{source}/")) {
                    return Err(invalid("Cannot move a container into itself"));
                }
            }
            let step = remove(&mut draft.document, step_id)?;
            // index is evaluated in the destination after removal.
            insert(
                &mut draft.document,
                parent_id.as_deref(),
                *lane,
                *index,
                step,
            )?;
        }
        Edit::UpdateFields { step_id, fields } => {
            let path = locate(&draft.document, step_id)?;
            let step = draft
                .document
                .pointer_mut(&path)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| invalid("Invalid step"))?;
            for (key, value) in fields {
                if ![
                    "name",
                    "description",
                    "on_error",
                    "capture",
                    "timeout_secs",
                    "do",
                ]
                .contains(&key.as_str())
                {
                    return Err(invalid(format!("Cannot update step field '{key}'")));
                }
                step.insert(key.clone(), value.clone());
            }
        }
        Edit::SetInputs { inputs } => {
            draft.document["inputs"] = Value::Array(inputs.clone());
        }
        Edit::Rename { name } => {
            if name.trim().is_empty() {
                return Err(invalid("Name is required"));
            }
            draft.document["name"] = Value::String(name.trim().into());
        }
        Edit::SetAuthoringMode { mode } => {
            draft.authoring_mode = *mode;
        }
        Edit::ReplaceDocument { document } => {
            draft.document = document.clone();
        }
    }
    check_document(&draft.document, &draft.workflow_id)
}
