/** 每次请求解析凭据，认证失败直接返回错误，不回退到其他计费来源。 */
use super::ProviderOAuth;
use std::path::PathBuf;

#[derive(Clone)]
pub struct TokenSource {
    pub config_path: Option<PathBuf>,
    pub provider: String,
    pub oauth: ProviderOAuth,
}

impl std::fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenSource")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

impl TokenSource {
    pub fn resolve(&self) -> Result<String, String> {
        if let Some(path) = &self.config_path {
            if let Some(account) = &self.oauth.account_ref {
                return super::oauth_accounts::fresh_token_with(
                    path,
                    account,
                    &self.oauth,
                    |client, token| super::oauth_flow::refresh(&self.oauth, client, token),
                );
            }
            return super::oauth::ensure_fresh_oauth_token(path, &self.provider);
        }
        super::oauth::in_memory_fresh_token(&self.oauth, super::oauth::now_unix())
            .ok_or_else(|| "OAuth token expired; persistent credentials or a fresh in-memory token are required".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_source_works_without_a_file_and_expiry_fails_closed() {
        let mut source = TokenSource {
            config_path: None,
            provider: "arbitrary".into(),
            oauth: ProviderOAuth {
                access_token: "memory-token".into(),
                expires_at: Some(i64::MAX),
                ..Default::default()
            },
        };
        assert_eq!(source.resolve().unwrap(), "memory-token");
        assert!(!format!("{source:?}").contains("memory-token"));
        source.oauth.expires_at = Some(0);
        assert!(source.resolve().is_err());
    }
}
