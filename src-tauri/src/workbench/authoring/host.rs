//! Storage adapters only. Upstream owns tools, skills, permissions and the agent loop.
use super::*;
use nuphus::permissions::{ToolCategory, ToolPermissions};

fn operations() -> Vec<nuphus_workbench::catalog::Operation> {
    nuphus_workbench::catalog::operations()
        .into_iter()
        .filter(|op| !op.name.starts_with("project."))
        .collect()
}

pub(super) fn operation(tool: &str) -> Option<nuphus_workbench::catalog::Operation> {
    operations()
        .into_iter()
        .find(|op| op.name.replace('.', "_") == tool)
}

fn definition(name: &str, description: &str, parameters: Value) -> ToolDefinition {
    ToolDefinition {
        tool_type: "function".into(),
        function: FunctionDefinition {
            name: name.into(),
            description: Some(description.into()),
            parameters,
            permission: None,
        },
    }
}

pub(super) fn schemas() -> Vec<ToolDefinition> {
    static SCHEMAS: std::sync::OnceLock<Vec<ToolDefinition>> = std::sync::OnceLock::new();
    SCHEMAS.get_or_init(build_schemas).clone()
}

fn build_schemas() -> Vec<ToolDefinition> {
    let mut tools: Vec<_> = operations()
        .into_iter()
        .map(|op| {
            let mut parameters = op.input_schema;
            parameters["properties"]
                .as_object_mut()
                .unwrap()
                .remove("project_id");
            parameters["required"]
                .as_array_mut()
                .unwrap()
                .retain(|key| key != "project_id");
            // Omitted workflow_id means the active canvas. Explicit IDs resolve only in this project.
            parameters["required"]
                .as_array_mut()
                .unwrap()
                .retain(|key| key != "workflow_id");
            definition(&op.name.replace('.', "_"), op.description, parameters)
        })
        .collect();
    for tool in &mut tools {
        if tool.function.name == "workflow_tools" {
            tool.function.parameters["properties"]["offset"] =
                json!({"type":"integer","minimum":0});
            tool.function.parameters["properties"]["limit"] =
                json!({"type":"integer","minimum":1,"maximum":50});
            tool.function.description = Some("Read complete native tool definitions in stable name order. Follow next_offset until null; keep tool_filter unchanged between pages.".into());
        }
    }
    tools.push(definition("workbench_schema", "Compatibility schema query. Prefer workflow_schema and paged workflow_tools.",
        json!({"type":"object","properties":{"tool_filter":{"type":"string"}},"additionalProperties":false})));
    let registry = nuphus::ToolRegistry::work_agent();
    for name in ["schedule_cron", "wf_call"] {
        if let Some(mut schema) = registry
            .get_schemas()
            .into_iter()
            .find(|s| s.function.name == name)
        {
            schema.function.description = Some(if name == "wf_call" {
                "Run a saved workflow in the current project. Returns a durable run_id; inspect run_get/run_steps until terminal. Never infer completion from launch."
            } else { "Manage this project's schedules using the upstream action/list/add/remove arguments. add/remove require the last-read draft revision." }.into());
            if name == "schedule_cron" {
                schema.function.parameters["properties"]["revision"] =
                    json!({"type":"integer","minimum":0});
            }
            if name == "wf_call" {
                // Authoring launches a durable run, not an Executor call frame.
                // Output mappings belong to call steps inside the workflow IR.
                schema.function.parameters["properties"]
                    .as_object_mut()
                    .unwrap()
                    .remove("outputs");
                schema.function.parameters["properties"]["request_id"] =
                    json!({"type":"string","minLength":1});
                schema.function.parameters["required"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("request_id"));
            }
            tools.push(schema);
        }
    }
    tools
}

pub(super) const CONTRACT: &str = r#"### 当前宿主：灵雀项目画布
通用设计方法、技能、工具与权限均沿用原版。项目已绑定，不要请求或猜测 project_id。
先读 canvas_get 和 workflow_schema；工具定义使用 workflow_tools 按需分页读取。
上游技能中的 workflow.json/index.json 是原版文件存储说明：此宿主的对应操作是 canvas_update（原生 IR、精确 revision）与 workflow_save（发布不可变 version_id）。不要写另一个文件工作流作为交付。
params.json、guide.md 等辅助产物仍可通过原版文件工具写入当前项目；步骤字段与表达式沿用原版。文档说明同步到画布 document.doc。
发生 revision_conflict 时重新读取并合并用户修改。未完成内容可以保留在草稿，但不能宣称已验证。
workflow_run 使用 workflow_save 返回的 version_id 与稳定 request_id；返回 run_id 只表示已启动，继续 run_get/run_steps 读取实际结果，失败按 workflow-recovery 修复。未知回执重试沿用同一 request_id，不创建第二次操作。
工作流、子工作流、调度和运行记录均通过当前项目工具访问；workflow_id 省略时是当前画布，子工作流可用 canvas_create 创建后传其 ID。
真实桌面、浏览器、Shell 和试运行遵循原版当前权限配置，无额外阶段确认。取消后不要自动重新执行。
既有会话可能来自只编辑版本；本轮重新读取画布、完整规格及适用技能，以当前工具契约为准。
"#;

#[async_trait]
impl WorkflowAuthoring for Authoring {
    fn schemas(&self) -> Vec<ToolDefinition> {
        schemas()
    }
    fn system_prompt(&self) -> String {
        CONTRACT.into()
    }
    fn authorize(&self) -> std::result::Result<ToolPermissions, String> {
        self.check_context().map_err(|e| e.to_string())?;
        let state = self.service.host.app.state::<crate::state::AppState>();
        let permissions = *state
            .tool_permissions_ref
            .lock()
            .map_err(|e| e.to_string())?;
        Ok(permissions)
    }
    fn tool_category(&self, tool: &str) -> ToolCategory {
        match operation(tool).map(|op| op.capability) {
            Some("run" | "automation") => ToolCategory::SystemAutomation,
            Some("edit") => ToolCategory::FileAccess,
            _ if matches!(tool, "schedule_cron" | "wf_call") => ToolCategory::SystemAutomation,
            _ => ToolCategory::Core,
        }
    }
    fn preserve_output(&self, tool: &str, text: &str) -> bool {
        (self.handles(tool) && serde_json::from_str::<Value>(text).is_ok())
            || [
                "workflow-design",
                "workflow-interaction",
                "workflow-recovery",
            ]
            .iter()
            .any(|name| {
                tool == "skill_read"
                    && nuphus::skill::SkillRegistry::new()
                        .get_skill_md(name)
                        .is_some_and(|content| content.trim() == text.trim())
            })
    }
    async fn execute(&self, tool: &str, params: &Value) -> nuphus::ToolResult {
        match self.execute_scoped(tool, params).await {
            Ok(value) => nuphus::ToolResult::success(value.to_string()),
            Err(error) => nuphus::ToolResult::failure(error.to_string()),
        }
    }
}

impl Authoring {
    pub(super) fn check_context(&self) -> Result<()> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(ApiError::new("cancelled", "Authoring cancelled"));
        }
        if self.service.host.session_epoch()?.unwrap_or_default() != self.epoch {
            return Err(ApiError::new(
                "product_session_changed",
                "旧生成任务已停止，请重新登录。",
            ));
        }
        if self
            .service
            .store
            .draft(&self.project, &self.workflow)?
            .authoring_mode
            != nuphus_workbench::AuthoringMode::Internal
        {
            return Err(ApiError::new(
                "authoring_mode_changed",
                "This canvas switched to external authoring",
            ));
        }
        Ok(())
    }

    async fn execute_scoped(&self, tool: &str, params: &Value) -> Result<Value> {
        self.check_context()?;
        if tool == "workbench_schema" || tool == "workflow_tools" {
            let filter = params
                .get("tool_filter")
                .and_then(Value::as_str)
                .unwrap_or("");
            let state = self.service.host.app.state::<crate::state::AppState>();
            let mut tools: Vec<_> = state
                .tools
                .get_schemas()
                .into_iter()
                .filter(|s| {
                    nuphus::tools::registry::is_workflow_step_tool(&s.function.name)
                        && s.function.name.contains(filter)
                })
                .collect();
            tools.sort_by(|a, b| a.function.name.cmp(&b.function.name));
            if tool == "workbench_schema" {
                return Ok(
                    json!({"step_schema":serde_json::from_str::<Value>(include_str!("../../../../src/workflow/step_schema.json"))?,"tools":tools}),
                );
            }
            return tool_page(tools, params);
        }
        let mut args = params.clone();
        if !args.is_object() {
            return Err(ApiError::new("invalid_params", "Object required"));
        }
        let op = if tool == "schedule_cron" {
            match params["action"].as_str() {
                Some("list") => "workflow.schedule.list".to_string(),
                Some("remove") => "workflow.schedule.remove".to_string(),
                Some("add") => {
                    args["config"] = json!({"cron":params["cron"],"timezone":params.get("timezone").cloned().unwrap_or(json!("UTC")),"enabled":true});
                    "workflow.schedule.set".into()
                }
                _ => return Err(ApiError::new("invalid_params", "Unknown schedule action")),
            }
        } else if tool == "wf_call" {
            let id = params
                .get("id")
                .or_else(|| params.get("workflow_id"))
                .and_then(Value::as_str)
                .unwrap_or(&self.workflow);
            let versions = self.service.store.versions(&self.project, id)?;
            let version = versions.first().ok_or_else(|| {
                ApiError::new("version_required", "Save the child workflow before running")
            })?;
            args["version_id"] = json!(version.version_id);
            if args.get("request_id").and_then(Value::as_str).is_none() {
                return Err(ApiError::new(
                    "invalid_params",
                    "request_id required; reuse it on uncertain replies",
                ));
            }
            "workflow.run".into()
        } else {
            operation(tool)
                .ok_or_else(|| ApiError::new("unknown_operation", tool))?
                .name
                .into()
        };
        let result = self
            .service
            .dispatch_authoring(&self.project, &self.workflow, &op, args)
            .await?;
        if matches!(
            op.as_str(),
            "workflow.run" | "workflow.debug" | "automation.execute" | "automation.observe"
        ) {
            if let Some(run_id) = result["run_id"].as_str() {
                self.runs.lock().map_err(native_error)?.push(run_id.into());
                let _ = self.service.host.app.emit("workbench-authoring", json!({"project_id":self.project,"workflow_id":self.workflow,"turn_id":self.turn,"event":{"type":"run","run_id":run_id,"text":"运行已启动，等待实际结果"}}));
            }
        }
        Ok(result)
    }
}

fn tool_page(tools: Vec<ToolDefinition>, params: &Value) -> Result<Value> {
    let offset = params.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    if offset > tools.len() {
        return Err(ApiError::new("invalid_params", "offset exceeds total"));
    }
    let mut end = offset;
    let mut chars = 0;
    while end < tools.len() && end - offset < limit {
        let size = serde_json::to_string(&tools[end])?.chars().count();
        if end > offset && chars + size > 24_000 {
            break;
        }
        chars += size;
        end += 1;
    }
    Ok(
        json!({"tools":tools[offset..end],"total":tools.len(),"next_offset":if end < tools.len() { Some(end) } else { None }}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authoring_exposes_existing_service_operations_without_project_arguments() {
        let tools = schemas();
        let names: std::collections::HashSet<_> = tools.iter().map(|t| &t.function.name).collect();
        assert_eq!(names.len(), tools.len());
        for op in operations() {
            let tool = tools
                .iter()
                .find(|t| t.function.name == op.name.replace('.', "_"))
                .unwrap();
            assert!(tool.function.parameters["properties"]
                .get("project_id")
                .is_none());
            assert!(!tool.function.parameters["required"]
                .as_array()
                .unwrap()
                .contains(&json!("workflow_id")));
        }
        let child = tools.iter().find(|t| t.function.name == "wf_call").unwrap();
        assert!(child.function.parameters["properties"]
            .get("outputs")
            .is_none());
        assert!(child.function.parameters["required"]
            .as_array()
            .unwrap()
            .contains(&json!("request_id")));
    }

    #[test]
    fn authoring_pages_retain_complete_large_definitions() {
        let tools: Vec<_> = (0..23)
            .map(|i| {
                definition(
                    &format!("tool_{i:02}"),
                    &"x".repeat(9000),
                    json!({"type":"object"}),
                )
            })
            .collect();
        let mut offset = 0;
        let mut names = vec![];
        loop {
            let page = tool_page(tools.clone(), &json!({"offset":offset})).unwrap();
            let roundtrip: Value = serde_json::from_str(&page.to_string()).unwrap();
            names.extend(
                roundtrip["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["function"]["name"].as_str().unwrap().to_string()),
            );
            if let Some(next) = page["next_offset"].as_u64() {
                offset = next;
            } else {
                break;
            }
        }
        assert_eq!(
            names,
            tools
                .iter()
                .map(|t| t.function.name.clone())
                .collect::<Vec<_>>()
        );
    }
}
