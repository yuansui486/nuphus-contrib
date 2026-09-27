//! 知识库管理命令
//!
//! 提供 search_knowledge / list_knowledge / delete_knowledge 三个 Tauri 命令，
//! 与 Agent 端 knowledge_search 工具共享同一索引引擎实例。

use nuphus_index::{IndexConfig, IndexEngine, KnowledgeHit, QueryRequest};
use tauri::State;

use crate::state::AppState;

// ── 索引引擎初始化 ──

/// 获取或初始化知识库索引引擎（lazy init）
fn ensure_knowledge_engine(
    state: &AppState,
) -> Result<std::sync::MutexGuard<'_, crate::state::ExecutionState>, String> {
    let mut guard = state
        .execution
        .lock()
        .map_err(|e| format!("锁异常: {}", e))?;
    if guard.knowledge_engine.is_none() {
        // docs_root = plugin_root()/knowledge（utils 单一来源，缺失即创建）；
        // index = nuphus_data_dir()/index —— 不再从 docs_root 反推两级，
        // Tauri 端与 Agent 端共用同一推导（nuphus::utils::knowledge_*）。
        let docs_root = nuphus::utils::knowledge_docs_root();
        let index_dir = nuphus::utils::knowledge_index_dir();
        let index_path = index_dir.join("knowledge_index.json");

        if let Err(e) = std::fs::create_dir_all(&index_dir) {
            tracing::warn!("[knowledge] 无法创建索引目录 {:?}: {}", index_dir, e);
        }

        tracing::info!(
            "[knowledge] Initializing IndexEngine: docs_root={:?}, index_path={:?}",
            docs_root,
            index_path
        );

        guard.knowledge_engine = Some(IndexEngine::new(IndexConfig {
            docs_root: docs_root.to_string_lossy().to_string(),
            index_path: index_path.to_string_lossy().to_string(),
        }));
    }
    Ok(guard)
}

// ── Tauri 命令 ──

#[tauri::command]
pub fn search_knowledge(
    state: State<'_, AppState>,
    query: String,
    tags: Option<Vec<String>>,
    max_results: Option<usize>,
) -> Result<Vec<KnowledgeHit>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 搜索前增量扫描，感知新增/修改文件
    engine.rescan_modified();
    let max = max_results.unwrap_or(10);
    let req = QueryRequest {
        query,
        tags: tags.unwrap_or_default(),
        max_results: max,
    };
    Ok(engine.search(&req))
}

#[tauri::command]
pub fn list_knowledge(state: State<'_, AppState>) -> Result<Vec<KnowledgeHit>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 每次列表查询前增量扫描，确保新增文件可见
    engine.rescan_modified();
    let req = QueryRequest {
        query: String::new(),
        tags: Vec::new(),
        max_results: 9999,
    };
    Ok(engine.search(&req))
}

#[tauri::command]
pub fn list_knowledge_tags(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    // 每次标签列表查询前增量扫描，确保新增文件的标签可见
    engine.rescan_modified();
    Ok(engine.all_tags())
}

#[tauri::command]
pub fn delete_knowledge(state: State<'_, AppState>, rel_path: String) -> Result<bool, String> {
    let engine_guard = ensure_knowledge_engine(state.inner())?;
    let engine = engine_guard
        .knowledge_engine
        .as_ref()
        .ok_or_else(|| "知识引擎未初始化".to_string())?;
    let docs_root = std::path::PathBuf::from(&engine.config().docs_root);
    let abs_path = docs_root.join(&rel_path);
    let canonical = abs_path
        .canonicalize()
        .map_err(|e| format!("无法解析路径: {}", e))?;
    let root_canonical = docs_root
        .canonicalize()
        .map_err(|e| format!("无法解析根目录: {}", e))?;
    if !canonical.starts_with(&root_canonical) {
        return Err("路径越权拒绝".to_string());
    }
    std::fs::remove_file(&abs_path).map_err(|e| format!("删除失败: {}", e))?;
    engine.rescan();
    Ok(true)
}
