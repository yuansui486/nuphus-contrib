//! Best-effort capability discovery. Local activation never awaits this work.
//! Lock order: providers config -> runtime. Neither lock may cross an await.

use super::{llm, toml_ops};
use crate::models::aggregator;
use crate::state::{AppState, LlamaConfig, RuntimeContext};
use nuphus::config::{lock_provider_config, registry::ProviderRegistry};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use tauri::{Emitter, Manager};

#[derive(Clone, Hash, PartialEq, Eq)]
struct JobKey {
    provider: String,
    model: String,
    connection: u64,
}

#[derive(Clone, Default)]
struct Discovery {
    context: Option<usize>,
    vision: Option<bool>,
    efforts: Vec<String>,
    default_effort: Option<String>,
}

pub(crate) struct MetadataTasks {
    // A repeated selection subscribes the latest generation to the same job.
    pending: HashMap<JobKey, (u64, bool)>,
    cache: HashMap<JobKey, Discovery>,
    permits: std::sync::Arc<tokio::sync::Semaphore>,
}

impl Default for MetadataTasks {
    fn default() -> Self {
        Self {
            pending: HashMap::new(),
            cache: HashMap::new(),
            permits: std::sync::Arc::new(tokio::sync::Semaphore::new(2)),
        }
    }
}

impl MetadataTasks {
    fn subscribe(&mut self, key: &JobKey, generation: u64, vision: bool) -> bool {
        if let Some(pending) = self.pending.get_mut(key) {
            pending.0 = generation;
            pending.1 |= vision;
            false
        } else {
            self.pending.insert(key.clone(), (generation, vision));
            true
        }
    }

    fn finish(&mut self, key: &JobKey, result: Discovery) -> Option<u64> {
        // Bound process-local cache growth; credentials/fingerprints are never logged.
        if self.cache.len() >= 128 {
            self.cache.clear();
        }
        self.cache.insert(key.clone(), result);
        self.pending.remove(key).map(|pending| pending.0)
    }
}

fn read_doc(path: &Path) -> Option<toml::Value> {
    std::fs::read_to_string(path).ok()?.parse().ok()
}

fn segment<'a>(doc: &'a toml::Value, provider: &str) -> Option<&'a toml::Value> {
    doc.get("providers")?
        .as_array()?
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(provider))
}

fn model_entry<'a>(segment: &'a toml::Value, model: &str) -> Option<&'a toml::Value> {
    segment
        .get("models")?
        .as_array()?
        .iter()
        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(model))
}

fn context(entry: Option<&toml::Value>) -> Option<usize> {
    entry?
        .get("context_window")?
        .as_integer()
        .filter(|n| *n > 0)
        .map(|n| n as usize)
}

// Ignore model metadata and display labels, but invalidate for endpoint, protocol,
// headers, credentials and OAuth changes. Decrypt before hashing: DPAPI can encode
// the same key differently on each save. This fingerprint stays in memory only.
fn connection(segment: &toml::Value, cfg: &LlamaConfig) -> Option<u64> {
    let mut fields = segment.as_table()?.clone();
    fields.remove("models");
    fields.remove("display_name");
    if let Some(raw) = fields.get("api_key").and_then(|v| v.as_str()) {
        let plain = nuphus::cookies::decrypt_secret(raw).unwrap_or_default();
        fields.insert("api_key".into(), toml::Value::String(plain));
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    toml::to_string(&fields).ok()?.hash(&mut hash);
    cfg.base_url.hash(&mut hash);
    cfg.api_key.hash(&mut hash);
    Some(hash.finish())
}

fn builtin_context(cfg: &LlamaConfig) -> Option<usize> {
    ProviderRegistry::builtin()
        .get(&cfg.provider)
        .and_then(|p| {
            p.models()
                .iter()
                .find(|m| m.id == cfg.model)
                .map(|m| m.context_window as usize)
        })
        .filter(|n| *n > 0)
}

/// Only local work: explicit/persisted values win, builtin is runtime-only, 0 is unknown.
pub(super) fn activate(
    state: &AppState,
    cfg: &LlamaConfig,
    explicit: Option<usize>,
) -> Result<u64, String> {
    if let Some(value) = explicit {
        toml_ops::update_model_context_window(
            &state.llm_config_path,
            &cfg.provider,
            &cfg.model,
            value,
        )?;
    }
    let _config_write = lock_provider_config();
    let doc = read_doc(&state.llm_config_path);
    let existing = doc
        .as_ref()
        .and_then(|d| segment(d, &cfg.provider))
        .and_then(|p| context(model_entry(p, &cfg.model)));
    let window = explicit
        .or(existing)
        .or_else(|| builtin_context(cfg))
        .unwrap_or(0);
    let mut runtime = state.runtime.lock().map_err(|e| e.to_string())?;
    runtime.model_generation = runtime.model_generation.wrapping_add(1);
    runtime.llm_config = Some(cfg.clone());
    runtime.model_context_window = window;
    runtime.model_context_explicit = explicit;
    Ok(runtime.model_generation)
}

fn is_current(runtime: &RuntimeContext, cfg: &LlamaConfig, generation: u64) -> bool {
    runtime.model_generation == generation
        && runtime.llm_config.as_ref().is_some_and(|current| {
            current.provider == cfg.provider
                && current.model == cfg.model
                && current.base_url == cfg.base_url
                && current.api_key == cfg.api_key
        })
}

pub(super) fn schedule<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
) {
    schedule_inner(app, cfg, generation, true);
}

/// Startup historically calibrated context only; it must not send a new paid
/// vision request. A later user switch can subscribe to/upgrade this same job.
pub(super) fn schedule_context_calibration<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
) {
    schedule_inner(app, cfg, generation, false);
}

fn schedule_inner<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cfg: LlamaConfig,
    generation: u64,
    vision: bool,
) {
    let state = app.state::<AppState>();
    let snapshot = {
        let _config_write = lock_provider_config();
        read_doc(&state.llm_config_path).and_then(|doc| segment(&doc, &cfg.provider).cloned())
    };
    let Some(snapshot) = snapshot else { return };
    let Some(connection) = connection(&snapshot, &cfg) else {
        return;
    };
    let key = JobKey {
        provider: cfg.provider.clone(),
        model: cfg.model.clone(),
        connection,
    };
    let (cached, permits) = {
        let Ok(mut runtime) = state.runtime.lock() else {
            return;
        };
        if !is_current(&runtime, &cfg, generation) {
            return;
        }
        if !runtime.metadata_tasks.subscribe(&key, generation, vision) {
            return;
        }
        (
            runtime
                .metadata_tasks
                .cache
                .get(&key)
                .cloned()
                .unwrap_or_default(),
            runtime.metadata_tasks.permits.clone(),
        )
    };
    let path = state.llm_config_path.clone();
    tauri::async_runtime::spawn(async move {
        let Ok(_permit) = permits.acquire_owned().await else {
            return;
        };
        // Discard queued work for selections that are no longer current.
        {
            let state = app.state::<AppState>();
            let Ok(mut runtime) = state.runtime.lock() else {
                return;
            };
            let latest = runtime
                .metadata_tasks
                .pending
                .get(&key)
                .map(|pending| pending.0);
            if !latest.is_some_and(|generation| is_current(&runtime, &cfg, generation)) {
                runtime.metadata_tasks.pending.remove(&key);
                return;
            }
        }
        let mut result = discover_context(&cfg, &snapshot, &path, cached).await;
        let state = app.state::<AppState>();
        let vision = state
            .runtime
            .lock()
            .ok()
            .and_then(|runtime| {
                runtime
                    .metadata_tasks
                    .pending
                    .get(&key)
                    .filter(|pending| is_current(&runtime, &cfg, pending.0))
                    .map(|pending| pending.1)
            })
            .unwrap_or(false);
        if vision {
            result = discover_vision(&cfg, &snapshot, &path, result).await;
        }
        let latest = state
            .runtime
            .lock()
            .ok()
            .and_then(|mut runtime| runtime.metadata_tasks.finish(&key, result.clone()));
        if let Some(generation) = latest {
            match commit(&state, &cfg, &snapshot, &key, generation, &result) {
                Ok(true) => {
                    let _ = app.emit(
                        "model-metadata-updated",
                        serde_json::json!({
                            "provider": cfg.provider, "model": cfg.model,
                        }),
                    );
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!("[model-metadata] could not persist discovery: {error}")
                }
            }
        }
    });
}

async fn discover_context(
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    path: &Path,
    mut result: Discovery,
) -> Discovery {
    if cfg.provider == "local" {
        return result;
    }
    let protocol = snapshot
        .get("provider_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&cfg.provider);
    let registry = ProviderRegistry::builtin();
    let (header, prefix) = registry
        .get(protocol)
        .map(|p| (p.auth_header().to_string(), p.auth_prefix().to_string()))
        .unwrap_or(("authorization".into(), "Bearer ".into()));
    if context(model_entry(snapshot, &cfg.model)).is_none() && result.context.is_none() {
        let (probe_cfg, header, prefix) = (cfg.clone(), header.clone(), prefix.clone());
        if let Ok(Some(meta)) = tokio::task::spawn_blocking(move || {
            llm::query_model_metadata_from_api(
                &probe_cfg.base_url,
                &probe_cfg.model,
                &probe_cfg.api_key,
                &header,
                &prefix,
            )
        })
        .await
        {
            result.context = meta.context_length.filter(|n| *n > 0);
            result.efforts = meta.reasoning_efforts;
            result.default_effort = meta.default_effort;
        }
        if result.context.is_none() && aggregator::has_vendor(&cfg.provider) {
            if let Some(dir) = path.parent() {
                let entries = aggregator::ensure_cache(&aggregator::cache_path(dir)).await;
                result.context = aggregator::lookup(&entries, &cfg.provider, &cfg.model)
                    .and_then(|entry| entry.context_length)
                    .map(|n| n as usize)
                    .filter(|n| *n > 0);
            }
        }
    }
    result
}

async fn discover_vision(
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    path: &Path,
    mut result: Discovery,
) -> Discovery {
    if cfg.provider == "local" {
        return result;
    }
    let protocol = snapshot
        .get("provider_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&cfg.provider);
    let registry = ProviderRegistry::builtin();
    let (header, prefix) = registry
        .get(protocol)
        .map(|p| (p.auth_header().to_string(), p.auth_prefix().to_string()))
        .unwrap_or(("authorization".into(), "Bearer ".into()));
    let user_vision = {
        let _config_write = lock_provider_config();
        toml_ops::model_has_user_vision_override(path, &cfg.provider, &cfg.model)
    };
    if !user_vision && result.vision.is_none() {
        result.vision = registry.get(&cfg.provider).and_then(|p| {
            p.models()
                .iter()
                .find(|m| m.id == cfg.model)
                .map(|m| m.supports_vision)
        });
        // The existing probe uses OpenAI chat/completions; never send it to Anthropic.
        if result.vision.is_none() && protocol != "anthropic" {
            let cfg = cfg.clone();
            result.vision = tokio::task::spawn_blocking(move || {
                llm::probe_vision(&cfg.base_url, &cfg.model, &cfg.api_key, &header, &prefix)
            })
            .await
            .ok()
            .flatten();
        }
    }
    result
}

// Caller holds the shared config transaction lock. Merge into freshly-read fields,
// never the old snapshot. This is also the final manual-override check.
fn merge(
    doc: &mut toml::Value,
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    key: &JobKey,
    result: &Discovery,
) -> Option<(bool, Option<usize>)> {
    let provider = segment(doc, &cfg.provider)?;
    if connection(provider, cfg)? != key.connection {
        return None;
    }
    if model_entry(provider, &cfg.model).is_none() {
        return model_entry(snapshot, &cfg.model)
            .is_none()
            .then_some((false, result.context));
    }
    let entry = doc
        .get_mut("providers")?
        .as_array_mut()?
        .iter_mut()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(cfg.provider.as_str()))?
        .get_mut("models")?
        .as_array_mut()?
        .iter_mut()
        .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(cfg.model.as_str()))?;
    let original = entry.clone();
    let existing = context(Some(entry));
    let fields = entry.as_table_mut()?;
    if existing.is_none() {
        if let Some(value) = result.context {
            fields.insert("context_window".into(), toml::Value::Integer(value as i64));
        }
    }
    if fields
        .get(toml_ops::VISION_SOURCE_KEY)
        .and_then(|v| v.as_str())
        != Some("user")
    {
        if let Some(value) = result.vision {
            fields.insert("supports_vision".into(), toml::Value::Boolean(value));
        }
    }
    // Provider refresh or a later manual edit owns fields changed since dispatch.
    let old = model_entry(snapshot, &cfg.model);
    for (field, value) in [
        (
            "reasoning_efforts",
            (!result.efforts.is_empty()).then(|| {
                toml::Value::Array(
                    result
                        .efforts
                        .iter()
                        .cloned()
                        .map(toml::Value::String)
                        .collect(),
                )
            }),
        ),
        (
            "default_effort",
            result.default_effort.clone().map(toml::Value::String),
        ),
    ] {
        if original.get(field) == old.and_then(|m| m.get(field)) {
            if let Some(value) = value {
                fields.insert(field.into(), value);
            }
        }
    }
    Some((*entry != original, existing.or(result.context)))
}

fn commit(
    state: &AppState,
    cfg: &LlamaConfig,
    snapshot: &toml::Value,
    key: &JobKey,
    generation: u64,
    result: &Discovery,
) -> Result<bool, String> {
    let _config_write = lock_provider_config();
    let Some(mut doc) = read_doc(&state.llm_config_path) else {
        return Ok(false);
    };
    let Some((changed, window)) = merge(&mut doc, cfg, snapshot, key, result) else {
        return Ok(false);
    };
    if changed {
        nuphus::cookies::encrypt_plaintext_provider_keys(&mut doc);
        let content = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        nuphus::config::write_provider_config(&state.llm_config_path, &content)
            .map_err(|e| e.to_string())?;
    }
    let mut runtime = state.runtime.lock().map_err(|e| e.to_string())?;
    let mut runtime_changed = false;
    if is_current(&runtime, cfg, generation) {
        if let Some(window) = runtime.model_context_explicit.or(window) {
            runtime_changed = runtime.model_context_window != window;
            runtime.model_context_window = window;
        }
    }
    Ok(changed || runtime_changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (LlamaConfig, toml::Value, toml::Value, JobKey) {
        let cfg = LlamaConfig {
            provider: "custom-a".into(),
            model: "m".into(),
            base_url: "https://example.test/v1".into(),
            ..Default::default()
        };
        let doc: toml::Value = "[[providers]]\nname='custom-a'\nbase_url='https://example.test/v1'\n[[providers.models]]\nid='m'\n".parse().unwrap();
        let snapshot = segment(&doc, &cfg.provider).unwrap().clone();
        let key = JobKey {
            provider: cfg.provider.clone(),
            model: cfg.model.clone(),
            connection: connection(&snapshot, &cfg).unwrap(),
        };
        (cfg, doc, snapshot, key)
    }

    #[test]
    fn duplicate_jobs_follow_latest_generation_and_cache_success() {
        let (_, _, _, key) = fixture();
        let mut tasks = MetadataTasks::default();
        assert!(tasks.subscribe(&key, 1, true));
        assert!(!tasks.subscribe(&key, 3, true));
        assert_eq!(
            tasks.finish(
                &key,
                Discovery {
                    vision: Some(true),
                    ..Default::default()
                }
            ),
            Some(3)
        );
        assert_eq!(tasks.cache[&key].vision, Some(true));
        assert!(tasks.subscribe(&key, 4, true));
    }

    #[test]
    fn stale_binding_or_generation_cannot_change_runtime() {
        let (cfg, _, _, _) = fixture();
        let mut runtime = RuntimeContext {
            llm_config: Some(cfg.clone()),
            model_generation: 2,
            ..Default::default()
        };
        assert!(!is_current(&runtime, &cfg, 1));
        assert!(is_current(&runtime, &cfg, 2));
        runtime.llm_config.as_mut().unwrap().model = "another".into();
        assert!(!is_current(&runtime, &cfg, 2));
    }

    #[test]
    fn late_discovery_preserves_manual_values_and_unrelated_fields() {
        let (cfg, mut doc, snapshot, key) = fixture();
        let model = doc["providers"][0]["models"][0].as_table_mut().unwrap();
        model.insert("context_window".into(), toml::Value::Integer(999));
        model.insert("supports_vision".into(), toml::Value::Boolean(false));
        model.insert(
            toml_ops::VISION_SOURCE_KEY.into(),
            toml::Value::String("user".into()),
        );
        model.insert("note".into(), toml::Value::String("keep".into()));
        let result = Discovery {
            context: Some(123),
            vision: Some(true),
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((false, Some(999)))
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["supports_vision"].as_bool(),
            Some(false)
        );
        assert_eq!(
            doc["providers"][0]["models"][0]["note"].as_str(),
            Some("keep")
        );
    }

    #[test]
    fn discovery_fills_missing_values_but_rejects_changed_endpoint_and_deleted_model() {
        let (cfg, mut doc, snapshot, key) = fixture();
        let result = Discovery {
            context: Some(123),
            vision: Some(true),
            ..Default::default()
        };
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((true, Some(123)))
        );
        assert_eq!(
            merge(&mut doc, &cfg, &snapshot, &key, &result),
            Some((false, Some(123)))
        );
        doc["providers"][0]["base_url"] = toml::Value::String("https://other.test".into());
        assert_eq!(merge(&mut doc, &cfg, &snapshot, &key, &result), None);
        let (cfg, mut doc, snapshot, key) = fixture();
        doc["providers"][0]["models"] = toml::Value::Array(vec![]);
        assert_eq!(merge(&mut doc, &cfg, &snapshot, &key, &result), None);
    }
    struct TestConfig(std::path::PathBuf);
    impl TestConfig {
        fn new(doc: &toml::Value) -> Self {
            let path =
                std::env::temp_dir().join(format!("nuphus-metadata-{}.toml", uuid::Uuid::new_v4()));
            std::fs::write(&path, toml::to_string(doc).unwrap()).unwrap();
            Self(path)
        }
    }
    impl Drop for TestConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[tokio::test]
    async fn activation_returns_while_background_discovery_is_blocked() {
        let (mut cfg, mut doc, _, _) = fixture();
        cfg.provider = "local".into(); // Release below must not contact a real provider.
        doc["providers"][0]["name"] = toml::Value::String("local".into());
        let config = TestConfig::new(&doc);
        let state = AppState {
            llm_config_path: config.0.clone(),
            ..Default::default()
        };
        let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        {
            let mut runtime = state.runtime.lock().unwrap();
            runtime.model_context_window = 999_999;
            runtime.metadata_tasks.permits = permits.clone();
        }
        let app = tauri::test::mock_builder()
            .manage(state)
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let state = app.state::<AppState>();
        let generation = activate(&state, &cfg, None).unwrap();
        schedule(app.handle().clone(), cfg.clone(), generation);
        {
            let runtime = state.runtime.lock().unwrap();
            assert_eq!(
                runtime.model_context_window, 0,
                "must not retain previous model window"
            );
            assert_eq!(runtime.metadata_tasks.pending.len(), 1);
            assert!(is_current(&runtime, &cfg, generation));
        }
        // A second activation is independent of the pending network job.
        let next = activate(&state, &cfg, Some(8192)).unwrap();
        schedule(app.handle().clone(), cfg.clone(), next);
        {
            let runtime = state.runtime.lock().unwrap();
            assert_eq!(runtime.model_context_window, 8192);
            assert_eq!(runtime.metadata_tasks.pending.len(), 1);
        }
        permits.add_permits(1);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if state
                    .runtime
                    .lock()
                    .unwrap()
                    .metadata_tasks
                    .pending
                    .is_empty()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state.runtime.lock().unwrap().model_context_window, 8192);
    }

    #[test]
    fn concurrent_config_writers_keep_both_fields() {
        let (_, doc, _, _) = fixture();
        let config = TestConfig::new(&doc);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for n in 1..=20 {
                    toml_ops::update_model_context_window(&config.0, "custom-a", "m", n).unwrap();
                }
            });
            scope.spawn(|| {
                for _ in 0..20 {
                    toml_ops::update_model_supports_vision(
                        &config.0,
                        "custom-a",
                        "m",
                        true,
                        Some("user"),
                    )
                    .unwrap();
                }
            });
        });
        assert_eq!(
            toml_ops::read_model_context_window(&config.0, "custom-a", "m"),
            Some(20)
        );
        assert_eq!(
            toml_ops::read_model_supports_vision(&config.0, "custom-a", "m"),
            Some(true)
        );
    }
}
