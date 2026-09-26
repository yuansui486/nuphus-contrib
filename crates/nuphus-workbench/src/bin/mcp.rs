//! Local MCP app; an explicit legacy URL selects HTTP instead of local IPC.
use nuphus_workbench::{gateway, local, ApiError};
use rmcp::{
    model::*,
    service::{RequestContext, RoleServer},
    ErrorData, ServerHandler, ServiceExt,
};
use serde_json::{json, Value};

struct Http {
    client: reqwest::Client,
    base: String,
    token: Option<String>,
}
enum Bridge {
    Local(local::Client),
    Http(Http),
}
impl Http {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let request = self
            .client
            .request(method, format!("{}{}", self.base, path));
        match &self.token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    }
}
impl Bridge {
    async fn operation(&self, operation: &str, args: Value) -> nuphus_workbench::Result<Value> {
        match self {
            Self::Local(client) => client.operation(operation, args).await,
            Self::Http(http) => {
                let response = http.request(reqwest::Method::POST, &format!("/api/v1/{operation}"))
                    .json(&args).send().await.map_err(|_| ApiError::new("transport_error",
                        "请求结果未确认，请先查询运行状态；重试运行请复用原 request_id，勿重复执行副作用。").details(json!({"outcome_unknown":true})))?;
                let body: Value = response.json().await.map_err(|_| {
                    ApiError::new("transport_error", "Invalid Workbench response")
                        .details(json!({"outcome_unknown":true}))
                })?;
                if let Some(error) = body.get("error") {
                    return Err(serde_json::from_value(error.clone())?);
                }
                body.get("result")
                    .cloned()
                    .ok_or_else(|| ApiError::new("transport_error", "Missing Workbench result"))
            }
        }
    }
}
fn mcp_error(error: ApiError) -> ErrorData {
    ErrorData::internal_error(error.to_string(), serde_json::to_value(error).ok())
}
impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        gateway::server_info()
    }
    async fn list_resources(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        self.operation("project.list", json!({}))
            .await
            .map_err(mcp_error)?;
        Ok(nuphus_workbench::resources::list())
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.operation("project.list", json!({}))
            .await
            .map_err(mcp_error)?;
        Ok(nuphus_workbench::resources::templates())
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let (operation, args) =
            nuphus_workbench::resources::resolve(&request.uri).map_err(mcp_error)?;
        let value = self.operation(operation, args).await.map_err(mcp_error)?;
        Ok(nuphus_workbench::resources::response(&request.uri, value))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let body = match self {
            Self::Local(_) => self
                .operation("system.capabilities", json!({}))
                .await
                .map_err(mcp_error)?,
            Self::Http(http) => {
                let response = http
                    .request(reqwest::Method::GET, "/api/v1/discover")
                    .send()
                    .await
                    .map_err(|_| {
                        ErrorData::internal_error("工作台不可达，请检查已配置的 HTTP 地址。", None)
                    })?;
                if !response.status().is_success() {
                    return Err(ErrorData::invalid_request("Workbench token rejected", None));
                }
                response
                    .json::<Value>()
                    .await
                    .map_err(|_| ErrorData::internal_error("Invalid Workbench response", None))?
            }
        };
        let mut tools = gateway::tool_list(None);
        tools.tools.retain(|tool| {
            body["operations"].as_array().is_some_and(|ops| {
                ops.iter().any(|op| {
                    op["name"]
                        .as_str()
                        .is_some_and(|n| n.replace('.', "_") == tool.name)
                })
            })
        });
        Ok(tools)
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let operation = gateway::operation_for_tool(&request.name)
            .ok_or_else(|| ErrorData::invalid_params("Unknown Workbench tool", None))?;
        Ok(gateway::tool_response(
            self.operation(operation, json!(request.arguments.unwrap_or_default()))
                .await,
        ))
    }
}
fn http(base: &str, token: Option<String>) -> Result<Http, Box<dyn std::error::Error>> {
    let url = reqwest::Url::parse(base)?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("NUPHUS_WORKBENCH_URL must be a loopback HTTP origin".into());
    }
    Ok(Http {
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(60))
            .build()?,
        base: base.trim_end_matches('/').into(),
        token,
    })
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() && args != ["serve"] {
        return Err("Usage: nuphus-workbench-mcp [serve]".into());
    }
    let token = std::env::var("NUPHUS_WORKBENCH_TOKEN")
        .ok()
        .filter(|s| !s.is_empty());
    let bridge = match std::env::var("NUPHUS_WORKBENCH_URL") {
        Ok(base) => Bridge::Http(http(&base, token)?),
        Err(_) => Bridge::Local(local::Client::installed(token)?),
    };
    // stdout is exclusively MCP, never startup logs or the host console.
    bridge
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_http_keeps_tokens_and_validates_origin() {
        let http = http("http://127.0.0.1:47731", Some("legacy-token".into())).unwrap();
        assert_eq!(
            http.request(reqwest::Method::GET, "/api/v1/discover")
                .build()
                .unwrap()
                .headers()["authorization"],
            "Bearer legacy-token"
        );
        assert!(super::http("https://example.com", None).is_err());
        assert!(super::http("http://127.0.0.1/other", None).is_err());
        assert!(super::http("http://127.0.0.1", None)
            .unwrap()
            .request(reqwest::Method::GET, "/")
            .build()
            .unwrap()
            .headers()
            .get("authorization")
            .is_none());
    }
}
