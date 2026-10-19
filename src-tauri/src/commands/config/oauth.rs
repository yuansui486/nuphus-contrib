//! custom 实例通用 OAuth2 接入（Authorization Code + PKCE + 本地回调）— 命令层。
//!
//! 三个命令：
//! - [`oauth_begin`]：拉起本地回调 server 并返回授权 URL（浏览器打开由前端
//!   既有 `openExternal` 完成，本模块不碰 UI）；回调→换令牌→加密落盘→发事件。
//! - [`oauth_status`]：登录状态查询（绝不回传令牌本身）。
//! - [`oauth_logout`]：清空令牌字段。
//!
//! 令牌协议交互与磁盘读写全部委托核心层 `nuphus::config::oauth`；本层只做
//! 回调 server（axum，仅绑 127.0.0.1，模式对齐 handoff_server.rs）、随机数、
//! PKCE 与事件分发。安全不变量与核心层一致：令牌不进日志 / 事件 / 错误文案。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::Emitter;

use nuphus::config::model::ProviderOAuth;
use nuphus::config::oauth as core_oauth;

use super::toml_ops::get_config_path;

/// 同一 provider 重复 begin 时旧会话的最长残留：回调超时上限。
const CALLBACK_TIMEOUT_SECS: u64 = 300;

struct LoginSession {
    /// true = 请求关闭（被新会话顶替）。回调到达经 Notify 通道另行终止 serve。
    cancel: tokio::sync::watch::Sender<bool>,
}

fn sessions() -> &'static Mutex<HashMap<String, LoginSession>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, LoginSession>>> = OnceLock::new();
    SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 关闭该 provider 的旧登录会话（存在则返回 true）。
fn cancel_existing_session(provider: &str) -> bool {
    let existing = sessions()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(provider));
    match existing {
        Some(session) => {
            let _ = session.cancel.send(true);
            true
        }
        None => false,
    }
}

// ============================================================================
// 命令参数 / 返回 DTO
// ============================================================================

/// create/update_custom_provider 的可选 OAuth 配置入参（表单五项，camelCase）。
/// 令牌字段不在此出现——它们由登录/刷新路径独占维护。
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OauthConfigDto {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    /// 空格分隔；空 = 授权请求不带 scope。
    pub scopes: String,
    /// 缺省 true（与 ProviderOAuth TOML 缺省语义一致；旧前端调用不传该字段时同样成立）。
    #[serde(default = "dto_default_use_pkce")]
    pub use_pkce: bool,
    pub redirect_port: Option<u16>,
}

fn dto_default_use_pkce() -> bool {
    true
}

/// oauth_begin 返回：授权 URL（前端经 openExternal 打开）与本地回调端口。
#[derive(Debug, serde::Serialize)]
pub struct OauthBeginResult {
    pub authorize_url: String,
    pub port: u16,
}

/// oauth_status 返回。蛇形命名与 ProviderInfo 现有字段口径一致。
#[derive(Debug, serde::Serialize)]
pub struct OauthStatus {
    pub configured: bool,
    pub logged_in: bool,
    pub expires_at: Option<i64>,
    pub needs_login: bool,
}

/// ProviderInfo.oauth 摘要：登录状态 + 配置五项（编辑表单回显），
/// **令牌字段永不出现**（状态里也只有过期时间，没有 token 值）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct OauthSummaryDto {
    pub configured: bool,
    pub logged_in: bool,
    pub expires_at: Option<i64>,
    /// 需重新登录：配置完整但未登录，或令牌已过期且无 refresh_token 可自愈。
    pub needs_login: bool,
    // ── 配置五项（camelCase 与 OauthConfigDto 一致，前端直接回填表单）──
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scopes: String,
    pub use_pkce: bool,
    pub redirect_port: Option<u16>,
}

impl OauthSummaryDto {
    pub fn from_oauth(oauth: &ProviderOAuth) -> Self {
        let logged_in = oauth.is_logged_in();
        let configured = oauth.config_complete();
        // 与 oauth_status 同一判定：无 refresh_token 可自愈且令牌已过期 → 需重新登录
        let now = core_oauth::now_unix();
        let fresh = core_oauth::token_is_fresh(oauth.expires_at, &oauth.access_token, now);
        let can_self_heal = !oauth.refresh_token.is_empty();
        OauthSummaryDto {
            configured,
            logged_in,
            expires_at: oauth.expires_at,
            needs_login: configured && (!logged_in || (!can_self_heal && !fresh)),
            // 配置五项原样带出（表单回显）；令牌字段到此为止，不进摘要。
            authorize_url: oauth.authorize_url.clone(),
            token_url: oauth.token_url.clone(),
            client_id: oauth.client_id.clone(),
            scopes: oauth.scopes.clone(),
            use_pkce: oauth.use_pkce,
            redirect_port: oauth.redirect_port,
        }
    }
}

/// ProviderInfo 组装用的摘要读取：段未配 oauth → None。
pub fn read_oauth_summary(provider: &str) -> Option<OauthSummaryDto> {
    let path = get_config_path()?;
    read_oauth_summary_in(&path, provider)
}

fn read_oauth_summary_in(path: &std::path::Path, provider: &str) -> Option<OauthSummaryDto> {
    let oauth = core_oauth::read_oauth_segment(path, provider)?;
    let mut summary = OauthSummaryDto::from_oauth(&oauth);
    if oauth.account_ref.is_some() {
        summary.logged_in = nuphus::config::auth_state::provider_auth(path, provider).has_oauth;
        summary.needs_login = !summary.logged_in;
    }
    Some(summary)
}

// ============================================================================
// oauth_begin：本地回调 server + 授权换令牌全流程
// ============================================================================

#[tauri::command]
pub async fn oauth_begin(
    app: tauri::AppHandle,
    provider: String,
) -> Result<OauthBeginResult, String> {
    let config_path =
        get_config_path().ok_or_else(|| "配置文件未找到，请先完成模型配置".to_string())?;
    let oauth = core_oauth::read_oauth_segment(&config_path, &provider).ok_or_else(|| {
        format!("实例 {provider} 未配置 OAuth 授权（请先在表单填写授权端点等信息）")
    })?;
    if !oauth.config_complete() {
        return Err("OAuth 配置不完整：授权端点、令牌端点与 Client ID 均为必填项".to_string());
    }

    // 回调端口：配置指定 → 绑定该端口；未指定 → OS 分配空闲端口。
    // 先绑定再返回，保证返回的 port 与实际监听一致。
    let listener = match oauth.redirect_port {
        Some(port) => tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("回调端口 {port} 绑定失败（可能被占用）: {e}"))?,
        None => tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| format!("本地回调端口分配失败: {e}"))?,
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取回调端口失败: {e}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let attempt = core_oauth::prepare_login(&config_path, &provider, redirect_uri)?;
    let authorize_url = attempt.authorize_url.clone();
    cancel_existing_session(&provider);
    let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
    sessions()
        .lock()
        .map_err(|_| "Could not register OAuth session")?
        .insert(provider.clone(), LoginSession { cancel: cancel_tx });
    let callback =
        super::oauth_callback::Callback::start(listener, "/callback", attempt.state.clone());
    tokio::spawn(async move {
        let params = callback
            .wait(
                async {
                    let _ = cancel_rx.changed().await;
                },
                std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS),
            )
            .await;
        if *cancel_rx.borrow() {
            return;
        }
        let result = match params {
            Err(e) => Err(e),
            Ok(params) => {
                let path = config_path;
                let name = provider.clone();
                let cancelled = cancel_rx.clone();
                tokio::task::spawn_blocking(move || {
                    let (client_id, code) = attempt.callback(&params)?;
                    let (tokens, _) = attempt.exchange(&client_id, &code)?;
                    let _sessions = sessions()
                        .lock()
                        .map_err(|_| "Could not lock OAuth sessions")?;
                    if *cancelled.borrow() {
                        return Err("OAuth sign-in cancelled".into());
                    }
                    core_oauth::write_oauth_session(&path, &name, &client_id, tokens)
                })
                .await
                .unwrap_or_else(|_| Err("OAuth sign-in task failed".into()))
            }
        };
        if *cancel_rx.borrow() {
            return;
        }
        if let Ok(mut sessions) = sessions().lock() {
            if *cancel_rx.borrow() {
                return;
            }
            sessions.remove(&provider);
        }
        emit_result(&app, &provider, result.is_ok(), result.err());
    });
    Ok(OauthBeginResult {
        authorize_url,
        port,
    })
}

fn emit_result(app: &tauri::AppHandle, provider: &str, ok: bool, error: Option<String>) {
    let payload = serde_json::json!({ "provider": provider, "ok": ok, "error": error });
    if let Err(e) = app.emit("oauth-login-result", payload) {
        tracing::warn!("[oauth] {provider} 结果事件发送失败: {e}");
    }
    tracing::info!(
        "[oauth] {provider} 授权登录{}",
        if ok { "成功" } else { "失败" }
    );
}

// ============================================================================
// oauth_status / oauth_logout
// ============================================================================

/// 查询登录状态。读盘 + 纯判定，不触发网络刷新。
#[tauri::command(async)]
pub fn oauth_status(provider: String) -> Result<OauthStatus, String> {
    let path = get_config_path().ok_or_else(|| "配置文件未找到".to_string())?;
    oauth_status_in(&path, &provider)
}

fn oauth_status_in(path: &std::path::Path, provider: &str) -> Result<OauthStatus, String> {
    let Some(oauth) = core_oauth::read_oauth_segment(path, provider) else {
        return Ok(OauthStatus {
            configured: false,
            logged_in: false,
            expires_at: None,
            needs_login: false,
        });
    };
    let now = core_oauth::now_unix();
    if oauth.account_ref.is_some() {
        let logged_in = nuphus::config::auth_state::provider_auth(path, provider).has_oauth;
        return Ok(OauthStatus {
            configured: oauth.config_complete(),
            logged_in,
            expires_at: None,
            needs_login: !logged_in,
        });
    }
    let logged_in = oauth.is_logged_in();
    let configured = oauth.config_complete();
    let fresh = core_oauth::token_is_fresh(oauth.expires_at, &oauth.access_token, now);
    let can_self_heal = !oauth.refresh_token.is_empty();
    Ok(OauthStatus {
        configured,
        logged_in,
        expires_at: oauth.expires_at,
        // 无法自愈才需要重新登录：未登录，或已过期且无 refresh_token
        needs_login: configured && (!logged_in || (!can_self_heal && !fresh)),
    })
}

/// 登出：清空三个令牌字段落盘（配置五项保留）。段未配 oauth → 幂等成功。
#[tauri::command(async)]
pub fn oauth_logout(provider: String) -> Result<(), String> {
    cancel_existing_session(&provider);
    let path = get_config_path().ok_or_else(|| "配置文件未找到".to_string())?;
    core_oauth::clear_oauth_tokens(&path, &provider)?;
    tracing::info!("[oauth] {provider} 已登出（令牌已清空）");
    Ok(())
}
