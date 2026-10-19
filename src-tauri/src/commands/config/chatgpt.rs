//! Desktop-only SIWC commands. Tokens, PKCE and nonce never cross IPC.
use nuphus::config::chatgpt::{self, AccountInfo, Model};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tokio::sync::{Mutex, Notify};
static LOGIN_CANCELLED: AtomicBool = AtomicBool::new(false);

fn login_guard() -> &'static Mutex<()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(()))
}
fn cancel_signal() -> &'static Notify {
    static SIGNAL: OnceLock<Notify> = OnceLock::new();
    SIGNAL.get_or_init(Notify::new)
}
fn path() -> Result<std::path::PathBuf, String> {
    super::get_config_path().ok_or_else(|| "Provider configuration not found".into())
}

#[tauri::command(async)]
pub fn chatgpt_accounts() -> Result<Vec<AccountInfo>, String> {
    chatgpt::accounts(&path()?)
}

#[tauri::command(async)]
pub fn chatgpt_cancel_login() {
    LOGIN_CANCELLED.store(true, Ordering::Release);
    cancel_signal().notify_waiters();
}

#[tauri::command]
pub async fn chatgpt_login(account_id: Option<String>) -> Result<AccountInfo, String> {
    let _guard = login_guard()
        .try_lock()
        .map_err(|_| "A ChatGPT sign-in is already in progress")?;
    LOGIN_CANCELLED.store(false, Ordering::Release);
    let cancelled = cancel_signal().notified();
    tokio::pin!(cancelled);
    cancelled.as_mut().enable();
    let config = path()?;
    // Bind before opening the browser; the exact same URI is used in exchange.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| "Could not start the ChatGPT loopback callback")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Could not read callback port")?
        .port();
    let prepare_path = config.clone();
    let attempt = tokio::task::spawn_blocking(move || {
        chatgpt::prepare_login(&prepare_path, port, account_id.as_deref())
    })
    .await
    .map_err(|_| "Could not prepare ChatGPT sign-in")??;
    let callback =
        super::oauth_callback::Callback::start(listener, "/auth/callback", attempt.state.clone());
    crate::commands::external::open_external(attempt.authorize_url.clone())
        .map_err(|_| "Could not open your browser for ChatGPT sign-in")?;
    let params = callback
        .wait(
            async {
                cancelled.await;
            },
            std::time::Duration::from_secs(300),
        )
        .await?;
    tokio::task::spawn_blocking(move || {
        chatgpt::finish_login(&config, attempt, params, &LOGIN_CANCELLED)
    })
    .await
    .map_err(|_| "ChatGPT sign-in task failed")?
}

/** Blocking HTTP and token refresh must run outside Tauri's async runtime workers. */
#[tauri::command]
pub async fn chatgpt_models(account_id: String) -> Result<Vec<Model>, String> {
    let config = path()?;
    tokio::task::spawn_blocking(move || chatgpt::sync_models(&config, &account_id))
        .await
        .map_err(|_| "ChatGPT model synchronization task failed")?
}

/** 退出后保留未配置的当前连接，避免下一次发送自动选择另一个计费连接。 */
#[tauri::command]
pub async fn chatgpt_logout(
    state: tauri::State<'_, crate::state::AppState>,
    account_id: String,
) -> Result<bool, String> {
    let _guard = login_guard()
        .try_lock()
        .map_err(|_| "Cancel the active ChatGPT sign-in before signing out")?;
    let config = path()?;
    let provider = chatgpt::accounts(&config)?
        .into_iter()
        .find(|a| a.account_id == account_id)
        .map(|a| a.provider);
    let revoked = tokio::task::spawn_blocking(move || chatgpt::logout(&config, &account_id))
        .await
        .map_err(|_| "ChatGPT sign-out task failed")??;
    if let Ok(mut runtime) = state.runtime.lock() {
        if let Some(current) = runtime.llm_config.as_mut() {
            if Some(&current.provider) == provider.as_ref() {
                current.api_key.clear();
                current.model.clear();
            }
        }
    }
    Ok(revoked)
}
