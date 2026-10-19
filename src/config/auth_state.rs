/** 无凭据解密的配置状态快照；文件变化时才重新解析，发送前检查不触发 DPAPI。 */
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

#[derive(Clone, Copy, Default)]
pub struct AuthState {
    pub has_key: bool,
    pub has_oauth: bool,
    pub oauth_configured: bool,
}

#[derive(Default)]
struct Snapshot {
    stamp: Option<(SystemTime, u64)>,
    credential_stamps: HashMap<PathBuf, Option<(SystemTime, u64)>>,
    providers: HashMap<String, AuthState>,
}

/** 统一识别 OAuth 配置；不依赖 provider 名称，也不读取或解密令牌内容。 */
pub fn oauth_entry(entry: &toml::Value) -> bool {
    let mut doc = toml::Value::Table(toml::map::Map::from_iter([(
        "providers".into(),
        toml::Value::Array(vec![entry.clone()]),
    )]));
    super::normalize_provider_document(&mut doc);
    doc["providers"][0].get("oauth").is_some_and(|oauth| {
        oauth
            .get("account_ref")
            .and_then(toml::Value::as_str)
            .is_some_and(|id| !id.is_empty())
            || oauth
                .get("access_token")
                .and_then(toml::Value::as_str)
                .is_some_and(|v| !v.is_empty())
    })
}

impl Snapshot {
    fn read(&mut self, path: &Path) {
        let stamp = std::fs::metadata(path)
            .ok()
            .and_then(|m| Some((m.modified().ok()?, m.len())));
        if stamp.is_some()
            && stamp == self.stamp
            && self
                .credential_stamps
                .iter()
                .all(|(p, stamp)| file_stamp(p) == *stamp)
        {
            return;
        }
        self.providers.clear();
        self.credential_stamps.clear();
        self.stamp = None;
        if let Ok(text) = std::fs::read_to_string(path) {
            if let Ok(mut doc) = text.parse::<toml::Value>() {
                super::normalize_provider_document(&mut doc);
                for entry in doc
                    .get("providers")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(name) = entry.get("name").and_then(toml::Value::as_str) {
                        self.providers.insert(
                            name.into(),
                            AuthState {
                                oauth_configured: entry.get("oauth").is_some(),
                                has_key: entry
                                    .get("api_key")
                                    .and_then(toml::Value::as_str)
                                    .is_some_and(|v| !v.is_empty()),
                                has_oauth: session_available(
                                    path,
                                    entry,
                                    &mut self.credential_stamps,
                                ),
                            },
                        );
                    }
                }
                self.stamp = stamp;
            }
        }
    }
}

fn file_stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/** 只检查加密令牌是否存在；凭据文件变化才刷新快照，不解密任何账号。 */
fn session_available(
    config: &Path,
    entry: &toml::Value,
    stamps: &mut HashMap<PathBuf, Option<(SystemTime, u64)>>,
) -> bool {
    let Some(oauth) = entry.get("oauth") else {
        return false;
    };
    let Some(account) = oauth.get("account_ref").and_then(toml::Value::as_str) else {
        return oauth_entry(entry);
    };
    let Some(file) = oauth.get("credential_file").and_then(toml::Value::as_str) else {
        return false;
    };
    if file.is_empty()
        || file.contains(['/', '\\', ':'])
        || Path::new(file).components().count() != 1
    {
        return false;
    }
    let path = config.with_file_name(file);
    stamps.insert(path.clone(), file_stamp(&path));
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .is_some_and(|doc| {
            doc["accounts"].as_array().is_some_and(|accounts| {
                accounts.iter().any(|a| {
                    a["client_id"] == account
                        && a["access_token"]
                            .as_str()
                            .is_some_and(|token| !token.is_empty())
                })
            })
        })
}

pub fn provider_auth(path: &Path, provider: &str) -> AuthState {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Snapshot>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let snapshot = cache.entry(path.to_path_buf()).or_default();
    snapshot.read(path);
    snapshot
        .providers
        .get(provider)
        .copied()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_changes_invalidate_state_even_when_provider_reference_remains() {
        let dir =
            std::env::temp_dir().join(format!("nuphus-auth-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("中文配置.toml");
        std::fs::write(&path, "[[providers]]\nname='arbitrary'\n[providers.oauth]\naccount_ref='id'\ncredential_file='accounts.json'\n").unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.read(&path);
        assert!(!snapshot.providers["arbitrary"].has_oauth);
        std::fs::write(
            dir.join("accounts.json"),
            r#"{"accounts":[{"client_id":"id","access_token":"enc:opaque"}]}"#,
        )
        .unwrap();
        snapshot.read(&path);
        assert!(snapshot.providers["arbitrary"].has_oauth);
        std::fs::write(
            dir.join("accounts.json"),
            r#"{"accounts":[{"client_id":"id","access_token":""}]}"#,
        )
        .unwrap();
        snapshot.read(&path);
        assert!(!snapshot.providers["arbitrary"].has_oauth);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unchanged_metadata_reuses_snapshot_and_deleted_provider_is_not_authenticated() {
        let dir = std::env::temp_dir().join(format!("nuphus-auth-state-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("中文配置.toml");
        std::fs::write(
            &path,
            "[[providers]]\nname='arbitrary-name'\n[providers.oauth]\naccess_token='enc:opaque'\n",
        )
        .unwrap();
        let mut snapshot = Snapshot::default();
        snapshot.read(&path);
        assert!(snapshot.providers["arbitrary-name"].has_oauth);
        snapshot
            .providers
            .insert("cache-sentinel".into(), AuthState::default());
        snapshot.read(&path);
        assert!(snapshot.providers.contains_key("cache-sentinel"));
        std::fs::write(&path, "providers=[]\n").unwrap();
        snapshot.read(&path);
        assert!(snapshot.providers.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
