//! MCP resources resolve to the same authorized service operations as tools.
use crate::{ApiError, Result};
use rmcp::model::*;
use serde_json::{json, Value};

pub fn list() -> ListResourcesResult {
    ListResourcesResult {
        resources: vec![
            Resource::new("workbench://projects", "projects").with_mime_type("application/json")
        ],
        ..Default::default()
    }
}

pub fn templates() -> ListResourceTemplatesResult {
    ListResourceTemplatesResult {
        resource_templates: [
            ("workbench://projects/{project_id}", "project"),
            ("workbench://projects/{project_id}/canvases", "canvases"),
            (
                "workbench://projects/{project_id}/canvases/{workflow_id}",
                "canvas",
            ),
            (
                "workbench://projects/{project_id}/canvases/{workflow_id}/versions",
                "versions",
            ),
            ("workbench://projects/{project_id}/runs", "runs"),
            ("workbench://projects/{project_id}/runs/{run_id}", "run"),
        ]
        .into_iter()
        .map(|(uri, name)| ResourceTemplate::new(uri, name).with_mime_type("application/json"))
        .collect(),
        ..Default::default()
    }
}

pub fn resolve(uri: &str) -> Result<(&'static str, Value)> {
    let path = uri
        .strip_prefix("workbench://projects")
        .ok_or_else(|| ApiError::new("not_found", "Unknown resource URI"))?;
    if path.is_empty() {
        return Ok(("project.list", json!({})));
    }
    let parts: Vec<_> = path
        .strip_prefix('/')
        .unwrap_or_default()
        .split('/')
        .collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
    {
        return Err(ApiError::new(
            "invalid_params",
            "Use exact IDs returned by the service",
        ));
    }
    match parts.as_slice() {
        [project] => Ok(("project.get", json!({"project_id":project}))),
        [project, "canvases"] => Ok(("workflow.list", json!({"project_id":project}))),
        [project, "canvases", workflow] => Ok((
            "canvas.get",
            json!({"project_id":project,"workflow_id":workflow}),
        )),
        [project, "canvases", workflow, "versions"] => Ok((
            "workflow.versions",
            json!({"project_id":project,"workflow_id":workflow}),
        )),
        [project, "runs"] => Ok(("run.list", json!({"project_id":project}))),
        [project, "runs", run] => Ok(("run.get", json!({"project_id":project,"run_id":run}))),
        _ => Err(ApiError::new("not_found", "Unknown resource URI")),
    }
}

pub fn response(uri: &str, value: Value) -> ReadResourceResponse {
    ReadResourceResult::new(vec![
        ResourceContents::text(value.to_string(), uri).with_mime_type("application/json")
    ])
    .with_ttl_ms(0)
    .into()
}
