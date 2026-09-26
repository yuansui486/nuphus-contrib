//! Native credential storage and the one authorization lifecycle per tray host.
use super::*;
use nuphus_workbench::product_auth::{Authority, Vault};
use std::io::Write;
use std::path::Path;

struct NativeVault {
    root: PathBuf,
}
fn storage_error() -> ApiError {
    ApiError::new(
        "auth_storage_error",
        "无法访问安全凭据存储，请检查系统权限或解锁钥匙串。",
    )
}
fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|_| storage_error())?;
    file.write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|_| storage_error())?;
    drop(file);
    std::fs::rename(&temp, path).map_err(|_| storage_error())?;
    Ok(())
}
impl NativeVault {
    #[cfg(windows)]
    fn seal(&self, value: &str) -> Result<String> {
        super::schedule_secrets::seal(value).map_err(|_| storage_error())
    }
    #[cfg(windows)]
    fn open(&self, value: &str) -> Result<String> {
        super::schedule_secrets::open(value).map_err(|_| storage_error())
    }
    #[cfg(target_os = "linux")]
    fn key(&self, create: bool) -> Result<ring::aead::LessSafeKey> {
        use ring::rand::SecureRandom;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        let path = self.root.join("session.key");
        if create && !path.exists() {
            let mut bytes = [0; 32];
            ring::rand::SystemRandom::new()
                .fill(&mut bytes)
                .map_err(|_| storage_error())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|_| storage_error())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| storage_error())?;
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| storage_error())?;
        if !metadata.is_file()
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid()
                != std::fs::metadata(&self.root)
                    .map_err(|_| storage_error())?
                    .uid()
        {
            return Err(storage_error());
        }
        let bytes = std::fs::read(path).map_err(|_| storage_error())?;
        Ok(ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, &bytes)
                .map_err(|_| storage_error())?,
        ))
    }
    #[cfg(target_os = "linux")]
    fn seal(&self, value: &str) -> Result<String> {
        use base64::Engine;
        use ring::{aead, rand::SecureRandom};
        let mut nonce = [0; 12];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| storage_error())?;
        let mut body = value.as_bytes().to_vec();
        self.key(true)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(b"lingque-auth-v1"),
                &mut body,
            )
            .map_err(|_| storage_error())?;
        let mut encoded = nonce.to_vec();
        encoded.extend(body);
        Ok(format!(
            "lqa:v1:{}",
            base64::engine::general_purpose::STANDARD.encode(encoded)
        ))
    }
    #[cfg(target_os = "linux")]
    fn open(&self, value: &str) -> Result<String> {
        use base64::Engine;
        use ring::aead;
        let mut data = base64::engine::general_purpose::STANDARD
            .decode(value.strip_prefix("lqa:v1:").ok_or_else(storage_error)?)
            .map_err(|_| storage_error())?;
        if data.len() < 28 {
            return Err(storage_error());
        }
        let nonce: [u8; 12] = data[..12].try_into().map_err(|_| storage_error())?;
        let key = self.key(false)?;
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(b"lingque-auth-v1"),
                &mut data[12..],
            )
            .map_err(|_| storage_error())?;
        String::from_utf8(plain.to_vec()).map_err(|_| storage_error())
    }
    #[cfg(target_os = "macos")]
    fn account(&self) -> String {
        use sha2::{Digest, Sha256};
        format!(
            "{:x}",
            Sha256::digest(self.root.to_string_lossy().as_bytes())
        )
    }
}
impl Vault for NativeVault {
    fn load(&self) -> Result<Option<String>> {
        if self.root.join("signed-out").exists() {
            return Ok(None);
        }
        #[cfg(target_os = "macos")]
        {
            match security_framework::passwords::get_generic_password(
                "lingque.product-session",
                &self.account(),
            ) {
                Ok(value) => String::from_utf8(value)
                    .map(Some)
                    .map_err(|_| storage_error()),
                Err(error) if error.code() == -25300 => Ok(None),
                Err(_) => Err(storage_error()),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            match std::fs::read_to_string(self.root.join("session.sealed")) {
                Ok(sealed) => self.open(&sealed).map(Some).map_err(|_| storage_error()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(storage_error()),
            }
        }
    }
    fn save(&self, value: &str) -> Result<()> {
        #[cfg(target_os = "macos")]
        security_framework::passwords::set_generic_password(
            "lingque.product-session",
            &self.account(),
            value.as_bytes(),
        )
        .map_err(|_| storage_error())?;
        #[cfg(not(target_os = "macos"))]
        {
            let sealed = self.seal(value)?;
            atomic_write(&self.root.join("session.sealed"), sealed.as_bytes())?;
        }
        if self.root.join("signed-out").exists() {
            std::fs::remove_file(self.root.join("signed-out")).map_err(|_| storage_error())?;
        }
        Ok(())
    }
    fn clear(&self) -> Result<()> {
        // This survives a crash or a credential deletion failure; never resurrect a logout.
        let marked = atomic_write(&self.root.join("signed-out"), b"signed-out");
        #[cfg(target_os = "macos")]
        let deleted = match security_framework::passwords::delete_generic_password(
            "lingque.product-session",
            &self.account(),
        ) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(_) => Err(storage_error()),
        };
        #[cfg(not(target_os = "macos"))]
        let deleted = match std::fs::remove_file(self.root.join("session.sealed")) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(storage_error()),
        };
        // Either deletion or the durable tombstone prevents restoration.
        // Try both even when one path is unwritable/locked.
        if marked.is_ok() || deleted.is_ok() {
            Ok(())
        } else {
            Err(storage_error())
        }
    }
}

pub fn create(app: &AppHandle) -> Result<Arc<Authority>> {
    let root = nuphus::profile::workbench_data_dir().join("lingque-auth");
    std::fs::create_dir_all(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    }
    let path = root.join("device-id");
    let id = if path.exists() {
        let id = std::fs::read_to_string(&path)?;
        uuid::Uuid::parse_str(&id).map_err(|_| {
            ApiError::new(
                "invalid_device_id",
                "设备标识损坏，请恢复原配置；不能自动更换设备身份。",
            )
        })?;
        id
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        atomic_write(&path, id.as_bytes())?;
        id
    };
    let name = sysinfo::System::host_name()
        .unwrap_or_else(|| format!("灵雀 {}", std::env::consts::OS))
        .chars()
        .take(128)
        .collect();
    let version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| app.package_info().version.to_string());
    Ok(Arc::new(Authority::new(
        Arc::new(NativeVault { root }),
        id,
        name,
        version,
    )?))
}

pub fn start(app: AppHandle, authority: Arc<Authority>) {
    tauri::async_runtime::spawn(async move {
        let mut previous = None;
        let mut previous_wall = chrono::Utc::now().timestamp();
        loop {
            let wall = chrono::Utc::now().timestamp();
            let resumed = wall - previous_wall > 5;
            previous_wall = wall;
            // Check local expiry before a potentially slow network request.
            let current = authority.status();
            let identity = if current.authorized {
                current.epoch.clone()
            } else {
                None
            };
            if identity != previous {
                if previous.is_some() {
                    stop_work(app.clone()).await;
                }
                previous = identity;
                if current.authorized {
                    let _ = ensure_default(&app);
                }
            }
            let _ = app.emit("lingque-auth-changed", &current);
            let a = authority.clone();
            // The Authority is single-flight; the monitor continues cancelling at deadlines.
            tauri::async_runtime::spawn(async move {
                let _ = a.heartbeat(resumed).await;
            });
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

fn ensure_default(app: &AppHandle) -> Result<()> {
    use sha2::{Digest, Sha256};
    let Some(state) = app.try_state::<WorkbenchState>() else {
        return Ok(());
    };
    let identity = state.authority.require()?;
    if state.service.store.projects()?.is_empty()
        && state.service.store.unclaimed_projects()?.is_empty()
    {
        let key = format!(
            "{:x}",
            Sha256::digest(identity.subject.tenant_id.as_bytes())
        );
        let directory = state
            .service
            .store
            .root()
            .join("tenants")
            .join(key)
            .join("workspace");
        std::fs::create_dir_all(&directory)?;
        state
            .service
            .store
            .register_project(&directory, "工作空间")?;
    }
    Ok(())
}

async fn stop_work(app: AppHandle) {
    let Some(state) = app.try_state::<WorkbenchState>() else {
        return;
    };
    if let Ok(flags) = state.generations.lock() {
        for flag in flags.values() {
            flag.store(true, Ordering::SeqCst);
        }
    }
    if let Ok(flags) = state.service.host.cancelled.lock() {
        for flag in flags.values() {
            flag.store(true, Ordering::SeqCst);
        }
    }
    let workflows = state
        .service
        .host
        .active_workflows
        .lock()
        .map(|items| items.values().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let native = app.state::<crate::state::AppState>();
    let engine = native.workflow_engine.read().await;
    for workflow in workflows {
        engine.cancel_workflow(&workflow).await;
    }
}

#[tauri::command]
pub async fn lingque_auth(
    app: AppHandle,
    action: String,
    tenant_code: Option<String>,
    username: Option<String>,
    password: Option<String>,
    expected_epoch: Option<String>,
    project_ids: Option<Vec<String>>,
) -> Result<Value> {
    let state = app
        .try_state::<WorkbenchState>()
        .ok_or_else(|| ApiError::new("edition_unavailable", "请启动灵雀版本"))?;
    let auth = &state.authority;
    match action.as_str() {
        "status" => Ok(serde_json::to_value(auth.status())?),
        "login" => Ok(serde_json::to_value(
            auth.login(
                tenant_code.as_deref().unwrap_or(""),
                username.as_deref().unwrap_or(""),
                password.as_deref().unwrap_or(""),
            )
            .await?,
        )?),
        "verify" => Ok(serde_json::to_value(auth.heartbeat(true).await?)?),
        "logout" => {
            // logout clears local state before its first await.
            let logout = auth.logout();
            let stop = async {
                tokio::task::yield_now().await;
                stop_work(app.clone()).await;
            };
            let (result, ()) = tokio::join!(logout, stop);
            Ok(serde_json::to_value(result?)?)
        }
        "unclaimed" => {
            auth.require()?;
            ensure_default(&app)?;
            Ok(serde_json::to_value(
                state.service.store.unclaimed_projects()?,
            )?)
        }
        "claim" => {
            let expected = expected_epoch
                .ok_or_else(|| ApiError::new("invalid_params", "请重新读取待确认项目。"))?;
            auth.require_epoch(&expected)?;
            for project in project_ids
                .ok_or_else(|| ApiError::new("invalid_params", "请选择已确认归属的项目。"))?
            {
                auth.require_epoch(&expected)?;
                state.service.store.claim_project(&project)?;
            }
            Ok(json!({"claimed":true}))
        }
        _ => Err(ApiError::new("unknown_operation", "未知登录操作")),
    }
}

pub fn public_command(command: &str) -> bool {
    matches!(
        command,
        "lingque_auth"
            | "finish_startup"
            | "splash_status_update"
            | "splash_bootstrap_status"
            | "get_app_version"
            | "get_changelog"
            | "toggle_main_window_topmost"
            | "exit_app"
            | "hide_main_window"
    )
}

pub fn guard_command(command: &str) -> std::result::Result<(), String> {
    if public_command(command) {
        return Ok(());
    }
    nuphus::profile::require_product()?;
    // Workbench uses versioned tenant storage, never the legacy unscoped workflow/chat store.
    if (command.starts_with("wf_")
        && !matches!(command, "wf_get_step_schema" | "wf_get_tool_schema"))
        || matches!(
            command,
            "send_message_cmd"
                | "handoff_ensure"
                | "agent_init"
                | "mobile_server_start"
                | "mobile_server_ensure"
        )
    {
        return Err("灵雀请通过工作台项目接口使用该能力。".into());
    }
    Ok(())
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;
    #[test]
    fn product_vault_encrypts_and_logout_tombstone_survives_failed_deletion() {
        let root = tempfile::tempdir().unwrap();
        let vault = NativeVault {
            root: root.path().into(),
        };
        let secret = "test-only-product-session";
        vault.save(secret).unwrap();
        assert_eq!(vault.load().unwrap().as_deref(), Some(secret));
        let sealed = std::fs::read_to_string(root.path().join("session.sealed")).unwrap();
        assert!(!sealed.contains(secret));
        vault.clear().unwrap();
        // Simulate a credential file left behind by a failed delete/crash.
        std::fs::write(root.path().join("session.sealed"), &sealed).unwrap();
        assert!(vault.load().unwrap().is_none());
        vault.save(secret).unwrap();
        assert_eq!(vault.load().unwrap().as_deref(), Some(secret));
        std::fs::write(
            root.path().join("session.sealed"),
            "plaintext-must-not-work",
        )
        .unwrap();
        assert!(vault.load().is_err());
    }
    #[test]
    fn login_allowlist_never_includes_business_commands() {
        for command in [
            "workbench_call",
            "execute_tool",
            "wf_run",
            "workbench_generate",
            "get_current_config",
            "mobile_server_start",
        ] {
            assert!(!public_command(command));
        }
        assert!(public_command("lingque_auth"));
    }
}
