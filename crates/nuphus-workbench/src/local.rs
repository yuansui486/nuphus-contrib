//! Local app transport: no port discovery, database access or secondary executor.
mod platform;
use crate::{
    auth::Principal,
    service::{Host, Service},
    ApiError, Result,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTOCOL_VERSION: u32 = 1;
const MAX_REQUEST: usize = 4 * 1024 * 1024;
const MAX_RESPONSE: usize = 64 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(60);

pub fn data_dir() -> PathBuf {
    std::env::var_os("NUPHUS_WORKBENCH_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nuphus-workbench")
        })
}

fn profile_key(root: &Path) -> std::io::Result<String> {
    let root = root.canonicalize()?;
    let mut identity = root.to_string_lossy().into_owned();
    if cfg!(windows) {
        identity = identity
            .trim_start_matches(r"\\?\")
            .replace('/', "\\")
            .to_lowercase();
    }
    Ok(format!("{:x}", Sha256::digest(identity.as_bytes()))[..24].into())
}

pub fn bridge_path() -> std::io::Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .ok_or_else(|| std::io::Error::other("Missing installation directory"))?
        .join(if cfg!(windows) {
            "nuphus-workbench-mcp.exe"
        } else {
            "nuphus-workbench-mcp"
        }))
}

fn upgrade_in_progress(host: &Path) -> bool {
    let Some(parent) = host.parent() else {
        return false;
    };
    let marker = parent.join(".workbench-upgrading");
    if !marker.exists() {
        return false;
    }
    // The installer holds an exclusive writer handle. An abandoned marker after
    // cancellation is harmless once that handle has closed.
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(marker)
    {
        Ok(file) => file.try_lock().is_err(),
        Err(_) => true,
    }
}

/// Exercise the installed executable and actual MCP protocol, not just IPC reachability.
pub async fn check(executable: &Path, root: &Path) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut command = tokio::process::Command::new(executable);
    command
        .arg("serve")
        .env("NUPHUS_WORKBENCH_DATA_DIR", root)
        .env_remove("NUPHUS_WORKBENCH_URL")
        .env_remove("NUPHUS_WORKBENCH_TOKEN")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| ApiError::new("mcp_not_found", format!("无法启动 MCP 程序：{e}")))?;
    let mut input = child.stdin.take().expect("piped stdin");
    let mut output = BufReader::new(child.stdout.take().expect("piped stdout"));
    let result = tokio::time::timeout(Duration::from_secs(40), async {
        let requests = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"workbench-self-test","version":"1"}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"system_capabilities","arguments":{}}}),
        ];
        for request in requests {
            input.write_all(format!("{request}\n").as_bytes()).await.map_err(|e| transport(e.to_string()))?;
            loop {
                let mut line = String::new();
                let count = (&mut output).take(MAX_RESPONSE as u64).read_line(&mut line).await.map_err(|e| transport(e.to_string()))?;
                if count == 0 || !line.ends_with('\n') { return Err(transport("MCP 程序未返回完整协议消息")); }
                let response: Value = serde_json::from_str(&line).map_err(|_| transport("MCP 标准输出包含非协议内容"))?;
                if response["id"] != request["id"] { continue; }
                if response.get("error").is_some() || response["result"]["isError"] == true { return Err(transport(response.to_string())); }
                if request["id"] == 2 && !response["result"]["tools"].as_array().is_some_and(|tools| tools.iter().any(|t| t["name"] == "workflow_run")) {
                    return Err(transport("MCP 工具清单不完整"));
                }
                if request["id"] == 3 && response["result"]["structuredContent"]["result"]["host"]["edition"] != "workbench" {
                    return Err(transport("宿主状态不是工作台版本"));
                }
                break;
            }
            if request["id"] == 1 { input.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n").await.map_err(|e| transport(e.to_string()))?; }
        }
        Ok(())
    }).await.map_err(|_| ApiError::new("mcp_check_timeout", "MCP 完整链路测试超时，请重新启动工作台后再试。"))?;
    drop(input);
    let _ = child.kill().await;
    result
}

#[derive(Serialize, Deserialize)]
struct Hello {
    protocol: u32,
    edition: String,
}
#[derive(Serialize, Deserialize)]
struct Request {
    id: String,
    operation: String,
    args: Value,
    token: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Response {
    id: String,
    result: std::result::Result<Value, ApiError>,
}

fn transport(message: impl Into<String>) -> ApiError {
    ApiError::new("transport_error", message)
}
fn unknown() -> ApiError {
    transport("请求已发送但结果未确认。请先查询运行记录；如需重试运行，复用原 request_id，不要重复执行副作用。")
        .details(json!({"outcome_unknown":true}))
}

async fn read_frame<T: DeserializeOwned>(
    stream: &mut (impl AsyncRead + Unpin),
    limit: usize,
) -> std::io::Result<T> {
    let size = stream.read_u32_le().await? as usize;
    if size == 0 || size > limit {
        return Err(std::io::Error::other("Invalid local frame length"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}
async fn write_frame(
    stream: &mut (impl AsyncWrite + Unpin),
    bytes: &[u8],
    limit: usize,
) -> std::io::Result<()> {
    if bytes.is_empty() || bytes.len() > limit {
        return Err(std::io::Error::other("Local frame too large"));
    }
    stream.write_u32_le(bytes.len() as u32).await?;
    stream.write_all(bytes).await?;
    stream.flush().await
}

/// Binding happens before serving so the UI cannot advertise a failed listener.
pub struct Server(platform::Listener);
impl Server {
    pub fn bind(root: &Path) -> std::io::Result<Self> {
        Ok(Self(platform::Listener::bind(root)?))
    }
    pub async fn serve<H: Host>(mut self, service: Arc<Service<H>>) -> std::io::Result<()> {
        loop {
            let mut stream = self.0.accept().await?;
            let service = service.clone();
            tokio::spawn(async move {
                // One operation per connection. A client disconnect does not cancel dispatch.
                let hello = serde_json::to_vec(&Hello {
                    protocol: PROTOCOL_VERSION,
                    edition: "workbench".into(),
                })
                .expect("hello");
                let request = tokio::time::timeout(IO_TIMEOUT, async {
                    write_frame(&mut stream, &hello, MAX_RESPONSE).await?;
                    read_frame::<Request>(&mut stream, MAX_REQUEST).await
                })
                .await;
                let Ok(Ok(request)) = request else { return };
                let result = match request.token {
                    Some(token) => match service.store.authenticate(&token) {
                        Ok(principal) => {
                            service
                                .dispatch(&principal, &request.operation, request.args)
                                .await
                        }
                        Err(error) => Err(error),
                    },
                    None => {
                        service
                            .dispatch(&Principal::LocalExternal, &request.operation, request.args)
                            .await
                    }
                };
                let response = Response {
                    id: request.id,
                    result,
                };
                if let Ok(bytes) = serde_json::to_vec(&response) {
                    let bytes = if bytes.len() > MAX_RESPONSE {
                        serde_json::to_vec(&Response {
                            id: response.id,
                            result: Err(ApiError::new(
                                "response_too_large",
                                "结果过大，请缩小查询范围或按步骤读取记录。",
                            )),
                        })
                        .expect("error response")
                    } else {
                        bytes
                    };
                    let _ = tokio::time::timeout(
                        IO_TIMEOUT,
                        write_frame(&mut stream, &bytes, MAX_RESPONSE),
                    )
                    .await;
                }
            });
        }
    }
}

pub struct Client {
    root: PathBuf,
    host: PathBuf,
    token: Option<String>,
    gate: tokio::sync::Mutex<()>,
}
impl Client {
    pub fn installed(token: Option<String>) -> std::io::Result<Self> {
        let executable = std::env::current_exe()?;
        let host = executable
            .parent()
            .ok_or_else(|| std::io::Error::other("Missing installation directory"))?
            .join(if cfg!(windows) {
                "nuphus-workbench.exe"
            } else {
                "nuphus-workbench"
            });
        Ok(Self::new(data_dir(), host, token))
    }
    pub fn new(root: PathBuf, host: PathBuf, token: Option<String>) -> Self {
        Self {
            root,
            host,
            token,
            gate: tokio::sync::Mutex::new(()),
        }
    }
    async fn connect(&self) -> Result<platform::Stream> {
        let _guard = self.gate.lock().await;
        std::fs::create_dir_all(&self.root).map_err(|e| transport(e.to_string()))?;
        if upgrade_in_progress(&self.host) {
            return Err(ApiError::new(
                "upgrade_in_progress",
                "工作台正在升级，请完成安装后重新连接。",
            ));
        }
        match platform::connect(&self.root).await {
            Ok(stream) => return Ok(stream),
            Err(error) if !platform::unavailable(&error) => {
                return Err(transport(error.to_string()))
            }
            Err(_) => {}
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("mcp-start.lock"))
            .map_err(|e| transport(e.to_string()))?;
        // Keep the cross-process lock until ready. Only its owner may spawn.
        let mut owner = false;
        let mut spawned = false;
        loop {
            if let Ok(stream) = platform::connect(&self.root).await {
                return Ok(stream);
            }
            if !owner {
                owner = lock.try_lock().is_ok();
            }
            if owner && !spawned {
                // A live old-version host must be upgraded, not started repeatedly.
                let host_lock = std::fs::OpenOptions::new()
                    .create(true)
                    .truncate(false)
                    .read(true)
                    .write(true)
                    .open(self.root.join("host.lock"))
                    .map_err(|e| transport(e.to_string()))?;
                if host_lock.try_lock().is_ok() {
                    drop(host_lock);
                    if !self.host.is_file() {
                        return Err(ApiError::new(
                            "app_not_found",
                            "未找到同安装目录的工作台主程序，请重新安装完整工作台。",
                        ));
                    }
                    if upgrade_in_progress(&self.host) {
                        return Err(ApiError::new(
                            "upgrade_in_progress",
                            "工作台正在升级，请稍后重新连接。",
                        ));
                    }
                    let mut command = tokio::process::Command::new(&self.host);
                    command
                        .arg("--background")
                        .current_dir(self.host.parent().expect("host parent"))
                        .env(
                            "NUPHUS_WORKBENCH_DATA_DIR",
                            self.root
                                .canonicalize()
                                .map_err(|e| transport(e.to_string()))?,
                        )
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null());
                    #[cfg(windows)]
                    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
                                                        // Deliberately not kill_on_drop: runs and schedules belong to the tray host.
                    let mut child = command
                        .spawn()
                        .map_err(|e| ApiError::new("app_start_failed", e.to_string()))?;
                    tokio::spawn(async move {
                        let _ = child.wait().await;
                    });
                }
                spawned = true;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ApiError::new(
                    "app_start_timeout",
                    "30 秒内未连接到工作台。请确认应用已启动，并安装同一版本的主程序和 MCP 程序。",
                ));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
    pub async fn operation(&self, operation: &str, args: Value) -> Result<Value> {
        let request = Request {
            id: uuid::Uuid::new_v4().to_string(),
            operation: operation.into(),
            args,
            token: self.token.clone(),
        };
        let bytes = serde_json::to_vec(&request)?;
        if bytes.len() > MAX_REQUEST {
            return Err(ApiError::new(
                "request_too_large",
                "请求超过 4 MiB，请拆分操作。",
            ));
        }
        let mut stream = self.connect().await?;
        let hello: Hello =
            tokio::time::timeout(Duration::from_secs(5), read_frame(&mut stream, 4096))
                .await
                .map_err(|_| transport("本机连接握手超时"))?
                .map_err(|e| transport(e.to_string()))?;
        if hello.protocol != PROTOCOL_VERSION || hello.edition != "workbench" {
            return Err(ApiError::new(
                "protocol_mismatch",
                "MCP 与工作台版本不兼容，请安装同一版本后重新连接。",
            ));
        }
        // Once writing begins, never transparently retry an operation.
        let response: Response = tokio::time::timeout(IO_TIMEOUT, async {
            write_frame(&mut stream, &bytes, MAX_REQUEST).await?;
            read_frame(&mut stream, MAX_RESPONSE).await
        })
        .await
        .map_err(|_| unknown())?
        .map_err(|_| unknown())?;
        if response.id != request.id {
            return Err(unknown());
        }
        response.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_identity_is_stable_and_isolated() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        assert_eq!(
            profile_key(a.path()).unwrap(),
            profile_key(&a.path().join(".")).unwrap()
        );
        assert_ne!(
            profile_key(a.path()).unwrap(),
            profile_key(b.path()).unwrap()
        );
    }
    #[tokio::test]
    async fn malformed_frames_are_rejected_without_allocation() {
        let (mut a, mut b) = tokio::io::duplex(8);
        a.write_u32_le(u32::MAX).await.unwrap();
        assert!(read_frame::<Value>(&mut b, MAX_REQUEST).await.is_err());
    }
    #[tokio::test]
    async fn missing_application_is_actionable() {
        let dir = tempfile::tempdir().unwrap();
        let client = Client::new(dir.path().into(), dir.path().join("missing-app"), None);
        assert_eq!(
            client
                .operation("system.capabilities", json!({}))
                .await
                .unwrap_err()
                .code,
            "app_not_found"
        );
    }
    #[tokio::test]
    async fn incompatible_host_is_rejected_before_sending_operation() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = platform::Listener::bind(dir.path()).unwrap();
        let task = tokio::spawn(async move {
            let mut stream = server.accept().await.unwrap();
            write_frame(
                &mut stream,
                &serde_json::to_vec(&Hello {
                    protocol: 999,
                    edition: "workbench".into(),
                })
                .unwrap(),
                MAX_RESPONSE,
            )
            .await
            .unwrap();
            assert!(read_frame::<Request>(&mut stream, MAX_REQUEST)
                .await
                .is_err());
        });
        let client = Client::new(dir.path().into(), dir.path().join("missing-app"), None);
        assert_eq!(
            client
                .operation("workflow.run", json!({}))
                .await
                .unwrap_err()
                .code,
            "protocol_mismatch"
        );
        task.await.unwrap();
    }
    #[tokio::test]
    async fn lost_reply_is_unknown_and_never_replayed() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = platform::Listener::bind(dir.path()).unwrap();
        let task = tokio::spawn(async move {
            let mut stream = server.accept().await.unwrap();
            write_frame(
                &mut stream,
                &serde_json::to_vec(&Hello {
                    protocol: PROTOCOL_VERSION,
                    edition: "workbench".into(),
                })
                .unwrap(),
                MAX_RESPONSE,
            )
            .await
            .unwrap();
            let request: Request = read_frame(&mut stream, MAX_REQUEST).await.unwrap();
            assert_eq!(request.operation, "workflow.run");
            drop(stream);
            assert!(
                tokio::time::timeout(Duration::from_millis(300), server.accept())
                    .await
                    .is_err()
            );
        });
        let client = Client::new(dir.path().into(), dir.path().join("missing-app"), None);
        let error = client
            .operation("workflow.run", json!({"request_id":"same-id"}))
            .await
            .unwrap_err();
        assert_eq!(error.details.unwrap()["outcome_unknown"], true);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn upgrade_marker_prevents_autostart() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".workbench-upgrading"), "test").unwrap();
        let marker = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.path().join(".workbench-upgrading"))
            .unwrap();
        marker.lock().unwrap();
        let client = Client::new(dir.path().into(), dir.path().join("missing-app"), None);
        assert_eq!(
            client
                .operation("system.capabilities", json!({}))
                .await
                .unwrap_err()
                .code,
            "upgrade_in_progress"
        );
        drop(marker);
        assert_eq!(
            client
                .operation("system.capabilities", json!({}))
                .await
                .unwrap_err()
                .code,
            "app_not_found"
        );
    }
}
