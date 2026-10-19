//! Sign in with ChatGPT for locally hosted open-source clients.
//!
//! Credentials live separately from exportable provider settings. Only opaque
//! account references enter providers.toml. All credential transactions acquire
//! an OS file lock (including refresh) so rotating refresh tokens cannot race.
//! Protocol: https://developers.openai.com/siwc/token-sharing-open-source
mod protocol;
mod store;

pub use protocol::{LoginAttempt, Model, API_BASE};
pub use store::{
    accounts, finish_login, fresh_token, logout, prepare_login, sync_models, AccountInfo,
};

pub fn response_options() -> crate::transports::responses::ResponseOptions {
    crate::transports::responses::ResponseOptions {
        store: Some(false),
        developer_role: true,
        tool_namespace: Some("nuphus".into()),
        require_completed: true,
        forbidden_fields: vec!["temperature".into(), "max_output_tokens".into()],
    }
}

/** 兼容旧版配置的单一迁移入口；磁盘凭据和用户模型能力保持不变。 */
pub(crate) fn migrate_legacy(doc: &mut toml::Value) {
    for provider in doc
        .get_mut("providers")
        .and_then(toml::Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        let Some(account) = provider
            .get("oauth")
            .and_then(|o| o.get("chatgpt_account"))
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        provider["oauth"] = toml::Value::try_from(protocol::oauth_config("", Some(&account)))
            .expect("OAuth profile");
        for model in provider
            .get_mut("models")
            .and_then(toml::Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if let Some(map) = model.as_table_mut() {
                map.entry("transport")
                    .or_insert_with(|| toml::Value::String("responses".into()));
                map.entry("responses").or_insert_with(|| {
                    toml::Value::try_from(response_options()).expect("response options")
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_account_migrates_without_losing_user_model_preferences() {
        let mut doc: toml::Value = "[[providers]]\nname='arbitrary'\n[providers.oauth]\nchatgpt_account='oaiapp_existing'\n[[providers.models]]\nid='model'\nsupports_vision=true\ncontext_window=123456\n".parse().unwrap();
        migrate_legacy(&mut doc);
        assert_eq!(
            doc["providers"][0]["oauth"]["account_ref"].as_str(),
            Some("oaiapp_existing")
        );
        assert!(doc["providers"][0]["oauth"]
            .get("chatgpt_account")
            .is_none());
        assert_eq!(
            doc["providers"][0]["models"][0]["transport"].as_str(),
            Some("responses")
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["supports_vision"].as_bool(),
            Some(true)
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["context_window"].as_integer(),
            Some(123456)
        );
        let previous = doc.clone();
        migrate_legacy(&mut doc);
        assert_eq!(doc, previous);
    }
}
