/** 独立 OAuth 账号存储：文件锁保护轮换令牌，写入使用原子替换和系统加密。 */
use super::oauth_flow::Tokens;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Store {
    pub(crate) host_id: String,
    pub(crate) accounts: Vec<Account>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Account {
    pub(crate) client_id: String,
    pub(crate) subject: String,
    pub(crate) email: String,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) id_token: String,
    pub(crate) scope: String,
    pub(crate) expires_at: i64,
    #[serde(default)]
    pub(crate) saved_provider: Option<toml::Value>,
}

impl Account {
    pub(crate) fn update(&mut self, tokens: Tokens) -> Result<(), String> {
        if tokens.expires_in <= 0 {
            return Err("OAuth account token is missing a valid expiry".into());
        }
        let scope = tokens.scope.unwrap_or_else(|| self.scope.clone());
        // Save rotated credentials even when permission was removed; subsequent
        // requests fail closed at require_scope rather than reuse stale tokens.
        self.scope = scope;
        self.access_token = protect(&tokens.access_token)?;
        if let Some(refresh) = tokens.refresh_token {
            self.refresh_token = protect(&refresh)?;
        }
        if let Some(id) = tokens.id_token {
            self.id_token = protect(&id)?;
        }
        self.expires_at = super::oauth::now_unix().saturating_add(tokens.expires_in);
        Ok(())
    }
}

pub(crate) fn protect(token: &str) -> Result<String, String> {
    let encrypted = crate::cookies::encrypt_secret(token);
    #[cfg(windows)]
    if !token.is_empty() && !encrypted.starts_with("enc:") {
        return Err("Could not protect OAuth credentials with Windows DPAPI".into());
    }
    // Unix credential files are owner-only, including on replacement.
    Ok(encrypted)
}
pub(crate) fn reveal(token: &str) -> Result<String, String> {
    crate::cookies::decrypt_secret(token)
        .ok_or_else(|| "Cannot decrypt OAuth credentials; sign in again".into())
}

pub(crate) struct LockedStore {
    _lock: File,
    path: PathBuf,
    pub(crate) data: Store,
}
impl LockedStore {
    pub(crate) fn open(config: &Path, filename: &str) -> Result<Self, String> {
        let dir = config
            .parent()
            .ok_or("Missing OAuth configuration directory")?;
        if filename.is_empty()
            || Path::new(filename).components().count() != 1
            || filename.contains(['/', '\\', ':'])
        {
            return Err("Invalid credential filename".into());
        }
        let path = dir.join(filename);
        let mut options = std::fs::OpenOptions::new();
        options.create(true).read(true).write(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(path.with_extension("lock"))
            .map_err(|_| "Could not open OAuth credential lock")?;
        lock.lock_exclusive()
            .map_err(|_| "Could not lock OAuth credentials")?;
        let data = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "Invalid OAuth credential file; existing file was preserved")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Store::default(),
            Err(_) => return Err("Could not read OAuth credentials".into()),
        };
        Ok(Self {
            _lock: lock,
            path,
            data,
        })
    }
    pub(crate) fn save(&self) -> Result<(), String> {
        // Enforce permissions before the shared atomic writer copies them.
        #[cfg(unix)]
        if self.path.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "Could not protect OAuth credential file")?;
        }
        let json = serde_json::to_string_pretty(&self.data)
            .map_err(|_| "Could not encode OAuth credentials")?;
        crate::config::write_provider_config(&self.path, &json)
            .map_err(|_| "Could not save OAuth credentials".into())
    }
    pub(crate) fn account_mut(&mut self, id: &str) -> Result<&mut Account, String> {
        self.data
            .accounts
            .iter_mut()
            .find(|a| a.client_id == id)
            .ok_or_else(|| "OAuth account not found".into())
    }
}

pub(crate) fn fresh_token_with(
    config: &Path,
    id: &str,
    oauth: &super::ProviderOAuth,
    refresh_tokens: impl FnOnce(&str, &str) -> Result<Tokens, String>,
) -> Result<String, String> {
    let mut store = LockedStore::open(config, &oauth.credential_file)?;
    let account = store.account_mut(id)?;
    super::oauth_flow::require_scopes(oauth, &account.scope)?;
    let access = reveal(&account.access_token)?;
    if super::oauth::token_is_fresh(Some(account.expires_at), &access, super::oauth::now_unix()) {
        return Ok(access);
    }
    let refresh = reveal(&account.refresh_token)?;
    if refresh.is_empty() {
        return Err("Sign in to continue".into());
    }
    let tokens = refresh_tokens(&account.client_id, &refresh)?;
    account.update(tokens)?;
    let access = reveal(&account.access_token)?;
    let scope = account.scope.clone();
    store.save()?;
    super::oauth_flow::require_scopes(oauth, &scope)?;
    Ok(access)
}
