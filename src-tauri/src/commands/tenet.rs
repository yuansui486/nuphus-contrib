//! tenet — 原则相关 Tauri 命令

use nuphus::memory::{EnforceLevel, Tenet, TenetPriority, TenetSource, TenetStore};
use serde_json::json;

fn parse_priority(raw: Option<&str>) -> Option<TenetPriority> {
    raw.and_then(|s| match s {
        "critical" => Some(TenetPriority::Critical),
        "high" => Some(TenetPriority::High),
        "medium" => Some(TenetPriority::Medium),
        "low" => Some(TenetPriority::Low),
        _ => None,
    })
}

/// 获取所有原则
#[tauri::command(async)]
pub fn get_tenets() -> Result<serde_json::Value, String> {
    let store = TenetStore::new();
    let items: Vec<serde_json::Value> = store
        .active()
        .iter()
        .map(|t| {
            json!({
                "id": t.id,
                "content": t.content,
                "project_path": t.project_path,
                "priority": t.priority,
                "enforce": t.enforce_level,
                "active": t.active,
                "created_at": t.created_at,
            })
        })
        .collect();

    Ok(json!({
        "count": store.active_count(),
        "total": store.all().len(),
        "items": items,
    }))
}

/// 添加一条原则
#[tauri::command(async)]
pub fn add_tenet(
    content: String,
    priority: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("原则内容不能为空".into());
    }
    // None/空白 = 通用原则；项目路径按路径比较口径去尾分隔符。
    let project_path = project_path
        .map(|p| p.trim().trim_end_matches(['\\', '/']).to_string())
        .filter(|p| !p.is_empty());
    let priority = parse_priority(priority.as_deref()).unwrap_or(TenetPriority::High);

    let now = chrono::Utc::now().to_rfc3339();
    let tenet = Tenet {
        id: uuid::Uuid::new_v4().to_string(),
        content: content.trim().to_string(),
        project_path,
        source: TenetSource::UserManual,
        priority,
        enforce_level: EnforceLevel::Warning,
        immutable: true,
        created_at: now.clone(),
        updated_at: now,
        active: true,
    };

    TenetStore::new()
        .add(tenet)
        .map_err(|e| format!("添加原则失败: {}", e))
}

/// 更新一条原则的内容与优先级，保留原有作用域与执行级别。
#[tauri::command(async)]
pub fn update_tenet(id: String, content: String, priority: Option<String>) -> Result<bool, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("原则内容不能为空".into());
    }
    let mut store = TenetStore::new();
    let (current_priority, current_enforce) = match store.get(&id) {
        Some(tenet) => (tenet.priority, tenet.enforce_level),
        None => return Ok(false),
    };
    let priority = parse_priority(priority.as_deref()).unwrap_or(current_priority);
    store
        .update(&id, content, priority, current_enforce)
        .map_err(|e| format!("更新原则失败: {}", e))
}

/// 删除（软删除）一条原则
#[tauri::command(async)]
pub fn delete_tenet(id: String) -> Result<bool, String> {
    let mut store = TenetStore::new();
    store
        .deactivate(&id)
        .map_err(|e| format!("删除原则失败: {}", e))
}
