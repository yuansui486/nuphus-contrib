//! OpenAI Responses Transport configuration
//!
//! Endpoint rule (refactor design §4.5): `{base}/responses`（base 去尾斜杠）。

use crate::config::provider::{ProviderQuirks, ResponseOptions};

/** Responses 传输配置：请求约束和每次请求的凭据来源独立声明。 */
#[derive(Debug, Clone)]
pub struct ResponsesConfig {
    pub credentials: Option<crate::config::credentials::TokenSource>,
    pub options: ResponseOptions,
    /// API endpoint base（不含 `/responses` 后缀；末尾斜杠会被剥离）
    pub base_url: String,
    /** 未配置动态凭据来源时使用的静态 API Key。 */
    pub api_key: String,
    /// Model name
    pub model: String,
    /// Request timeout (seconds)
    pub timeout_secs: u64,
    /// Auth header name（默认 `"authorization"`；Codex 变体可配）
    pub auth_header: String,
    /// Auth header value prefix（默认 `"Bearer "`）
    pub auth_prefix: String,
    /// Explicit Provider type（由接线方在构造时传入）
    pub provider_kind: Option<crate::api::ProviderKind>,
    /// Per-Provider protocol quirks（extra_headers 注入静态请求头等）
    pub quirks: ProviderQuirks,
    /// Reasoning effort（`reasoning.effort`），仅在 quirks 声明支持时发送。
    pub reasoning_effort: Option<String>,
}

impl ResponsesConfig {
    /// Full URL for the Responses API endpoint.
    pub(crate) fn endpoint(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_appends_responses() {
        let cfg = ResponsesConfig {
            base_url: "https://api.openai.com/v1".into(),
            api_key: "sk-test".into(),
            model: "gpt-5.6-luna".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: ProviderQuirks::default(),
            reasoning_effort: None,
            credentials: None,
            options: Default::default(),
        };
        assert_eq!(cfg.endpoint(), "https://api.openai.com/v1/responses");
    }

    #[test]
    fn endpoint_strips_trailing_slash() {
        let cfg = ResponsesConfig {
            base_url: "https://example.com/".into(),
            api_key: "sk-test".into(),
            model: "m".into(),
            timeout_secs: 30,
            auth_header: "authorization".into(),
            auth_prefix: "Bearer ".into(),
            provider_kind: None,
            quirks: ProviderQuirks::default(),
            reasoning_effort: None,
            credentials: None,
            options: Default::default(),
        };
        assert_eq!(cfg.endpoint(), "https://example.com/responses");
    }
}
