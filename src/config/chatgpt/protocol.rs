pub use crate::config::oauth_flow::{LoginAttempt, Tokens};
use crate::config::{
    oauth_flow::{self, get_json, http},
    ProviderOAuth,
};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::HashMap;
pub const API_BASE: &str = "https://api.openai.com/v1";
const ISSUER: &str = "https://auth.openai.com";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
pub(super) const CREDENTIAL_FILE: &str = "chatgpt-accounts.json";

#[derive(Clone, Deserialize, Serialize)]
pub struct Model {
    pub slug: String,
    pub display_name: String,
}

/** SIWC 的服务商约束集中在此；通用 OAuth 和 transport 只读取声明。 */
pub(super) fn oauth_config(host: &str, client: Option<&str>) -> ProviderOAuth {
    let mut extra = std::collections::BTreeMap::from([
        ("resource".into(), API_BASE.into()),
        ("nonce".into(), "{{nonce}}".into()),
        (
            "ext_agent_host_id".into(),
            if host.is_empty() {
                "{{host_id}}".into()
            } else {
                host.into()
            },
        ),
    ]);
    if client.is_none() {
        extra.insert("agent_name_hint".into(), "Nuphus".into());
    }
    ProviderOAuth {
        account_ref: client.map(str::to_owned),
        credential_file: CREDENTIAL_FILE.into(),
        authorize_url: format!("{ISSUER}/api/accounts/authorize"),
        token_url: format!("{ISSUER}/api/accounts/oauth/token"),
        client_id: client.unwrap_or(DYNAMIC_CLIENT).into(),
        scopes: "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct"
            .into(),
        use_pkce: true,
        extra_authorize_params: extra,
        extra_token_params: std::collections::BTreeMap::from([(
            "resource".into(),
            API_BASE.into(),
        )]),
        verify_id_token: true,
        issuer: ISSUER.into(),
        jwks_url: format!("{ISSUER}/.well-known/jwks.json"),
        dynamic_client_registration: client.is_none(),
        required_scopes: vec!["chatgpt.tokens.use.direct".into()],
        ..Default::default()
    }
}

pub(super) fn login_attempt(
    host: &str,
    port: u16,
    client: Option<&str>,
    subject: Option<String>,
) -> LoginAttempt {
    LoginAttempt::new(
        &oauth_config(host, client),
        format!("http://127.0.0.1:{port}/auth/callback"),
        subject,
    )
    .expect("valid SIWC configuration")
}
#[cfg(test)]
pub(super) fn require_scope(scope: &str) -> Result<(), String> {
    oauth_flow::require_scopes(&oauth_config("", None), scope)
}
pub(super) fn refresh(client: &str, token: &str) -> Result<Tokens, String> {
    oauth_flow::refresh(&oauth_config("", Some(client)), client, token)
}

#[cfg(test)]
fn verify_identity(
    token: &str,
    jwks: &jsonwebtoken::jwk::JwkSet,
    client: &str,
    nonce: &str,
) -> Result<oauth_flow::Identity, String> {
    oauth_flow::verify_identity(token, jwks, ISSUER, client, nonce)
}
pub(super) fn models(token: &str) -> Result<Vec<Model>, String> {
    let value: serde_json::Value = get_json(&http()?, &format!("{API_BASE}/models"), Some(token))?;
    parse_models(&value)
}

pub(super) fn parse_models(value: &serde_json::Value) -> Result<Vec<Model>, String> {
    value
        .get("models")
        .and_then(|v| v.as_array())
        .ok_or("Missing ChatGPT model catalog")?
        .iter()
        .filter(|m| m["visibility"] == "list")
        .map(|m| {
            let model: Model =
                serde_json::from_value(m.clone()).map_err(|_| "Invalid ChatGPT model entry")?;
            if model.slug.is_empty() {
                return Err("Empty ChatGPT model slug".into());
            }
            Ok(model)
        })
        .collect()
}

pub(super) fn revoke(client: &str, refresh: &str) -> bool {
    let Ok(http) = http() else {
        return false;
    };
    let Ok(discovery) = get_json::<serde_json::Value>(
        &http,
        &format!("{ISSUER}/.well-known/openid-configuration"),
        None,
    ) else {
        return false;
    };
    let Some(endpoint) = discovery["revocation_endpoint"].as_str() else {
        return false;
    };
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    http.post(url)
        .form(&[
            ("token", refresh),
            ("token_type_hint", "refresh_token"),
            ("client_id", client),
        ])
        .send()
        .is_ok_and(|r| r.status().as_u16() == 200)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use jsonwebtoken::{jwk::JwkSet, Algorithm};
    #[test]
    fn authorization_and_callback_bind_registration() {
        let attempt = login_attempt("urn:uuid:host", 54321, None, None);
        let url = reqwest::Url::parse(&attempt.authorize_url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], DYNAMIC_CLIENT);
        assert_eq!(query["agent_name_hint"], "Nuphus");
        assert_eq!(
            query["redirect_uri"],
            "http://127.0.0.1:54321/auth/callback"
        );
        let mut callback = HashMap::from([
            ("state".into(), attempt.state.clone()),
            ("code".into(), "code".into()),
        ]);
        assert!(attempt.callback(&callback).is_err());
        callback.insert("client_id".into(), "oaiapp_a".into());
        assert_eq!(attempt.callback(&callback).unwrap().0, "oaiapp_a");
        callback.insert("state".into(), "forged".into());
        assert!(attempt.callback(&callback).is_err());
        let returning = login_attempt("urn:uuid:host", 54322, Some("oaiapp_b"), Some("sub".into()));
        assert!(!returning.authorize_url.contains("agent_name_hint"));
        callback.insert("state".into(), returning.state.clone());
        assert!(returning.callback(&callback).is_err());
        callback.remove("client_id");
        assert_eq!(returning.callback(&callback).unwrap().0, "oaiapp_b");
        callback.insert("error".into(), "access_denied".into());
        assert!(returning.callback(&callback).is_err());
    }
    #[test]
    fn scope_and_catalog_are_account_specific() {
        assert!(require_scope("openid chatgpt.tokens.use.direct email").is_ok());
        assert!(require_scope("chatgpt.tokens.use.direct.fake").is_err());
        let list = parse_models(&serde_json::json!({"models":[
            {"slug":"b","display_name":"B","visibility":"list"},
            {"slug":"hidden","display_name":"Hidden","visibility":"hide"},
            {"slug":"a","display_name":"A","visibility":"list"}
        ]}))
        .unwrap();
        assert_eq!(
            list.iter().map(|m| m.slug.as_str()).collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        assert!(parse_models(&serde_json::json!({"data":[]})).is_err());
    }
    #[test]
    fn verifies_signature_issuer_audience_expiry_nonce_and_subject() {
        // Public, synthetic fixture keys are used only in tests.
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!("test_key.pem")).unwrap();
        let jwks: JwkSet = serde_json::from_str(include_str!("test_jwks.json")).unwrap();
        let now = super::super::super::oauth::now_unix();
        let claims = serde_json::json!({"iss":ISSUER,"aud":"oaiapp_test","sub":"user-1",
            "iat":now,"exp":now+3600,"nonce":"nonce-1","email":"test@example.invalid"});
        let sign = |claims: &serde_json::Value| {
            let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
            header.kid = Some("test-only".into());
            jsonwebtoken::encode(&header, claims, &key).unwrap()
        };
        let token = sign(&claims);
        assert!(verify_identity(&token, &jwks, "oaiapp_test", "nonce-1").is_ok());
        assert!(verify_identity(&token, &jwks, "oaiapp_other", "nonce-1").is_err());
        assert!(verify_identity(&token, &jwks, "oaiapp_test", "wrong-nonce").is_err());
        for (field, bad) in [
            ("iss", serde_json::json!("https://attacker.invalid")),
            ("sub", serde_json::json!("")),
            ("exp", serde_json::json!(now - 60)),
            ("iat", serde_json::json!(now + 60)),
        ] {
            let mut altered = claims.clone();
            altered[field] = bad;
            assert!(
                verify_identity(&sign(&altered), &jwks, "oaiapp_test", "nonce-1").is_err(),
                "{field}"
            );
        }
        let mut missing_iat = claims.clone();
        missing_iat.as_object_mut().unwrap().remove("iat");
        assert!(verify_identity(&sign(&missing_iat), &jwks, "oaiapp_test", "nonce-1").is_err());
        let parts: Vec<_> = token.split('.').collect();
        let forged = format!(
            "{}.{}.{}",
            parts[0],
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"sub":"attacker"}"#),
            parts[2]
        );
        assert!(verify_identity(&forged, &jwks, "oaiapp_test", "nonce-1").is_err());
    }
}
