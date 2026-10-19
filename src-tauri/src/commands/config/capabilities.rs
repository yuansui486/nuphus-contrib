//! Capabilities, refinement, and tool permission commands.

use super::toml_ops::get_config_path;
use crate::state::AppState;
use tauri::State;
#[tauri::command(async)]
pub fn get_tool_permissions(state: State<'_, AppState>) -> Result<String, String> {
    let perms = state.runtime.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::to_string(&perms.tool_permissions).unwrap_or_default())
}

#[tauri::command(async)]
pub fn set_capability(
    key: String,
    value: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let _config_write = nuphus::config::lock_provider_config();
    // Use providers.toml as canonical location (same as get_capabilities)
    let config_path = {
        let providers_path = state.llm_config_path.with_file_name("providers.toml");
        if providers_path.exists() {
            providers_path
        } else {
            get_config_path().ok_or_else(|| "Unable to locate config file".to_string())?
        }
    };

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config.toml: {}", e))?;

    let mut doc: toml::Value = content
        .parse()
        .unwrap_or_else(|_| toml::Value::Table(toml::value::Table::new()));

    // Get or create [capabilities] table
    if doc.get("capabilities").is_none() {
        let table = doc
            .as_table_mut()
            .ok_or_else(|| "config.toml root is not a table".to_string())?;
        table.insert(
            "capabilities".to_string(),
            toml::Value::Table(toml::value::Table::new()),
        );
    }
    let caps = doc
        .get_mut("capabilities")
        .and_then(|v| v.as_table_mut())
        .ok_or_else(|| "Cannot create [capabilities] table".to_string())?;

    caps.insert(
        key.clone(),
        if let Ok(n) = value.parse::<i64>() {
            toml::Value::Integer(n)
        } else {
            toml::Value::String(value)
        },
    );

    nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
    let new_content =
        toml::to_string_pretty(&doc).map_err(|e| format!("Failed to serialize config: {}", e))?;
    nuphus::config::write_provider_config(&config_path, &new_content)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;

    Ok(())
}

/// 原子设置能力模型绑定：`{kind}` 与 `{kind}_provider` 必须一起落盘。
///
/// 前端原先分两次调用 `set_capability(kind, …)` / `set_capability(kind_provider, …)`；
/// 第二次失败就会留下「新 model + 旧 provider」的中间态 —— 后端按
/// provider + model 精确解析时找不到该组合，能力请求直接失败，而 UI 已经提示成功。
/// 这里收敛为一次读写：要么两个字段都更新，要么都不动。
///
/// `kind` ∈ { vision, stt, tts, voice, image_generation, video_generation }。
#[tauri::command(async)]
pub fn set_capability_binding(
    kind: String,
    model: String,
    provider: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    const KINDS: [&str; 7] = [
        "vision",
        "stt",
        "tts",
        "voice",
        "image_generation",
        "video_generation",
        // 网页搜索源：值为 provider 段名（不是模型 id），见
        // `nuphus::config::Capabilities::web_search_source`。
        "web_search_source",
    ];
    if !KINDS.contains(&kind.as_str()) {
        return Err(format!("未知能力类型: {kind}"));
    }

    let config_path = {
        let providers_path = state.llm_config_path.with_file_name("providers.toml");
        if providers_path.exists() {
            providers_path
        } else {
            get_config_path().ok_or_else(|| "Unable to locate config file".to_string())?
        }
    };

    super::toml_ops::set_capability_in_config_toml(&config_path, &kind, &model, &provider)?;

    tracing::info!(
        "[set_capability_binding] kind={} model={} provider={}",
        kind,
        model,
        provider
    );
    Ok(())
}

/// 图像理解生效路径解析（只读，C 方向：跟随链透明化）。
///
/// 与 `resolve_vision_strategy` 同源的三态：capability（显式绑定）/ main（跟随
/// Leader 主模型）/ none（不可用）。设置页「图像理解」区据此渲染只读行
/// 「跟随 Leader → 模型 @ provider 段名」，让用户知道留空时实际是谁在理解图片。
/// 只读查询：不改 set_capability / set_capability_binding 的任何写入语义。
#[derive(Debug, serde::Serialize)]
pub struct VisionResolution {
    /// "capability" | "main" | "none"
    pub strategy: String,
    /// capability = 绑定的模型 id；main = Leader 模型 id；none = ""
    pub model: String,
    /// 模型所在 provider 段名（解析不到时为空串）
    pub provider: String,
    /// 当前 Leader 模型 id（前端展示跟随链用）
    pub leader_model: String,
}

#[tauri::command(async)]
pub fn get_vision_resolution() -> Result<VisionResolution, String> {
    let registry = nuphus::config::load_registry().map_err(|e| format!("加载模型配置失败: {e}"))?;
    let leader_model = registry.model.clone();

    let resolution = match nuphus::config::resolve_vision_strategy() {
        nuphus::config::VisionStrategy::Capability(model) => {
            // provider 归属：显式 vision_provider 优先；旧配置无该字段时按
            // 模型 id 首段命中兼容解析（与 vision_ocr 的 resolve_vision_model 同口径）
            let provider = nuphus::config::resolve_vision_provider()
                .filter(|p| !p.is_empty())
                .or_else(|| {
                    registry
                        .find_model(&model)
                        .map(|(provider, _)| provider.name.clone())
                })
                .unwrap_or_default();
            VisionResolution {
                strategy: "capability".to_string(),
                model,
                provider,
                leader_model,
            }
        }
        nuphus::config::VisionStrategy::Main => {
            // 成对权威解析（resolve_main_binding）；解析失败（绑定陈旧等）降级为
            // 只给模型 id——跟随链仍可展示，不因 provider 缺失整个报错
            let (provider, model) = registry
                .resolve_main_binding()
                .unwrap_or_else(|_| (String::new(), leader_model.clone()));
            VisionResolution {
                strategy: "main".to_string(),
                model,
                provider,
                leader_model,
            }
        }
        nuphus::config::VisionStrategy::None => VisionResolution {
            strategy: "none".to_string(),
            model: String::new(),
            provider: String::new(),
            leader_model,
        },
    };
    tracing::debug!("[get_vision_resolution] {:?}", resolution);
    Ok(resolution)
}

/// 设置页「发送测试图片」探测结果（D 方向）。
#[derive(Debug, serde::Serialize)]
pub struct VisionTestOutcome {
    pub ok: bool,
    pub latency_ms: u64,
    /// 成功 = "ok"；失败 = 复用 classify_vision_error 的分类名
    /// （auth / billing / rate_limit / image_too_large / other）
    pub kind: String,
    pub message: String,
}

/// 1x1 透明 PNG（最小合法 PNG）——探测整条视觉链路的固定测试图，
/// data URL 约 130B，加上请求头/提示词整包约 1KB（前端按钮旁照此标注）。
const VISION_PROBE_PNG_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// 图像理解模型连通性探测（D 方向）：内嵌 1x1 PNG 直调 vision_ocr_data_url
/// （复用 desktop_vision 的完整调用链与传输层，不经桌面截图路径），超时口径与
/// client.ocr 的 120s 一致。async command：tauri 在线程池执行，不阻塞主线程。
///
/// 失败不抛 Err（前端按 ok=false 渲染分类文案），Err 仅留给意外 panic 场景；
/// 返回的 kind 复用方向1的 classify_vision_error_text（对最终透传文案判别）。
#[tauri::command]
pub async fn test_vision_model() -> Result<VisionTestOutcome, String> {
    let data_url = format!("data:image/png;base64,{VISION_PROBE_PNG_BASE64}");
    let started = std::time::Instant::now();
    let call = nuphus::desktop::vision_ocr::vision_ocr_data_url(
        &data_url,
        Some("这是一张 1x1 像素的连通性测试图片，请只回复两个字符：OK"),
    );
    // 与 client.ocr 的视觉超时同一口径：推理系视觉模型单次可达 50s+
    let outcome = match tokio::time::timeout(std::time::Duration::from_secs(120), call).await {
        Ok(Ok(reply)) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            tracing::info!("[test_vision_model] ok in {latency_ms}ms");
            VisionTestOutcome {
                ok: true,
                latency_ms,
                kind: "ok".to_string(),
                message: reply.trim().chars().take(80).collect(),
            }
        }
        Ok(Err(message)) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            let kind = nuphus::desktop::vision_ocr::classify_vision_error_text(&message);
            let kind_name = nuphus::desktop::vision_ocr::vision_error_kind_name(kind);
            tracing::warn!("[test_vision_model] failed ({kind_name}) in {latency_ms}ms");
            VisionTestOutcome {
                ok: false,
                latency_ms,
                kind: kind_name.to_string(),
                message,
            }
        }
        Err(_) => {
            let latency_ms = started.elapsed().as_millis() as u64;
            VisionTestOutcome {
                ok: false,
                latency_ms,
                kind: "other".to_string(),
                message: "视觉模型超时（120秒）".to_string(),
            }
        }
    };
    Ok(outcome)
}

#[tauri::command(async)]
pub fn get_session_refine_config(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let rt = state.runtime.lock().map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        // 大窗口强制线（仅 RefineTier::Large 生效）；附带可调范围供前端渲染 slider
        "force_threshold": rt.large_force_refine_threshold,
        "force_min": nuphus::agent::distill::LARGE_FORCE_MIN,
        "force_max": nuphus::agent::distill::LARGE_FORCE_MAX,
    }))
}

/// Set the large-window force-refine ratio.
///
/// Only meaningful for `RefineTier::Large` (context_window > 600K); Small/Medium
/// tiers use their fixed ratios and ignore this value. Rejects out-of-range input
/// rather than silently clamping — a bad value here means the caller's UI and the
/// backend disagree, which is worth surfacing instead of hiding.
#[tauri::command(async)]
pub fn set_session_refine_config(
    state: State<'_, AppState>,
    force_threshold: Option<f64>,
) -> Result<String, String> {
    if let Some(th) = force_threshold {
        let min = nuphus::agent::distill::LARGE_FORCE_MIN;
        let max = nuphus::agent::distill::LARGE_FORCE_MAX;
        if !(min..=max).contains(&th) {
            return Err(format!("force_threshold must be between {} ~ {}", min, max));
        }
        let mut guard = state.runtime.lock().map_err(|e| e.to_string())?;
        guard.large_force_refine_threshold = th;
        tracing::info!("set_session_refine_config: force_threshold={}", th);
        // 落盘：内存 guard 随进程消失，不持久化等于「用户调了但下次启动悄悄退回
        // 默认 50%」。先落盘再返回——写失败要让调用方看到（滑杆会弹错误提示），
        // 否则又变成「显示已生效、实际没保存」。
        crate::commands::config::toml_ops::write_refine_force_threshold_to_config_toml(th)?;
    }
    Ok("Refinement config updated".to_string())
}
