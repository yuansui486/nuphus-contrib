//! Loopback-only transports. Authentication is per request, not per connection.
//! Execution belongs to the host and survives a disconnected HTTP/MCP client.
use crate::{
    auth::Principal,
    catalog,
    service::{Host, Service},
    ApiError, Result,
};
use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    routing::{get, post},
    Extension, Json, Router,
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::{RequestContext, RoleServer},
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct Gateway<H: Host> {
    service: Arc<Service<H>>,
    sessions: Mutex<HashMap<String, (String, Instant)>>,
}

#[derive(Clone)]
struct Bearer(String);

fn bearer(headers: &HeaderMap) -> Result<Bearer> {
    headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty() && t.len() < 512)
        .map(|t| Bearer(t.into()))
        .ok_or_else(|| {
            ApiError::new(
                "unauthorized",
                "A Workbench client Bearer token is required",
            )
        })
}

fn error_response(error: ApiError) -> Response {
    let status = match error.code.as_str() {
        "unauthorized" => StatusCode::UNAUTHORIZED,
        "permission_denied" => StatusCode::FORBIDDEN,
        "not_found" | "unknown_operation" => StatusCode::NOT_FOUND,
        "revision_conflict" | "idempotency_conflict" | "invalid_run_state" | "automation_busy" => {
            StatusCode::CONFLICT
        }
        "storage_error" | "native_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    (status, Json(json!({"error":error}))).into_response()
}

async fn authorize<H: Host>(
    State(state): State<Arc<Gateway<H>>>,
    mut request: Request,
    next: Next,
) -> Response {
    // The UI uses Tauri IPC. Websites need no access to this local control API.
    let host = request
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<axum::http::uri::Authority>().ok());
    if request.headers().contains_key("origin")
        || !host.is_some_and(|h| matches!(h.host(), "localhost" | "127.0.0.1" | "[::1]"))
    {
        return error_response(ApiError::new(
            "permission_denied",
            "Only local non-browser clients are supported",
        ));
    }
    let token = match bearer(request.headers()) {
        Ok(token) => token,
        Err(e) => return error_response(e),
    };
    let principal = match state.service.store.authenticate(&token.0) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };
    let session = request
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    if let Some(id) = &session {
        let Ok(mut sessions) = state.sessions.lock() else {
            return error_response(ApiError::new(
                "native_error",
                "Session registry unavailable",
            ));
        };
        sessions.retain(|_, (_, time)| time.elapsed() < Duration::from_secs(3600));
        match sessions.get_mut(id) {
            Some((owner, time)) if owner == principal.id() => *time = Instant::now(),
            _ => {
                return error_response(ApiError::new(
                    "permission_denied",
                    "Unknown session or different session owner; reconnect",
                ))
            }
        }
    }
    let deleting = request.method() == axum::http::Method::DELETE;
    request.extensions_mut().insert(principal.clone());
    request.extensions_mut().insert(token);
    let response = next.run(request).await;
    if let Ok(mut sessions) = state.sessions.lock() {
        if deleting && response.status().is_success() {
            if let Some(id) = session {
                sessions.remove(&id);
            }
        } else if let Some(id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|h| h.to_str().ok())
        {
            sessions.insert(id.into(), (principal.id().into(), Instant::now()));
        }
    }
    response
}

async fn call<H: Host>(
    State(state): State<Arc<Gateway<H>>>,
    Extension(token): Extension<Bearer>,
    Path(operation): Path<String>,
    Json(args): Json<Value>,
) -> Response {
    // Recheck after receiving/parsing the body, in case the token was revoked.
    let principal = match state.service.store.authenticate(&token.0) {
        Ok(p) => p,
        Err(e) => return error_response(e),
    };
    match state.service.dispatch(&principal, &operation, args).await {
        Ok(value) => Json(json!({"result":value})).into_response(),
        Err(error) => error_response(error),
    }
}

async fn discover(Extension(principal): Extension<Principal>) -> Json<Value> {
    Json(
        json!({"api_version":crate::API_VERSION,"operations":catalog::operations().into_iter()
        .filter(|op|principal.authorize(op.capability,None).is_ok()).collect::<Vec<_>>()}),
    )
}

#[derive(serde::Deserialize)]
struct EventQuery {
    project_id: String,
    run_id: Option<String>,
    #[serde(default)]
    after: u64,
}

async fn events<H: Host>(
    State(state): State<Arc<Gateway<H>>>,
    Extension(token): Extension<Bearer>,
    Query(query): Query<EventQuery>,
) -> Response {
    let initial = state
        .service
        .store
        .authenticate(&token.0)
        .and_then(|p| p.authorize("read", Some(&query.project_id)))
        .and_then(|_| {
            state
                .service
                .store
                .events(&query.project_id, query.run_id.as_deref(), query.after, 1)
        });
    if let Err(e) = initial {
        return error_response(e);
    }
    let stream = futures_util::stream::unfold(
        (state, token, query, false),
        |(state, token, mut query, done)| async move {
            if done {
                return None;
            }
            loop {
                let result = state
                    .service
                    .store
                    .authenticate(&token.0)
                    .and_then(|p| p.authorize("read", Some(&query.project_id)))
                    .and_then(|_| {
                        state.service.store.events(
                            &query.project_id,
                            query.run_id.as_deref(),
                            query.after,
                            1,
                        )
                    });
                match result {
                    Ok(items) => {
                        if let Some(item) = items.first() {
                            query.after = item.cursor;
                            let event = Event::default()
                                .id(item.cursor.to_string())
                                .event(&item.kind)
                                .data(serde_json::to_string(item).unwrap_or_default());
                            return Some((
                                Ok::<_, Infallible>(event),
                                (state, token, query, false),
                            ));
                        }
                    }
                    Err(error) => {
                        let event = Event::default()
                            .event("error")
                            .data(json!({"error":error}).to_string());
                        return Some((Ok(event), (state, token, query, true)));
                    }
                }
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        },
    );
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

struct Mcp<H: Host> {
    service: Arc<Service<H>>,
}

impl<H: Host> Mcp<H> {
    fn principal(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> std::result::Result<Principal, McpError> {
        let token = context
            .extensions
            .get::<axum::http::request::Parts>()
            .and_then(|p| p.extensions.get::<Bearer>())
            .ok_or_else(|| {
                McpError::invalid_request("Authenticated HTTP context required", None)
            })?;
        self.service
            .store
            .authenticate(&token.0)
            .map_err(|e| McpError::invalid_request(e.message, None))
    }
}

pub fn server_info() -> ServerConfig {
    ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
        .with_instructions("Nuphus Workbench: native project/workflow service, not an Agent proxy. Read a canvas revision before editing. Publish then run with a unique request_id; reuse that ID for retries. Use run_events cursors to follow durable progress. Closing the client does not cancel a run.")
}

pub fn tool_list(principal: Option<&Principal>) -> ListToolsResult {
    ListToolsResult {
        tools: catalog::operations()
            .into_iter()
            .filter(|op| principal.is_none_or(|p| p.authorize(op.capability, None).is_ok()))
            .map(|op| {
                Tool::new(
                    op.name.replace('.', "_"),
                    op.description,
                    op.input_schema
                        .as_object()
                        .expect("static object schema")
                        .clone(),
                )
            })
            .collect(),
        ..Default::default()
    }
}

pub fn operation_for_tool(name: &str) -> Option<&'static str> {
    catalog::operations()
        .into_iter()
        .find(|op| op.name.replace('.', "_") == name)
        .map(|op| op.name)
}

pub fn tool_response(result: Result<Value>) -> CallToolResponse {
    match result {
        Ok(value) => CallToolResult::structured(json!({"result":value})),
        Err(error) => CallToolResult::structured_error(json!({"error":error})),
    }
    .into()
}

impl<H: Host> ServerHandler for Mcp<H> {
    fn get_info(&self) -> ServerConfig {
        server_info()
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        Ok(tool_list(Some(&self.principal(&context)?)))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        let principal = self.principal(&context)?;
        let operation = operation_for_tool(&request.name)
            .ok_or_else(|| McpError::invalid_params("Unknown Workbench tool", None))?;
        Ok(tool_response(
            self.service
                .dispatch(
                    &principal,
                    operation,
                    Value::Object(request.arguments.unwrap_or_default()),
                )
                .await,
        ))
    }
}

pub fn router<H: Host>(service: Arc<Service<H>>) -> Router {
    let mcp_service = service.clone();
    let mcp = StreamableHttpService::new(
        move || {
            Ok(Mcp {
                service: mcp_service.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().enforce_origin_validation(),
    );
    let state = Arc::new(Gateway {
        service,
        sessions: Mutex::new(HashMap::new()),
    });
    Router::new()
        .route("/api/v1/:operation", post(call::<H>))
        .route("/api/v1/discover", get(discover))
        .route("/api/v1/events", get(events::<H>))
        .nest_service("/mcp", mcp)
        .layer(DefaultBodyLimit::max(4 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authorize::<H>,
        ))
        .with_state(state)
}

/// Bind only loopback. Remote access must use an authenticated local tunnel;
/// arbitrary network binding is deliberately not a silent configuration option.
pub async fn bind(port: u16) -> std::io::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await
}
