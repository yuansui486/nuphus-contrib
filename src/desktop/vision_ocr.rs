//! Vision OCR via user-configured image understanding model
//!
//! Sends screenshot to user's configured vision model (capabilities.vision).
//!
//! 传输层复用：本模块**不**自行拼 URL / 鉴权头 / JSON body，而是取出模型所在段的
//! `Provider::transport()`（协议族由 Provider 自身声明：OpenAI 兼容自定义/中转段走
//! Chat Completions，`provider_type = "anthropic"` 走 Messages API），把图片作为
//! 一条 user 消息交给传输层。这样自定义/中转 Provider 与官方 Provider 的请求参数、
//! 鉴权、重试、超时、代理回退完全一致——不再存在第二套旁路实现。
//!
//! `stream = true`：部分 OpenAI 兼容中转端点在非流式模式下不返回图片结果（要求
//! 流式）；传输层用 `parse_sse` 解析 SSE，Anthropic 传输层同样按 SSE 解析，两者
//! 返回统一的 `Vec<StreamEvent>`。
//!
//! `max_tokens` 仅在模型段显式配置了 `ModelEntry.max_tokens` 时下发；未配置则不
//! 带该字段，由传输层回退链（`resolve_max_output_tokens` → 服务端默认上限）决定，
//! 与本仓其它 LLM 调用同一口径。
//!
//! Returns `Result<String, String>` — Ok(text) on success, Err(message) on failure.
//! Errors are wrapped into NuphusError::Tool by the caller (client::ocr).

use crate::api::MessageRequest;
use crate::config::{self, resolve_vision_provider, resolve_vision_strategy, VisionStrategy};

fn resolve_vision_model<'a>(
    registry: &'a config::ModelRegistry,
    model_id: &str,
    provider: Option<&str>,
) -> Result<(&'a config::ProviderConfig, &'a config::ModelEntry), String> {
    if let Some(provider) = provider.filter(|p| !p.is_empty()) {
        registry
            .find_model_for_provider(provider, model_id)
            .ok_or_else(|| format!("未找到视觉模型: {provider}/{model_id}"))
    } else {
        registry
            .find_model(model_id)
            .ok_or_else(|| format!("未找到视觉模型: {model_id}"))
    }
}

/// OCR via vision model (image content block + unified transport layer)
///
/// Loads the user's vision model config from capabilities.vision in the
/// model registry, encodes the BMP/PNG image as base64 data URL, and sends it
/// through the model segment's `Provider::transport()`.
///
/// 对外行为不变：接受文件路径，内部读取文件 → data URL → 直调内部函数。
pub async fn vision_ocr(image_path: &str, prompt: Option<&str>) -> Result<String, String> {
    // 4. Read image — PNG passes through as-is (screenshot now emits PNG);
    //    other formats are converted because LLM APIs don't support image/bmp
    let image_bytes = std::fs::read(image_path).map_err(|e| format!("读取图片失败: {e}"))?;
    let (mime_type, final_bytes) = if image_path.to_lowercase().ends_with(".png") {
        ("image/png", image_bytes)
    } else {
        let img =
            image::load_from_memory(&image_bytes).map_err(|e| format!("解析图片失败: {e}"))?;
        let mut png_buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png_buf, image::ImageFormat::Png)
            .map_err(|e| format!("转换PNG失败: {e}"))?;
        ("image/png", png_buf.into_inner())
    };
    let base64_image = base64_encode(&final_bytes);
    let data_url = format!("data:{mime_type};base64,{base64_image}");

    vision_ocr_data_url(&data_url, prompt).await
}

/// 视觉模型直调（data URL 版本）
///
/// 用户消息图片在 session 中已是冻结的 base64 data URL（BMP→PNG 在入 session 时
/// 已完成），无需再落临时文件。此函数复用 vision_ocr 的完整调用链，供
/// desktop_vision 工具与按需图片查看使用。
///
/// 协议解析（Chat Completions / Anthropic Messages）由对应 Provider 的传输层负责。
pub async fn vision_ocr_data_url(data_url: &str, prompt: Option<&str>) -> Result<String, String> {
    let vision_model_id = resolve_vision_model_id()?;

    // 2. 加载 registry 解析模型配置
    let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;

    // 3. Resolve model alias to find provider config
    let vision_provider = resolve_vision_provider();
    let (provider_config, model_entry) =
        resolve_vision_model(&registry, &vision_model_id, vision_provider.as_deref())?;

    // 错误文案上下文：实际模型 + provider 段名 + 跟随态标注。未显式配置视觉模型时
    // （strategy == Main）实际调用的是 Leader 主模型，错误文案必须讲清这一点，
    // 否则用户会去错误的配置项里排查。resolve_vision_model_id 内部已按同一策略
    // 解析过模型 id，这里重复调 resolve_vision_strategy 只为取跟随态布尔值
    // （load_registry 为轻量读盘，与本函数既有的多次读取同一口径）。
    let error_ctx = VisionErrorContext {
        model: model_entry.id.clone(),
        provider: provider_config.name.clone(),
        follow_leader: matches!(resolve_vision_strategy(), VisionStrategy::Main),
    };

    // 4. 该模型段的传输层（协议族由 Provider 自身声明；鉴权头 / 端点 / 重试 /
    //    超时 / 代理回退与该模型的常规 LLM 调用完全同源）
    let provider = config::registry::ProviderRegistry::builtin()
        .get(provider_config.provider_type.as_str())
        .ok_or_else(|| {
            format!(
                "未找到内置 Provider: {}",
                provider_config.provider_type.as_str()
            )
        })?;
    let transport = provider.transport(provider_config, &model_entry.id);

    // 5. 解析 data URL → mime_type + base64 载荷
    let (mime_type, base64_image) = split_data_url(data_url)?;

    // 6. 图片内容块：Anthropic 原生 image source block / OpenAI image_url
    //    （图片对两种协议都是「按 URL 取图」的远程资源，故统一用 data URL 形式传递）
    let prompt_text = prompt
        .filter(|p| !p.is_empty())
        .unwrap_or("请识别并输出这张图片中的所有文字，只输出文字内容，不要添加任何解释。");

    let content = if provider_config.provider_type == crate::api::ProviderKind::Anthropic {
        serde_json::json!([
            {
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": mime_type,
                    "data": base64_image
                }
            },
            { "type": "text", "text": prompt_text }
        ])
    } else {
        serde_json::json!([
            { "type": "text", "text": prompt_text },
            {
                "type": "image_url",
                "image_url": {
                    "url": data_url,
                    "detail": "high"
                }
            }
        ])
    };

    // temperature 不传：部分模型（如 Kimi 推理系）强制 temperature=1，传 0.0 会被 400 拒绝
    let mut request = MessageRequest::new(
        &model_entry.id,
        vec![serde_json::json!({
            "role": "user",
            "content": content,
        })],
    );
    // max_tokens：仅显式配置时下发（未配置则由传输层回退链决定，见模块文档）
    if let Some(max_tokens) = model_entry.max_tokens {
        request = request.with_max_tokens(max_tokens);
    }
    // 流式：中转端点要求（非流式不返回图片结果），传输层按 SSE 解析
    request = request.with_stream(true);

    // 7. 经统一传输层发起请求并解析为统一事件流（错误分类富化后单层透传）
    let events = transport.stream(request).await.map_err(|e| {
        let (status, body) = split_transport_error(&e);
        let kind = classify_vision_error(status, &body);
        format_vision_error(kind, &error_ctx, &body, status)
    })?;

    // 8. 事件流 → 纯文本（TextDelta 拼接；Error 不吞）
    events_to_text(events).map_err(|e| {
        let kind = classify_vision_error(None, &e);
        format_vision_error(kind, &error_ctx, &e, None)
    })
}

/// 统一事件流 → 文本。
///
/// - 所有 `TextDelta` 按到达顺序拼接（SSE 分片已由传输层合并）；
/// - 任一 `Error(e)` 直接失败——视觉调用的失败必须让 Leader 看到真实原因，
///   不能当作「空结果」静默降级；
/// - 空文本单独报错（模型拒答 / 事件流里没有可见文本），避免把空串当成功交付。
fn events_to_text(events: Vec<crate::transports::StreamEvent>) -> Result<String, String> {
    let mut text = String::new();
    for event in events {
        match event {
            crate::transports::StreamEvent::TextDelta(delta) => text.push_str(&delta),
            crate::transports::StreamEvent::Error(e) => return Err(e),
            _ => {}
        }
    }
    let text = text.trim();
    if text.is_empty() {
        return Err("视觉模型未返回任何文本内容".to_string());
    }
    Ok(text.to_string())
}

// ══════════════════════════════════════════════
// 失败分类与文案（desktop_vision 失败提醒体系）
// ══════════════════════════════════════════════

/// 视觉调用失败分类——纯状态码/字符串匹配，不引入任何 HTTP 错误处理依赖。
///
/// 分类决定两件事：
/// 1. 错误文案的「出路」段（用户下一步该做什么）；
/// 2. 是否升级 UI 横幅（Auth/Billing 重试无意义 → 发；RateLimit 稍后重试有意义 → 不发）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionErrorKind {
    /// API 密钥无效 / 未授权——重试无意义
    Auth,
    /// 余额不足 / 配额耗尽——重试无意义
    Billing,
    /// 限流 / 服务过载——稍后重试有意义
    RateLimit,
    /// 图片过大 / 格式不被接受
    ImageTooLarge,
    /// 其它（网络故障、未知错误、空响应等）
    Other,
}

/// 分类名（snake_case，跨进程契约：test_vision_model command / 前端按此渲染）。
pub fn vision_error_kind_name(kind: VisionErrorKind) -> &'static str {
    match kind {
        VisionErrorKind::Auth => "auth",
        VisionErrorKind::Billing => "billing",
        VisionErrorKind::RateLimit => "rate_limit",
        VisionErrorKind::ImageTooLarge => "image_too_large",
        VisionErrorKind::Other => "other",
    }
}

/// 计费类关键词：命中即 Billing——**优先级高于 HTTP 429**（OpenAI 的
/// insufficient_quota 就是 429 返回，但重试无意义，必须归入计费而非限流）。
const BILLING_KEYWORDS: &[&str] = &[
    "余额不足",
    "余额",
    "充值",
    "资源包",
    "欠费",
    "配额已用尽",
    "配额耗尽",
    "insufficient_quota",
    "insufficient balance",
    "exceeded your current quota",
    "quota exceeded",
    "billing",
    "balance",
];

/// 密钥类关键词
const AUTH_KEYWORDS: &[&str] = &[
    "invalid api key",
    "incorrect api key",
    "api key not valid",
    "invalid_api_key",
    "unauthorized",
    "authentication",
    "鉴权失败",
    "密钥无效",
    "无效的密钥",
    "密钥错误",
    "令牌无效",
    "access denied",
];

/// 限流类关键词
const RATE_LIMIT_KEYWORDS: &[&str] = &[
    "rate limit",
    "ratelimit",
    "too many requests",
    "请求过于频繁",
    "限流",
    "触发频率限制",
    "requests per min",
];

/// 图片过大类关键词
const IMAGE_TOO_LARGE_KEYWORDS: &[&str] = &[
    "image too large",
    "图片过大",
    "图片太大",
    "payload too large",
    "request entity too large",
    "file too large",
    "max image size",
    "image size exceeds",
];

fn body_contains(body_lower: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|k| body_lower.contains(k))
}

/// 视觉调用失败的分类纯函数（HTTP status + 关键词，纯 match/字符串）。
///
/// 匹配顺序 = 语义优先级：
/// 1. 计费关键词（429 + insufficient_quota 是计费不是限流——重试无意义）
/// 2. HTTP 401/403 → 密钥
/// 3. 密钥关键词
/// 4. HTTP 429 / 限流关键词
/// 5. HTTP 413 / 图片过大关键词
/// 6. 其它
pub fn classify_vision_error(status: Option<u16>, body: &str) -> VisionErrorKind {
    let body_lower = body.to_lowercase();
    if body_contains(&body_lower, BILLING_KEYWORDS) {
        return VisionErrorKind::Billing;
    }
    if matches!(status, Some(401) | Some(403)) {
        return VisionErrorKind::Auth;
    }
    if body_contains(&body_lower, AUTH_KEYWORDS) {
        return VisionErrorKind::Auth;
    }
    if status == Some(429) || body_contains(&body_lower, RATE_LIMIT_KEYWORDS) {
        return VisionErrorKind::RateLimit;
    }
    if status == Some(413) || body_contains(&body_lower, IMAGE_TOO_LARGE_KEYWORDS) {
        return VisionErrorKind::ImageTooLarge;
    }
    VisionErrorKind::Other
}

/// 文案模板的判别短语——[`format_vision_error`] 与 [`classify_vision_error_text`]
/// 共用同一常量，保证 format → classify 往返一致。
///
/// 横幅升级与 kind 回传只认这些短语（对最终透传文案判别时优先命中），这样
/// 限流/其它文案里上游恰好携带 quota/balance 字样的 raw body 不会把分类劫持成
/// 计费而误弹横幅（round-trip 单测对每种 kind 用敌意 body 验证此性质）。
const TEMPLATE_PHRASE_AUTH: &str = "API 密钥无效或未授权";
const TEMPLATE_PHRASE_BILLING: &str = "账户余额不足或配额已用尽";
const TEMPLATE_PHRASE_RATE_LIMIT: &str = "请求被限流";
const TEMPLATE_PHRASE_IMAGE_TOO_LARGE: &str = "图片过大或格式不被接受";

/// 对**最终透传文案**做二次分类（exec_tool 横幅升级 / test_vision_model 复用）。
///
/// 先认模板判别短语（`format_vision_error` 的产物必然携带，不受 raw body 关键词
/// 噪声干扰），再退回 [`classify_vision_error`] 处理非模板文本
/// （超时 / 未找到模型 / None 策略说明等——它们不携带判别短语）。
pub fn classify_vision_error_text(text: &str) -> VisionErrorKind {
    if text.contains(TEMPLATE_PHRASE_AUTH) {
        return VisionErrorKind::Auth;
    }
    if text.contains(TEMPLATE_PHRASE_BILLING) {
        return VisionErrorKind::Billing;
    }
    if text.contains(TEMPLATE_PHRASE_RATE_LIMIT) {
        return VisionErrorKind::RateLimit;
    }
    if text.contains(TEMPLATE_PHRASE_IMAGE_TOO_LARGE) {
        return VisionErrorKind::ImageTooLarge;
    }
    classify_vision_error(None, text)
}

/// 视觉错误文案的展示上下文（模型 / provider / 是否跟随 Leader）。
pub struct VisionErrorContext {
    /// 实际调用的模型 id
    pub model: String,
    /// 模型所在 provider 段名
    pub provider: String,
    /// strategy == Main（未显式配置、跟随 Leader 主模型）时为 true
    pub follow_leader: bool,
}

/// 组装**单层**错误文案：一段话讲清【哪个模型+provider】+【失败原因分类】+【出路】。
///
/// 单层化契约：本函数的产物就是最终文本——调用方（client.rs / exec_tool）不得再
/// 叠加前缀。Error 文案会进入 Leader 上下文，要求信息完整且不撒谎（失败必须如实
/// 透传，禁止编造「已重试」）。
pub fn format_vision_error(
    kind: VisionErrorKind,
    ctx: &VisionErrorContext,
    raw_detail: &str,
    status: Option<u16>,
) -> String {
    let target = if ctx.provider.is_empty() {
        ctx.model.clone()
    } else {
        format!("{} @ {}", ctx.model, ctx.provider)
    };
    let who = if ctx.follow_leader {
        format!("视觉模型 {target}（跟随 Leader）")
    } else {
        format!("视觉模型 {target}")
    };
    let detail = raw_detail.trim();
    let detail = if detail.is_empty() {
        match status {
            Some(s) => format!("HTTP {s}"),
            None => "无详细信息".to_string(),
        }
    } else {
        match status {
            Some(s) => format!("HTTP {s}：{detail}"),
            None => detail.to_string(),
        }
    };
    match kind {
        VisionErrorKind::Auth => format!(
            "图像理解调用失败（{who}）：{TEMPLATE_PHRASE_AUTH}。{detail}。\
             出路：到 设置 → 模型 → 图像音频模型 检查该服务商的 API Key 是否正确/过期；\
             修复前重试不会成功，请如实告知用户。"
        ),
        VisionErrorKind::Billing => format!(
            "图像理解调用失败（{who}）：{TEMPLATE_PHRASE_BILLING}。{detail}。\
             出路：到该服务商控制台充值或购买资源包后重试；\
             修复前图片理解类操作将持续失败，请如实告知用户，不要推测图像内容。"
        ),
        VisionErrorKind::RateLimit => format!(
            "图像理解调用失败（{who}）：{TEMPLATE_PHRASE_RATE_LIMIT}。{detail}。\
             出路：瞬时限制，稍等数十秒重试即可，无需改配置。"
        ),
        VisionErrorKind::ImageTooLarge => format!(
            "图像理解调用失败（{who}）：{TEMPLATE_PHRASE_IMAGE_TOO_LARGE}。{detail}。\
             出路：压缩或裁剪图片后重试。"
        ),
        VisionErrorKind::Other => format!(
            "图像理解调用失败（{who}）。{detail}。\
             出路：可稍后重试；若持续失败请检查该模型与服务商配置。"
        ),
    }
}

/// UI 横幅文案（一次性 island warning 提醒）——与 Leader 透传文案分开措辞：
/// 面向用户讲清「坏了什么 + 去哪修」，不携带原始 HTTP 细节。
/// 仅 Auth/Billing 返回 Some（重试无意义才值得弹横幅打断）；其它分类返回 None。
pub fn vision_banner_message(kind: VisionErrorKind) -> Option<String> {
    match kind {
        VisionErrorKind::Auth => Some(
            "图像理解模型鉴权失败（API 密钥无效或未授权），截图与图片识别暂不可用\
             ——请到 设置 → 模型 → 图像音频模型 检查 API Key。"
                .to_string(),
        ),
        VisionErrorKind::Billing => Some(
            "图像理解模型余额不足或配额已用尽，截图与图片识别暂不可用\
             ——请到对应服务商充值或购买资源包后重试。"
                .to_string(),
        ),
        _ => None,
    }
}

/// 从统一传输层错误抽取 (HTTP status, 明文 body)——分类与文案的输入。
/// 非结构化变体取内层明文，避免 `NuphusError::LLM` 的「LLM error: 」Display 前缀
/// 混进单层文案（三层前缀收敛为此处的唯一一层）。
fn split_transport_error(err: &crate::NuphusError) -> (Option<u16>, String) {
    match err {
        crate::NuphusError::LLM(crate::LLMError::ApiError { status, body }) => {
            (Some(*status), body.clone())
        }
        crate::NuphusError::LLM(crate::LLMError::RequestFailed { error }) => (None, error.clone()),
        crate::NuphusError::LLM(crate::LLMError::StreamError { error }) => (None, error.clone()),
        crate::NuphusError::LLM(crate::LLMError::Other(s)) => (None, s.clone()),
        other => (None, other.to_string()),
    }
}

/// 使用统一判定获取视觉模型 ID（desktop_vision 工具与描述注入共用）
fn resolve_vision_model_id() -> Result<String, String> {
    match resolve_vision_strategy() {
        VisionStrategy::Main => {
            let registry = config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
            Ok(registry.model.clone())
        }
        VisionStrategy::Capability(m) => Ok(m),
        // 不可用时给**真实的、可执行**的说明：讲清原因（Leader 不支持视觉）+ 两条出路。
        // 这段文本会随工具结果进入 Leader 上下文——是「当前能力确实不可用」的事实告知，
        // 不是静默降级：Leader 看到后应如实告诉用户，而不是假装看到了图像内容。
        VisionStrategy::None => {
            let leader = config::load_registry().map(|r| r.model).unwrap_or_default();
            let leader_part = if leader.is_empty() {
                "当前 Leader 模型不支持视觉输入".to_string()
            } else {
                format!("当前 Leader 模型（{leader}）不支持视觉输入")
            };
            Err(format!(
                "图像理解不可用：{leader_part}，且未配置图像理解模型。\n\
                 请在 Nuphus 设置 → 模型 → 图像音频模型 中指定一个图像理解模型，或把 Leader 切换到支持视觉的模型。\n\
                 在解决之前，涉及截图/图像识别的桌面操作无法完成——请如实告知用户，不要推测图像内容。"
            ))
        }
    }
}

/// 解析 data URL 为 (mime_type, base64 载荷)
fn split_data_url(data_url: &str) -> Result<(String, String), String> {
    // 格式: data:<mime>;base64,<payload>
    let (header, payload) = data_url
        .split_once(',')
        .ok_or_else(|| "Invalid data URL: no comma found".to_string())?;
    let mime_type = header
        .trim_start_matches("data:")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .to_string();
    Ok((mime_type, payload.to_string()))
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transports::StreamEvent;

    fn error_ctx(follow_leader: bool) -> VisionErrorContext {
        VisionErrorContext {
            model: "glm-5.3-flash".to_string(),
            provider: "custom-workbuddy-intl".to_string(),
            follow_leader,
        }
    }

    #[test]
    fn classify_status_codes() {
        assert_eq!(classify_vision_error(Some(401), ""), VisionErrorKind::Auth);
        assert_eq!(classify_vision_error(Some(403), ""), VisionErrorKind::Auth);
        assert_eq!(
            classify_vision_error(Some(429), ""),
            VisionErrorKind::RateLimit
        );
        assert_eq!(
            classify_vision_error(Some(413), ""),
            VisionErrorKind::ImageTooLarge
        );
        assert_eq!(
            classify_vision_error(Some(500), "internal error"),
            VisionErrorKind::Other
        );
        assert_eq!(classify_vision_error(None, ""), VisionErrorKind::Other);
    }

    #[test]
    fn classify_billing_keywords_beat_429() {
        // OpenAI insufficient_quota 走 429，但语义是计费（重试无意义）
        assert_eq!(
            classify_vision_error(
                Some(429),
                r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#
            ),
            VisionErrorKind::Billing
        );
        assert_eq!(
            classify_vision_error(None, "账户余额不足，请充值"),
            VisionErrorKind::Billing
        );
        assert_eq!(
            classify_vision_error(None, "Insufficient Balance"),
            VisionErrorKind::Billing
        );
    }

    #[test]
    fn classify_auth_and_rate_limit_keywords() {
        assert_eq!(
            classify_vision_error(None, "Invalid API key provided"),
            VisionErrorKind::Auth
        );
        assert_eq!(
            classify_vision_error(None, "密钥无效，鉴权失败"),
            VisionErrorKind::Auth
        );
        assert_eq!(
            classify_vision_error(Some(400), "rate limit reached for requests"),
            VisionErrorKind::RateLimit
        );
        assert_eq!(
            classify_vision_error(None, "Too Many Requests"),
            VisionErrorKind::RateLimit
        );
    }

    #[test]
    fn classify_image_too_large_keywords() {
        assert_eq!(
            classify_vision_error(None, "image too large: max 5MB"),
            VisionErrorKind::ImageTooLarge
        );
        assert_eq!(
            classify_vision_error(Some(400), "图片过大，请压缩后上传"),
            VisionErrorKind::ImageTooLarge
        );
    }

    #[test]
    fn format_single_layer_contains_context_and_guidance() {
        let text = format_vision_error(
            VisionErrorKind::Auth,
            &error_ctx(true),
            "invalid api key",
            Some(401),
        );
        // 单层：无「视觉模型请求失败：」「视觉模型调用失败：」叠加前缀
        assert!(!text.contains("视觉模型请求失败"));
        assert!(!text.contains("视觉模型调用失败"));
        // 模型 + provider + 跟随态 + 分类出路
        assert!(text.contains("glm-5.3-flash"));
        assert!(text.contains("custom-workbuddy-intl"));
        assert!(text.contains("跟随 Leader"));
        assert!(text.contains("API Key"));
        assert!(text.starts_with("图像理解调用失败"));
    }

    #[test]
    fn format_explicit_binding_has_no_follow_marker() {
        let text = format_vision_error(
            VisionErrorKind::Billing,
            &error_ctx(false),
            "insufficient balance",
            None,
        );
        assert!(!text.contains("跟随 Leader"));
        assert!(text.contains("充值"));
    }

    #[test]
    fn format_then_classify_round_trips_for_all_kinds() {
        // 横幅升级依赖「最终文案 → 分类」往返一致；限流/图片过大文案故意携带
        // quota/balance 噪声，验证模板判别短语优先、不被 raw body 劫持。
        // 注：Other 无专属短语，但其性质是「输入 body 不含任何分类关键词才落到
        // Other」——带余额/quota 字样的 body 本就会分类为 Billing，不构成 Other 用例。
        let cases = [
            (VisionErrorKind::Auth, "unauthorized"),
            (VisionErrorKind::Billing, "billing error"),
            (
                VisionErrorKind::RateLimit,
                "quota balance 余额 充值 rate limit",
            ),
            (VisionErrorKind::ImageTooLarge, "余额 balance"),
            (VisionErrorKind::Other, "connection reset by peer"),
        ];
        for (kind, hostile_body) in cases {
            let text = format_vision_error(kind, &error_ctx(true), hostile_body, Some(500));
            assert_eq!(classify_vision_error_text(&text), kind, "kind={kind:?}");
        }
    }

    #[test]
    fn classify_text_falls_back_to_keywords_for_non_template_text() {
        // 非模板文本（超时 / None 策略说明）不带判别短语 → 走通用关键词分类
        assert_eq!(
            classify_vision_error_text("视觉模型超时（120秒）"),
            VisionErrorKind::Other
        );
        assert_eq!(
            classify_vision_error_text("图像理解不可用：当前 Leader 模型不支持视觉输入"),
            VisionErrorKind::Other
        );
        assert_eq!(
            classify_vision_error_text("上游返回：Invalid API key"),
            VisionErrorKind::Auth
        );
    }

    #[test]
    fn kind_names_are_stable_contract() {
        assert_eq!(vision_error_kind_name(VisionErrorKind::Auth), "auth");
        assert_eq!(vision_error_kind_name(VisionErrorKind::Billing), "billing");
        assert_eq!(
            vision_error_kind_name(VisionErrorKind::RateLimit),
            "rate_limit"
        );
        assert_eq!(
            vision_error_kind_name(VisionErrorKind::ImageTooLarge),
            "image_too_large"
        );
        assert_eq!(vision_error_kind_name(VisionErrorKind::Other), "other");
    }

    #[test]
    fn banner_message_only_for_auth_and_billing() {
        assert!(vision_banner_message(VisionErrorKind::Auth).is_some());
        assert!(vision_banner_message(VisionErrorKind::Billing).is_some());
        assert!(vision_banner_message(VisionErrorKind::RateLimit).is_none());
        assert!(vision_banner_message(VisionErrorKind::ImageTooLarge).is_none());
        assert!(vision_banner_message(VisionErrorKind::Other).is_none());
    }

    #[test]
    fn split_transport_error_extracts_api_error_fields_without_display_prefix() {
        let err = crate::NuphusError::LLM(crate::LLMError::ApiError {
            status: 429,
            body: "rate limit reached".to_string(),
        });
        let (status, body) = split_transport_error(&err);
        assert_eq!(status, Some(429));
        assert_eq!(body, "rate limit reached");
        // 非结构化错误取内层明文，不带「LLM error: 」前缀
        let err = crate::NuphusError::LLM(crate::LLMError::RequestFailed {
            error: "connection timed out".to_string(),
        });
        let (status, body) = split_transport_error(&err);
        assert_eq!(status, None);
        assert_eq!(body, "connection timed out");
        assert!(!body.contains("LLM error"));
    }

    fn duplicate_model_registry() -> config::ModelRegistry {
        toml::from_str(
            r#"
[[providers]]
name = "first"
provider_type = "custom"
api_key = "first-key"
base_url = "https://first.example/v1"

[[providers.models]]
id = "shared-model"

[[providers]]
name = "selected"
provider_type = "custom"
api_key = "selected-key"
base_url = "https://selected.example/v1"

[[providers.models]]
id = "shared-model"
"#,
        )
        .unwrap()
    }

    #[test]
    fn vision_model_uses_configured_provider_for_duplicate_ids() {
        let registry = duplicate_model_registry();
        let (provider, model) =
            resolve_vision_model(&registry, "shared-model", Some("selected")).unwrap();
        assert_eq!(provider.name, "selected");
        assert_eq!(model.id, "shared-model");
    }

    #[test]
    fn vision_model_keeps_legacy_first_match_without_provider() {
        let registry = duplicate_model_registry();
        let (provider, _) = resolve_vision_model(&registry, "shared-model", None).unwrap();
        assert_eq!(provider.name, "first");
    }

    #[test]
    fn events_join_all_text_deltas() {
        let events = vec![
            StreamEvent::Reasoning("thinking".to_string()),
            StreamEvent::TextDelta("第一行".to_string()),
            StreamEvent::TextDelta("\n第二行".to_string()),
            StreamEvent::Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_hit_tokens: 0,
            },
            StreamEvent::Done,
        ];
        assert_eq!(events_to_text(events).unwrap(), "第一行\n第二行");
    }

    #[test]
    fn events_error_event_is_not_swallowed() {
        let events = vec![
            StreamEvent::TextDelta("部分".to_string()),
            StreamEvent::Error("HTTP 400: bad request".to_string()),
        ];
        assert_eq!(events_to_text(events).unwrap_err(), "HTTP 400: bad request");
    }

    #[test]
    fn events_empty_text_is_an_error() {
        let events = vec![StreamEvent::Done];
        assert!(events_to_text(events).is_err());
    }
}
