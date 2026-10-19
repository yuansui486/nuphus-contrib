use super::protocol::{self, LoginAttempt, Model, Tokens, API_BASE, CREDENTIAL_FILE};
#[cfg(test)]
use crate::config::oauth_accounts::protect;
use crate::config::oauth_accounts::{reveal, Account, LockedStore};
use serde::Serialize;
use std::{collections::HashMap, path::Path};
#[derive(Serialize)]
pub struct AccountInfo {
    pub account_id: String,
    pub provider: String,
    pub label: String,
    pub logged_in: bool,
}

impl Account {
    fn info(&self) -> AccountInfo {
        AccountInfo {
            account_id: self.client_id.clone(),
            provider: provider_name(&self.client_id),
            // Client IDs distinguish same-email workspace registrations.
            label: format!(
                "{} · {}",
                if self.email.is_empty() {
                    "ChatGPT"
                } else {
                    &self.email
                },
                self.client_id
            ),
            logged_in: !self.access_token.is_empty(),
        }
    }
}

pub fn accounts(config: &Path) -> Result<Vec<AccountInfo>, String> {
    Ok(LockedStore::open(config, CREDENTIAL_FILE)?
        .data
        .accounts
        .iter()
        .map(Account::info)
        .collect())
}

pub fn prepare_login(
    config: &Path,
    port: u16,
    account: Option<&str>,
) -> Result<LoginAttempt, String> {
    let mut store = LockedStore::open(config, CREDENTIAL_FILE)?;
    if store.data.host_id.is_empty() {
        store.data.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        store.save()?;
    }
    let subject = if let Some(id) = account {
        Some(store.account_mut(id)?.subject.clone())
    } else {
        None
    };
    Ok(protocol::login_attempt(
        &store.data.host_id,
        port,
        account,
        subject,
    ))
}

pub fn finish_login(
    config: &Path,
    attempt: LoginAttempt,
    params: HashMap<String, String>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<AccountInfo, String> {
    let (client_id, code) = attempt.callback(&params)?;
    if !client_id.starts_with("oaiapp_") {
        return Err("Invalid ChatGPT registration ID".into());
    }
    let (tokens, identity) = attempt.exchange(&client_id, &code)?;
    let identity = identity.ok_or("ChatGPT requires a verified identity")?;
    if tokens.expires_in <= 0 {
        return Err("ChatGPT did not return a token expiry".into());
    }
    let mut store = LockedStore::open(config, CREDENTIAL_FILE)?;
    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
        return Err("ChatGPT sign-in cancelled".into());
    }
    if let Some(existing) = store
        .data
        .accounts
        .iter()
        .find(|a| a.client_id == client_id)
    {
        if existing.subject != identity.sub {
            return Err("ChatGPT registration identity changed".into());
        }
    }
    let mut account = Account {
        client_id: client_id.clone(),
        subject: identity.sub,
        email: identity.email.unwrap_or_default(),
        access_token: String::new(),
        refresh_token: String::new(),
        id_token: String::new(),
        scope: String::new(),
        expires_at: 0,
        saved_provider: store
            .data
            .accounts
            .iter()
            .find(|a| a.client_id == client_id)
            .and_then(|a| a.saved_provider.clone()),
    };
    account.update(tokens)?;
    let info = account.info();
    store
        .data
        .accounts
        .retain(|a| a.client_id != account.client_id);
    store.data.accounts.push(account);
    store.save()?;
    Ok(info)
}

pub fn fresh_token(config: &Path, id: &str) -> Result<String, String> {
    fresh_token_with(config, id, protocol::refresh)
}

fn fresh_token_with(
    config: &Path,
    id: &str,
    refresh: impl FnOnce(&str, &str) -> Result<Tokens, String>,
) -> Result<String, String> {
    crate::config::oauth_accounts::fresh_token_with(
        config,
        id,
        &protocol::oauth_config("", Some(id)),
        refresh,
    )
}

/// Returns false when local sign-out succeeded but remote revocation was not confirmed.
pub fn logout(config: &Path, id: &str) -> Result<bool, String> {
    let mut store = LockedStore::open(config, CREDENTIAL_FILE)?;
    let account = store.account_mut(id)?;
    let refresh = reveal(&account.refresh_token).unwrap_or_default();
    let revoked = refresh.is_empty() || protocol::revoke(&account.client_id, &refresh);
    account.access_token.clear();
    account.refresh_token.clear();
    account.id_token.clear();
    account.expires_at = 0;
    let provider = provider_name(id);
    store.save()?;
    let saved = detach_provider(config, &provider)?;
    let account = store.account_mut(id)?;
    account.saved_provider = saved.or(account.saved_provider.take());
    store.save()?;
    Ok(revoked)
}

/** 移除已退出账号的模型及精确绑定，保留其他服务商的同名模型绑定。 */
fn detach_provider(config: &Path, provider: &str) -> Result<Option<toml::Value>, String> {
    let _guard = crate::config::lock_provider_config();
    if !config.exists() {
        return Ok(None);
    }
    let mut doc: toml::Value = std::fs::read_to_string(config)
        .map_err(|_| "Could not read provider configuration")?
        .parse()
        .map_err(|_| "Invalid provider configuration")?;
    let mut saved = None;
    crate::config::normalize_provider_document(&mut doc);
    if let Some(entries) = doc.get_mut("providers").and_then(toml::Value::as_array_mut) {
        saved = entries
            .iter()
            .find(|p| p.get("name").and_then(toml::Value::as_str) == Some(provider))
            .cloned();
        entries.retain(|p| p.get("name").and_then(toml::Value::as_str) != Some(provider));
    }
    if let Some(bindings) = doc
        .get_mut("agent_models")
        .and_then(toml::Value::as_table_mut)
    {
        let keys: Vec<String> = bindings
            .iter()
            .filter(|(key, value)| key.ends_with("_provider") && value.as_str() == Some(provider))
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            bindings.remove(key.trim_end_matches("_provider"));
            bindings.remove(&key);
        }
    }
    if let Some(bindings) = doc
        .get_mut("last_model")
        .and_then(toml::Value::as_table_mut)
    {
        bindings.retain(|_, value| value.as_str() != Some(provider));
    }
    if let Some(caps) = doc
        .get_mut("capabilities")
        .and_then(toml::Value::as_table_mut)
    {
        let keys: Vec<String> = caps
            .iter()
            .filter(|(k, v)| k.ends_with("_provider") && v.as_str() == Some(provider))
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            caps.remove(key.trim_end_matches("_provider"));
            caps.remove(&key);
        }
    }
    crate::config::write_provider_config(
        config,
        &toml::to_string_pretty(&doc).map_err(|_| "Could not encode providers")?,
    )
    .map_err(|_| "Could not remove signed-out models")?;
    Ok(saved)
}

pub fn provider_name(id: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(id.as_bytes());
    format!(
        "custom-chatgpt-{}",
        hash[..12]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

pub fn sync_models(config: &Path, id: &str) -> Result<Vec<Model>, String> {
    let token = fresh_token(config, id)?;
    let models = protocol::models(&token)?;
    // Recheck session after I/O so logout cannot silently reconnect a provider.
    let store = LockedStore::open(config, CREDENTIAL_FILE)?;
    let account = store
        .data
        .accounts
        .iter()
        .find(|a| a.client_id == id && !a.access_token.is_empty())
        .ok_or("Sign in with ChatGPT to continue")?;
    write_provider_with_saved(
        config,
        &account.info(),
        &models,
        account.saved_provider.as_ref(),
    )?;
    Ok(models)
}

#[cfg(test)]
fn write_provider(config: &Path, account: &AccountInfo, models: &[Model]) -> Result<(), String> {
    write_provider_with_saved(config, account, models, None)
}

fn write_provider_with_saved(
    config: &Path,
    account: &AccountInfo,
    models: &[Model],
    saved: Option<&toml::Value>,
) -> Result<(), String> {
    let _guard = crate::config::lock_provider_config();
    let text =
        std::fs::read_to_string(config).map_err(|_| "Could not read provider configuration")?;
    let mut doc: toml::Value = text.parse().map_err(|_| "Invalid provider configuration")?;
    crate::config::normalize_provider_document(&mut doc);
    let root = doc.as_table_mut().ok_or("Invalid provider configuration")?;
    let entries = root
        .entry("providers")
        .or_insert_with(|| toml::Value::Array(vec![]))
        .as_array_mut()
        .ok_or("Invalid provider list")?;
    let previous = entries
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(&account.provider))
        .cloned()
        .or_else(|| saved.cloned());
    let mut provider = previous.unwrap_or_else(|| toml::Value::Table(Default::default()));
    let map = provider.as_table_mut().ok_or("Invalid ChatGPT provider")?;
    for (key, value) in [
        ("name", account.provider.as_str()),
        ("display_name", account.label.as_str()),
        ("provider_type", "custom"),
        ("api_key", ""),
        ("base_url", API_BASE),
    ] {
        map.insert(key.into(), toml::Value::String(value.into()));
    }
    // No authorization URL or token is stored in exportable configuration.
    map.insert(
        "oauth".into(),
        toml::Value::try_from(protocol::oauth_config("", Some(&account.account_id)))
            .map_err(|_| "Could not encode ChatGPT provider")?,
    );
    let old_models = map
        .get("models")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    map.insert(
        "models".into(),
        toml::Value::Array(
            models
                .iter()
                .map(|m| {
                    // 复用旧条目以保留用户的能力设置，但协议路由与订阅约束必须
                    // 每次都补齐：旧版本写入的条目没有 transport / responses，
                    // 缺失会让 ResponseOptions 落到全零默认值，temperature /
                    // max_output_tokens 照发，SIWC 预览直接 400。
                    let mut value = old_models
                        .iter()
                        .find(|old| old.get("id").and_then(|v| v.as_str()) == Some(&m.slug))
                        .cloned()
                        .unwrap_or_else(|| {
                            let mut value = toml::value::Table::new();
                            value.insert("id".into(), toml::Value::String(m.slug.clone()));
                            // No guessed context window or modality capabilities.
                            toml::Value::Table(value)
                        });
                    if let Some(table) = value.as_table_mut() {
                        table
                            .entry("transport")
                            .or_insert_with(|| toml::Value::String("responses".into()));
                        table.entry("responses").or_insert_with(|| {
                            toml::Value::try_from(super::response_options())
                                .expect("response options")
                        });
                    }
                    value
                })
                .collect(),
        ),
    );
    entries.retain(|p| p.get("name").and_then(|v| v.as_str()) != Some(&account.provider));
    entries.push(provider);
    crate::config::write_provider_config(
        config,
        &toml::to_string_pretty(&doc).map_err(|_| "Could not encode providers")?,
    )
    .map_err(|_| "Could not save ChatGPT models".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logout_removes_provider_and_bindings_but_keeps_registration() {
        let dir = std::env::temp_dir().join(format!("nuphus-logout-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("中文配置.toml");
        std::fs::write(&config, format!("model = ''\n[agent_models]\nleader='model'\nleader_provider='{}'\nworkflow='other'\nworkflow_provider='other-provider'\n", provider_name("oaiapp_logout"))).unwrap();
        let info = AccountInfo {
            account_id: "oaiapp_logout".into(),
            provider: provider_name("oaiapp_logout"),
            label: "Account".into(),
            logged_in: true,
        };
        write_provider(
            &config,
            &info,
            &[Model {
                slug: "model".into(),
                display_name: "Model".into(),
            }],
        )
        .unwrap();
        {
            let mut store = LockedStore::open(&config, CREDENTIAL_FILE).unwrap();
            store.data.accounts.push(Account {
                client_id: info.account_id.clone(),
                subject: "sub".into(),
                email: String::new(),
                access_token: protect("access").unwrap(),
                refresh_token: String::new(),
                id_token: String::new(),
                scope: "chatgpt.tokens.use.direct".into(),
                expires_at: i64::MAX,
                saved_provider: None,
            });
            store.save().unwrap();
        }
        assert!(logout(&config, &info.account_id).unwrap());
        let registry = crate::config::ModelRegistry::from_toml(config.to_str().unwrap()).unwrap();
        assert!(
            registry.providers.is_empty(),
            "signed-out models must leave the provider picker"
        );
        let saved = accounts(&config).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].logged_in);
        let doc: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        assert!(doc["agent_models"].get("leader").is_none());
        assert_eq!(
            doc["agent_models"]["workflow_provider"].as_str(),
            Some("other-provider")
        );
        assert!(LockedStore::open(&config, CREDENTIAL_FILE)
            .unwrap()
            .account_mut(&info.account_id)
            .unwrap()
            .saved_provider
            .is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
    /// 已存在的模型条目必须在同步时补齐 Responses 约束。
    ///
    /// 回归场景：用户先用旧版接过 ChatGPT（models 里只有 id + 能力字段），
    /// 升级后 sync_models 命中 `.cloned()` 复用分支。若不复用分支补写
    /// `transport` / `responses`，ResponseOptions 会落到全零默认值，
    /// 导致 temperature / max_output_tokens 照发，SIWC 预览直接 400。
    #[test]
    fn syncing_an_existing_model_still_gets_response_constraints() {
        let dir = std::env::temp_dir().join(format!("nuphus-chatgpt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("providers.toml");
        let info = AccountInfo {
            account_id: "oaiapp_test".into(),
            provider: provider_name("oaiapp_test"),
            label: "Account".into(),
            logged_in: true,
        };
        // 预先写入一个「旧式」模型条目：只带 id 与用户能力设置，没有 transport / responses。
        let legacy = format!(
            "[[providers]]\nname = \"{}\"\nprovider_type = \"custom\"\napi_key = \"\"\nbase_url = \"https://api.openai.com/v1\"\n[[providers.models]]\nid = \"gpt-6-luna\"\nsupports_vision = true\ncontext_window = 200000\n",
            info.provider
        );
        std::fs::write(&config, legacy).unwrap();

        write_provider(
            &config,
            &info,
            &[Model {
                slug: "gpt-6-luna".into(),
                display_name: "GPT-6 Luna".into(),
            }],
        )
        .unwrap();

        let doc: toml::Value = std::fs::read_to_string(&config).unwrap().parse().unwrap();
        // 先确认结构，避免 index not found 掩盖真实断言
        let models = doc["providers"][0]["models"]
            .as_array()
            .expect("providers[0].models must be an array");
        assert_eq!(
            models.len(),
            1,
            "exactly one model expected, got {models:#?}"
        );
        let model = &models[0];
        assert!(
            model.get("transport").is_some(),
            "existing model must gain per-model transport routing, got {model:#?}"
        );
        assert_eq!(model["transport"].as_str(), Some("responses"));
        let responses = model
            .get("responses")
            .expect("existing model must gain Responses constraints");
        assert_eq!(responses["store"].as_bool(), Some(false));
        assert_eq!(responses["developer_role"].as_bool(), Some(true));
        assert_eq!(responses["tool_namespace"].as_str(), Some("nuphus"));
        assert_eq!(responses["require_completed"].as_bool(), Some(true));
        let forbidden: Vec<&str> = responses["forbidden_fields"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert!(forbidden.contains(&"temperature"));
        assert!(forbidden.contains(&"max_output_tokens"));
        // 用户原有的能力设置必须保留。
        assert_eq!(model["supports_vision"].as_bool(), Some(true));
        assert_eq!(model["context_window"].as_integer(), Some(200000));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn host_is_stable_and_credentials_do_not_enter_provider_settings() {
        let dir = std::env::temp_dir().join(format!("nuphus-chatgpt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("providers.toml");
        std::fs::write(&config, "model = ''\n").unwrap();
        let a = prepare_login(&config, 1234, None).unwrap();
        let b = prepare_login(&config, 1235, None).unwrap();
        let host = |s: &str| {
            reqwest::Url::parse(s)
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "ext_agent_host_id")
                .unwrap()
                .1
                .into_owned()
        };
        assert_eq!(host(&a.authorize_url), host(&b.authorize_url));
        assert_ne!(a.state, b.state);
        let info = AccountInfo {
            account_id: "oaiapp_test".into(),
            provider: provider_name("oaiapp_test"),
            label: "Account".into(),
            logged_in: true,
        };
        write_provider(
            &config,
            &info,
            &[Model {
                slug: "model-a".into(),
                display_name: "Model A".into(),
            }],
        )
        .unwrap();
        let registry = crate::config::ModelRegistry::from_toml(config.to_str().unwrap()).unwrap();
        let p = &registry.providers[0];
        assert!(p.api_key.is_empty());
        assert_eq!(
            p.oauth.as_ref().unwrap().account_ref.as_deref(),
            Some("oaiapp_test")
        );
        assert!(fresh_token(&config, "oaiapp_missing").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.join("chatgpt-accounts.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn rotating_refresh_is_serialized_and_signed_out_sessions_fail_closed() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let dir =
            std::env::temp_dir().join(format!("nuphus-chatgpt-refresh-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("providers.toml");
        {
            let mut store = LockedStore::open(&config, CREDENTIAL_FILE).unwrap();
            store.data.accounts.push(Account {
                client_id: "oaiapp_test".into(),
                subject: "sub".into(),
                email: String::new(),
                access_token: protect("expired").unwrap(),
                refresh_token: protect("refresh-1").unwrap(),
                id_token: String::new(),
                scope: "chatgpt.tokens.use.direct".into(),
                expires_at: 1,
                saved_provider: None,
            });
            store.save().unwrap();
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let path = config.clone();
                let calls = calls.clone();
                std::thread::spawn(move || {
                    fresh_token_with(&path, "oaiapp_test", |client, refresh| {
                        assert_eq!(client, "oaiapp_test");
                        assert_eq!(refresh, "refresh-1");
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(Tokens {
                            access_token: "new-access".into(),
                            refresh_token: Some("refresh-2".into()),
                            id_token: None,
                            expires_in: 3600,
                            scope: None,
                            token_type: "Bearer".into(),
                        })
                    })
                    .unwrap()
                })
            })
            .collect();
        for worker in workers {
            assert_eq!(worker.join().unwrap(), "new-access");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        {
            let mut store = LockedStore::open(&config, CREDENTIAL_FILE).unwrap();
            let account = store.account_mut("oaiapp_test").unwrap();
            assert_eq!(reveal(&account.refresh_token).unwrap(), "refresh-2");
            account.access_token.clear();
            account.refresh_token.clear();
            store.save().unwrap();
        }
        assert!(fresh_token_with(&config, "oaiapp_test", |_, _| panic!(
            "must not refresh signed-out account"
        ))
        .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
