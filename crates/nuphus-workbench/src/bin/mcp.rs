//! MCP stdio adapter for clients without Streamable HTTP support. This process
//! never opens the database or starts an Agent; the tray application owns runs.
use nuphus_workbench::{gateway, ApiError};
use rmcp::{
    model::*,
    service::{RequestContext, RoleServer},
    ErrorData, ServerHandler, ServiceExt,
};
use serde_json::Value;

struct Bridge {
    client: reqwest::Client,
    base: String,
    token: String,
}

impl Bridge {
    async fn operation(&self, operation: &str, args: Value) -> Result<Value, ErrorData> {
        let response = self
            .client
            .post(format!("{}/api/v1/{operation}", self.base))
            .bearer_auth(&self.token)
            .json(&args)
            .send()
            .await
            .map_err(|_| ErrorData::internal_error("Workbench is not reachable", None))?;
        let body: Value = response
            .json()
            .await
            .map_err(|_| ErrorData::internal_error("Invalid response", None))?;
        body.get("result").cloned().ok_or_else(|| {
            ErrorData::invalid_request("Workbench request rejected", body.get("error").cloned())
        })
    }
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
        self.operation("project.list", serde_json::json!({}))
            .await?;
        Ok(nuphus_workbench::resources::list())
    }
    async fn list_resource_templates(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        self.operation("project.list", serde_json::json!({}))
            .await?;
        Ok(nuphus_workbench::resources::templates())
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let (op, args) = nuphus_workbench::resources::resolve(&request.uri)
            .map_err(|e| ErrorData::invalid_params(e.message, None))?;
        let value = self.operation(op, args).await?;
        Ok(nuphus_workbench::resources::response(&request.uri, value))
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        // Do not announce a usable service when the application is stopped or
        // this token has been revoked. The remote catalog filters capabilities.
        let response = self
            .client
            .get(format!("{}/api/v1/discover", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| {
                ErrorData::internal_error(
                    "Workbench is not reachable; start the tray application",
                    None,
                )
            })?;
        if !response.status().is_success() {
            return Err(ErrorData::invalid_request("Workbench token rejected", None));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| ErrorData::internal_error("Invalid Workbench response", None))?;
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
        let response = self
            .client
            .post(format!("{}/api/v1/{operation}", self.base))
            .bearer_auth(&self.token)
            .json(&request.arguments.unwrap_or_default())
            .send()
            .await;
        let result = match response {
            Ok(response) => match response.json::<Value>().await {
                Ok(body) if body.get("error").is_some() => Err(serde_json::from_value(body["error"].clone()).unwrap_or_else(|_|ApiError::new("transport_error","Invalid error response"))),
                Ok(body) => body.get("result").cloned().ok_or_else(||ApiError::new("transport_error","Missing result")),
                Err(_) => Err(ApiError::new("transport_error","Invalid Workbench response")),
            },
            Err(_) => Err(ApiError::new("transport_error","Workbench request did not return. Reconnect and reuse the same request_id to query/retry a run; do not blindly repeat side effects.")),
        };
        Ok(gateway::tool_response(result))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base =
        std::env::var("NUPHUS_WORKBENCH_URL").unwrap_or_else(|_| "http://127.0.0.1:47731".into());
    let url = reqwest::Url::parse(&base)?;
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
    let token = std::env::var("NUPHUS_WORKBENCH_TOKEN")
        .map_err(|_| "Set NUPHUS_WORKBENCH_TOKEN to a token created in the Workbench UI")?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let bridge = Bridge {
        client,
        base: base.trim_end_matches('/').into(),
        token,
    };
    // stdout is reserved exclusively for MCP; diagnostics use stderr.
    let server = bridge.serve(rmcp::transport::stdio()).await?;
    server.waiting().await?;
    Ok(())
}
