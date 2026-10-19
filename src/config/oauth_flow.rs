/** OAuth 授权码、PKCE 和 OIDC 的共享协议实现；不负责浏览器或凭据存储。 */
use super::ProviderOAuth;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub struct LoginAttempt {
    pub state: String,
    pub authorize_url: String,
    pub client_id: String,
    pub expected_subject: Option<String>,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    oauth: ProviderOAuth,
}

#[derive(Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
    pub scope: Option<String>,
    #[serde(default = "bearer")]
    pub token_type: String,
}
fn bearer() -> String {
    "Bearer".into()
}

#[derive(Clone, Deserialize)]
pub struct Identity {
    pub sub: String,
    pub email: Option<String>,
    nonce: String,
    iat: u64,
}

fn random() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl LoginAttempt {
    pub fn new(
        oauth: &ProviderOAuth,
        redirect_uri: String,
        subject: Option<String>,
    ) -> Result<Self, String> {
        let state = random();
        let nonce = random();
        let verifier = random();
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        let mut url = reqwest::Url::parse(&oauth.authorize_url)
            .map_err(|_| "Invalid OAuth authorization URL")?;
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", &oauth.client_id),
            ("redirect_uri", &redirect_uri),
            ("state", &state),
        ]);
        if !oauth.scopes.is_empty() {
            url.query_pairs_mut().append_pair("scope", &oauth.scopes);
        }
        if oauth.use_pkce {
            url.query_pairs_mut().extend_pairs([
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
            ]);
        }
        for (key, value) in &oauth.extra_authorize_params {
            if matches!(
                key.as_str(),
                "response_type"
                    | "client_id"
                    | "redirect_uri"
                    | "state"
                    | "scope"
                    | "code_challenge"
                    | "code_challenge_method"
            ) {
                return Err("OAuth extra parameters cannot replace protocol fields".into());
            }
            url.query_pairs_mut()
                .append_pair(key, &value.replace("{{nonce}}", &nonce));
        }
        if oauth.verify_id_token {
            if oauth.issuer.is_empty() || oauth.jwks_url.is_empty() {
                return Err("OIDC requires issuer and JWKS URL".into());
            }
            if !oauth.extra_authorize_params.contains_key("nonce") {
                url.query_pairs_mut().append_pair("nonce", &nonce);
            }
        }
        Ok(Self {
            state,
            nonce,
            verifier,
            redirect_uri,
            client_id: oauth.client_id.clone(),
            expected_subject: subject,
            authorize_url: url.into(),
            oauth: oauth.clone(),
        })
    }

    pub fn callback(&self, params: &HashMap<String, String>) -> Result<(String, String), String> {
        if params.get("state") != Some(&self.state) {
            return Err("OAuth callback state mismatch".into());
        }
        if params.contains_key("error") {
            return Err("OAuth authorization was declined".into());
        }
        let client = params
            .get("client_id")
            .map(String::as_str)
            .unwrap_or(&self.client_id);
        if self.oauth.dynamic_client_registration {
            if client.is_empty() || client == self.client_id {
                return Err("OAuth registration did not return an issued client ID".into());
            }
        } else if client != self.client_id {
            return Err("OAuth callback changed the selected registration".into());
        }
        let code = params
            .get("code")
            .filter(|s| !s.is_empty())
            .ok_or("OAuth callback is missing an authorization code")?;
        Ok((client.into(), code.clone()))
    }

    pub fn exchange(
        &self,
        client_id: &str,
        code: &str,
    ) -> Result<(Tokens, Option<Identity>), String> {
        let mut form = vec![
            ("grant_type".into(), "authorization_code".into()),
            ("client_id".into(), client_id.into()),
            ("code".into(), code.into()),
            ("redirect_uri".into(), self.redirect_uri.clone()),
        ];
        if self.oauth.use_pkce {
            form.push(("code_verifier".into(), self.verifier.clone()));
        }
        let http = http()?;
        let tokens = token_request(&http, &self.oauth, form)?;
        require_scopes(&self.oauth, tokens.scope.as_deref().unwrap_or(""))?;
        let identity = if self.oauth.verify_id_token {
            let token = tokens
                .id_token
                .as_deref()
                .ok_or("OIDC provider did not return an ID token")?;
            let jwks: JwkSet = get_json(&http, &self.oauth.jwks_url, None)?;
            let identity =
                verify_identity(token, &jwks, &self.oauth.issuer, client_id, &self.nonce)?;
            if self
                .expected_subject
                .as_ref()
                .is_some_and(|sub| *sub != identity.sub)
            {
                return Err("OIDC identity does not match the selected account".into());
            }
            Some(identity)
        } else {
            None
        };
        Ok((tokens, identity))
    }
}

pub fn verify_identity(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    client: &str,
    nonce: &str,
) -> Result<Identity, String> {
    let header = decode_header(token).map_err(|_| "Invalid ID token")?;
    if header.alg != Algorithm::RS256 {
        return Err("Unsupported ID-token signing algorithm".into());
    }
    let key = header
        .kid
        .as_deref()
        .and_then(|kid| jwks.find(kid))
        .ok_or("Unknown signing key")?;
    let key = DecodingKey::from_jwk(key).map_err(|_| "Invalid signing key")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[issuer]);
    validation.set_audience(&[client]);
    validation.set_required_spec_claims(&["sub", "exp", "iat", "iss", "aud"]);
    validation.leeway = 5;
    let identity = decode::<Identity>(token, &key, &validation)
        .map_err(|_| "ID-token signature or claims validation failed")?
        .claims;
    if identity.sub.is_empty()
        || identity.nonce != nonce
        || identity.iat > super::oauth::now_unix() as u64 + 5
    {
        return Err("ID-token subject or nonce mismatch".into());
    }
    Ok(identity)
}

pub fn require_scopes(oauth: &ProviderOAuth, scope: &str) -> Result<(), String> {
    if oauth
        .required_scopes
        .iter()
        .all(|required| scope.split_whitespace().any(|s| s == required))
    {
        Ok(())
    } else {
        Err("Required OAuth permission was not granted; sign in again".into())
    }
}

pub fn http() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create OAuth HTTP client".into())
}

fn token_request(
    http: &reqwest::blocking::Client,
    oauth: &ProviderOAuth,
    mut form: Vec<(String, String)>,
) -> Result<Tokens, String> {
    for (key, value) in &oauth.extra_token_params {
        if matches!(
            key.as_str(),
            "grant_type"
                | "client_id"
                | "code"
                | "redirect_uri"
                | "code_verifier"
                | "refresh_token"
        ) {
            return Err("OAuth extra parameters cannot replace protocol fields".into());
        }
        form.push((key.clone(), value.clone()));
    }
    let response = http
        .post(&oauth.token_url)
        .form(&form)
        .send()
        .map_err(|_| "OAuth token request failed; check your connection")?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth token request failed (HTTP {}); sign in again if authorization expired",
            response.status().as_u16()
        ));
    }
    let tokens: Tokens = response
        .json()
        .map_err(|_| "Invalid OAuth token response")?;
    if tokens.access_token.is_empty()
        || tokens.expires_in < 0
        || !tokens.token_type.eq_ignore_ascii_case("bearer")
    {
        return Err("Invalid OAuth token credentials or expiry".into());
    }
    Ok(tokens)
}

pub fn refresh(oauth: &ProviderOAuth, client: &str, token: &str) -> Result<Tokens, String> {
    token_request(
        &http()?,
        oauth,
        vec![
            ("grant_type".into(), "refresh_token".into()),
            ("client_id".into(), client.into()),
            ("refresh_token".into(), token.into()),
        ],
    )
}

pub fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::blocking::Client,
    url: &str,
    token: Option<&str>,
) -> Result<T, String> {
    let mut request = http.get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().map_err(|_| "OAuth service request failed")?;
    if !response.status().is_success() {
        return Err(format!(
            "OAuth service returned HTTP {}",
            response.status().as_u16()
        ));
    }
    response
        .json()
        .map_err(|_| "Invalid OAuth service response".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorize_binds_nonce_pkce_and_dynamic_registration_without_a_vendor() {
        let mut oauth = ProviderOAuth {
            authorize_url: "https://identity.example/authorize?audience=api".into(),
            client_id: "bootstrap".into(),
            use_pkce: true,
            dynamic_client_registration: true,
            ..Default::default()
        };
        oauth
            .extra_authorize_params
            .insert("nonce".into(), "{{nonce}}".into());
        oauth
            .extra_authorize_params
            .insert("resource".into(), "https://service.example/v1".into());
        let attempt =
            LoginAttempt::new(&oauth, "http://127.0.0.1:1234/callback".into(), None).unwrap();
        let query: HashMap<String, String> = reqwest::Url::parse(&attempt.authorize_url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
        assert_eq!(query["nonce"], attempt.nonce);
        assert_eq!(
            query["code_challenge"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(attempt.verifier.as_bytes()))
        );
        assert_eq!(query["audience"], "api");
        let mut params = HashMap::from([
            ("state".into(), attempt.state.clone()),
            ("code".into(), "code".into()),
        ]);
        assert!(attempt.callback(&params).is_err());
        params.insert("client_id".into(), "issued-by-any-provider".into());
        assert_eq!(
            attempt.callback(&params).unwrap().0,
            "issued-by-any-provider"
        );
        params.insert("state".into(), "wrong".into());
        assert!(attempt.callback(&params).is_err());
        oauth
            .extra_authorize_params
            .insert("state".into(), "fixed".into());
        assert!(LoginAttempt::new(&oauth, "http://127.0.0.1:1234/callback".into(), None).is_err());
    }

    #[test]
    fn static_client_cannot_be_replaced_and_denial_is_redacted() {
        let oauth = ProviderOAuth {
            authorize_url: "https://identity.example/authorize".into(),
            client_id: "registered".into(),
            ..Default::default()
        };
        let attempt =
            LoginAttempt::new(&oauth, "http://127.0.0.1:1234/callback".into(), None).unwrap();
        let mut params = HashMap::from([
            ("state".into(), attempt.state.clone()),
            ("code".into(), "code".into()),
            ("client_id".into(), "attacker".into()),
        ]);
        assert!(attempt.callback(&params).is_err());
        params.remove("client_id");
        assert_eq!(attempt.callback(&params).unwrap().0, "registered");
        params.insert("error".into(), "secret-must-not-escape".into());
        assert!(!attempt
            .callback(&params)
            .unwrap_err()
            .contains("secret-must-not-escape"));
    }
}
