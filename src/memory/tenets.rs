//! TenetStore — user-defined immutable principle storage layer
//!
//! Separated from the evolution funnel: Tenets are user-confirmed principles with immutable=true,
//! not processed by Cleaner/Extractor/Solidifier, and not subject to forgetting mechanisms.
//!
//! Capacity limit: MAX_TENETS = 20, rejects addition when full and prompts user to clean up.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single tenet entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenet {
    pub id: String,
    /// Principle content text (user natural language description)
    pub content: String,
    /// Owning project directory; None/empty = general tenet applying to every project.
    /// Legacy entries without this field load as general (serde default).
    #[serde(default)]
    pub project_path: Option<String>,
    /// Source
    #[serde(default)]
    pub source: TenetSource,
    /// Priority (affects sorting and prompt strength)
    #[serde(default)]
    pub priority: TenetPriority,
    /// Enforcement level
    #[serde(default)]
    pub enforce_level: EnforceLevel,
    /// true = cannot be deleted by evolution funnel / auto cleanup
    #[serde(default = "default_true")]
    pub immutable: bool,
    /// Creation time ISO 8601
    pub created_at: String,
    /// Update time ISO 8601
    pub updated_at: String,
    /// false = soft delete (retain record but not active)
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_true() -> bool {
    true
}

/// Principle source
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TenetSource {
    /// Manually set by user in UI or conversation
    #[default]
    UserManual,
    /// Automatically extracted from successful patterns (becomes immutable after user confirmation)
    AutoExtracted,
    /// Built-in system rules (e.g. safety boundaries)
    System,
}

/// Priority — affects sorting and visual prompt strength
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TenetPriority {
    /// Core principle, always pinned to top
    Critical,
    /// Important principle
    #[default]
    High,
    /// General advice
    Medium,
    /// Weak reminder
    Low,
}

impl TenetPriority {
    pub fn label(self) -> &'static str {
        match self {
            TenetPriority::Critical => "Core",
            TenetPriority::High => "Important",
            TenetPriority::Medium => "General",
            TenetPriority::Low => "Reminder",
        }
    }
}

/// Enforcement level — determines how violations are handled
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EnforceLevel {
    /// Prompt only in UI/logs, does not block execution
    Suggestion,
    /// Inject reminder to LLM but allow continuation
    #[default]
    Warning,
    /// Directly block tool call, return failure
    Block,
}

impl EnforceLevel {
    pub fn label(self) -> &'static str {
        match self {
            EnforceLevel::Suggestion => "Suggestion",
            EnforceLevel::Warning => "Warning",
            EnforceLevel::Block => "Block",
        }
    }
}

/// Tenet alert — produced by tenet_check, consumed by ProtectionGuard
#[derive(Debug, Clone)]
pub struct TenetAlert {
    pub tenet_id: String,
    pub content: String,
    pub enforce_level: EnforceLevel,
    pub reason: String,
}

/// Maximum tenets storage limit
const MAX_TENETS: usize = 20;

/// Per-scope cap for prompt injection (general / current project independently).
const MAX_PROMPT_TENETS_PER_SCOPE: usize = 10;

/// Tenet storage
pub struct TenetStore {
    tenets: Vec<Tenet>,
    path: PathBuf,
}

impl TenetStore {
    /// Initialize TenetStore, automatically loads from tenets.json
    pub fn new() -> Self {
        let path = crate::memory::get_memory_config()
            .base_path()
            .join("tenets.json");
        let tenets = Self::load(&path).unwrap_or_default();
        tracing::info!(
            "[TenetStore] loaded {} tenets from {}",
            tenets.len(),
            path.display()
        );
        Self { tenets, path }
    }

    /// Use custom path (mainly for testing)
    pub fn with_path(path: PathBuf) -> Self {
        let tenets = Self::load(&path).unwrap_or_default();
        Self { tenets, path }
    }

    // ── CRUD ──

    /// Add a tenet. Returns Err(TenetStoreError::CapacityExceeded) when over limit.
    pub fn add(&mut self, mut tenet: Tenet) -> Result<(), TenetStoreError> {
        if self.active_count() >= MAX_TENETS {
            return Err(TenetStoreError::CapacityExceeded {
                max: MAX_TENETS,
                current: self.active_count(),
            });
        }
        if tenet.id.is_empty() {
            tenet.id = uuid::Uuid::new_v4().to_string();
        }
        let now = chrono::Utc::now().to_rfc3339();
        if tenet.created_at.is_empty() {
            tenet.created_at = now.clone();
        }
        tenet.updated_at = now;
        self.tenets.push(tenet);
        self.sort_by_priority();
        self.save()?;
        tracing::info!(
            "[TenetStore] added tenet, total active={}",
            self.active_count()
        );
        Ok(())
    }

    /// Soft delete (sets active=false)
    pub fn deactivate(&mut self, id: &str) -> Result<bool, TenetStoreError> {
        if let Some(t) = self.tenets.iter_mut().find(|t| t.id == id) {
            t.active = false;
            t.updated_at = chrono::Utc::now().to_rfc3339();
            self.save()?;
            tracing::info!("[TenetStore] deactivated tenet {}", id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Physical delete (removes deactivated entries, frees capacity)
    pub fn purge_inactive(&mut self) -> Result<usize, TenetStoreError> {
        let before = self.tenets.len();
        self.tenets.retain(|t| t.active);
        let removed = before - self.tenets.len();
        if removed > 0 {
            self.save()?;
            tracing::info!("[TenetStore] purged {} inactive tenets", removed);
        }
        Ok(removed)
    }

    /// Update tenet content
    pub fn update(
        &mut self,
        id: &str,
        content: &str,
        priority: TenetPriority,
        enforce: EnforceLevel,
    ) -> Result<bool, TenetStoreError> {
        if let Some(t) = self.tenets.iter_mut().find(|t| t.id == id && t.active) {
            t.content = content.to_string();
            t.priority = priority;
            t.enforce_level = enforce;
            t.updated_at = chrono::Utc::now().to_rfc3339();
            self.sort_by_priority();
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ── Query ──

    /// All active tenets (sorted by priority descending)
    pub fn active(&self) -> Vec<&Tenet> {
        self.tenets.iter().filter(|t| t.active).collect()
    }

    /// All tenets (including inactive)
    pub fn all(&self) -> &[Tenet] {
        &self.tenets
    }

    /// Number of active tenets
    pub fn active_count(&self) -> usize {
        self.tenets.iter().filter(|t| t.active).count()
    }

    /// Capacity status
    pub fn capacity_status(&self) -> CapacityStatus {
        let active = self.active_count();
        if active >= MAX_TENETS {
            CapacityStatus::Full { max: MAX_TENETS }
        } else if active >= MAX_TENETS * 3 / 4 {
            CapacityStatus::NearLimit {
                active,
                max: MAX_TENETS,
                remaining: MAX_TENETS - active,
            }
        } else {
            CapacityStatus::Ok {
                active,
                max: MAX_TENETS,
                remaining: MAX_TENETS - active,
            }
        }
    }

    /// Find by ID
    pub fn get(&self, id: &str) -> Option<&Tenet> {
        self.tenets.iter().find(|t| t.id == id)
    }

    // ── Pre-tool-call check ──

    /// Check if a tool call violates any active tenet
    ///
    /// Simple keyword matching: checks if tenet.content is contained in tool_name/params.
    /// Can be upgraded to LLM semantic judgment in the future, but keyword matching
    /// is sufficient for principles like "don't modify the registry".
    pub fn check(&self, tool_name: &str, params: &serde_json::Value) -> Vec<TenetAlert> {
        let param_str = serde_json::to_string(params)
            .unwrap_or_default()
            .to_lowercase();
        let tool_lower = tool_name.to_lowercase();
        let mut alerts = Vec::new();

        for tenet in self.tenets.iter().filter(|t| t.active) {
            let content_lower = tenet.content.to_lowercase();
            let mut matched = false;
            let mut reason = String::new();

            // Extract potential keywords from tenet content
            // Split CJK text character by character, split English by whitespace
            let mut keywords: Vec<String> = Vec::new();
            for word in content_lower.split_whitespace() {
                if word.chars().any(|c| ('\u{4e00}'..'\u{9fff}').contains(&c)) {
                    // Contains CJK: extract character by character (UTF-8 single char 3 bytes, satisfies >= 2)
                    for ch in word.chars() {
                        let s = ch.to_string();
                        if !is_stop_word(&s) {
                            keywords.push(s);
                        }
                    }
                } else if word.len() >= 2 && !is_stop_word(word) {
                    keywords.push(word.to_string());
                }
            }

            // 1. Tool name match (tenet content directly mentions tool name)
            if content_lower.contains(&tool_lower) {
                matched = true;
                reason = format!(
                    "Tenet '{}' directly involves tool '{}'",
                    tenet.content, tool_name
                );
            }
            // 1.5 Keyword matches tool name (tenet keyword appears in tool name)
            if !matched {
                for kw in &keywords {
                    if tool_lower.contains(kw) {
                        matched = true;
                        reason = format!(
                            "Tenet '{}' keyword '{}' matches tool name '{}'",
                            tenet.content, kw, tool_name
                        );
                        break;
                    }
                }
            }
            // 2. Parameter keyword match (tenet keyword appears in parameters)
            if !matched {
                for kw in &keywords {
                    if param_str.contains(kw) {
                        matched = true;
                        reason = format!(
                            "Tenet '{}' keyword '{}' matches parameter content",
                            tenet.content, kw
                        );
                        break;
                    }
                }
            }

            if matched {
                alerts.push(TenetAlert {
                    tenet_id: tenet.id.clone(),
                    content: tenet.content.clone(),
                    enforce_level: tenet.enforce_level,
                    reason,
                });
            }
        }

        // Sort by enforce_level severity (Block > Warning > Suggestion)
        alerts.sort_by(|a, b| {
            let ord_a = match a.enforce_level {
                EnforceLevel::Block => 0,
                EnforceLevel::Warning => 1,
                EnforceLevel::Suggestion => 2,
            };
            let ord_b = match b.enforce_level {
                EnforceLevel::Block => 0,
                EnforceLevel::Warning => 1,
                EnforceLevel::Suggestion => 2,
            };
            ord_a.cmp(&ord_b)
        });

        alerts
    }

    /// Generate tenet list text for prompt injection.
    ///
    /// Only general tenets plus tenets owned by `current_project` are emitted.
    /// Project tenets never leak into other projects' prompts.
    pub fn format_for_prompt(&self, current_project: Option<&str>) -> Option<String> {
        let mut general: Vec<&Tenet> = Vec::new();
        let mut project: Vec<&Tenet> = Vec::new();
        for t in self.tenets.iter().filter(|t| t.active) {
            match t.project_path.as_deref().map(str::trim) {
                None | Some("") => general.push(t),
                Some(path) if current_project.is_some_and(|cur| same_project_path(path, cur)) => {
                    project.push(t)
                }
                Some(_) => {}
            }
        }
        if general.is_empty() && project.is_empty() {
            return None;
        }

        let mut lines = vec!["### User-set immutable principles".to_string()];
        if !general.is_empty() {
            lines.push("#### 通用原则（所有项目适用）".to_string());
            push_tenet_lines(&mut lines, &general);
        }
        if !project.is_empty() {
            lines.push("#### 当前项目原则（仅当前项目适用）".to_string());
            push_tenet_lines(&mut lines, &project);
        }
        Some(lines.join("\n"))
    }

    // ── Internal ──

    fn sort_by_priority(&mut self) {
        self.tenets.sort_by(|a, b| {
            // Active first
            let active_ord = b.active.cmp(&a.active);
            if active_ord != std::cmp::Ordering::Equal {
                return active_ord;
            }
            // Higher priority first
            b.priority.cmp(&a.priority)
        });
    }

    fn save(&self) -> Result<(), TenetStoreError> {
        let json = serde_json::to_string_pretty(&self.tenets)
            .map_err(|e| TenetStoreError::Serialize(e.to_string()))?;
        std::fs::write(&self.path, json).map_err(|e| TenetStoreError::Io(e.to_string()))?;
        Ok(())
    }

    fn load(path: &PathBuf) -> Result<Vec<Tenet>, TenetStoreError> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let json = std::fs::read_to_string(path).map_err(|e| TenetStoreError::Io(e.to_string()))?;
        let tenets: Vec<Tenet> =
            serde_json::from_str(&json).map_err(|e| TenetStoreError::Serialize(e.to_string()))?;
        Ok(tenets)
    }
}

fn push_tenet_lines(lines: &mut Vec<String>, tenets: &[&Tenet]) {
    for t in tenets.iter().take(MAX_PROMPT_TENETS_PER_SCOPE) {
        lines.push(format!(
            "- [{}][{}] {}",
            t.priority.label(),
            t.enforce_level.label(),
            t.content
        ));
    }
    if tenets.len() > MAX_PROMPT_TENETS_PER_SCOPE {
        lines.push(format!(
            "- ... {} more tenets not shown",
            tenets.len() - MAX_PROMPT_TENETS_PER_SCOPE
        ));
    }
}

/// Project path equality: trim, ignore trailing separators, Windows case-insensitive.
fn same_project_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_string();
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if cfg!(windows) {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Capacity status
#[derive(Debug, Clone)]
pub enum CapacityStatus {
    Ok {
        active: usize,
        max: usize,
        remaining: usize,
    },
    NearLimit {
        active: usize,
        max: usize,
        remaining: usize,
    },
    Full {
        max: usize,
    },
}

impl CapacityStatus {
    pub fn is_full(&self) -> bool {
        matches!(self, CapacityStatus::Full { .. })
    }
    pub fn is_near_limit(&self) -> bool {
        matches!(self, CapacityStatus::NearLimit { .. })
    }
}

/// Storage error
#[derive(Debug, Clone)]
pub enum TenetStoreError {
    CapacityExceeded { max: usize, current: usize },
    Serialize(String),
    Io(String),
}

impl std::fmt::Display for TenetStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TenetStoreError::CapacityExceeded { max, current } => {
                write!(
                    f,
                    "Tenet storage full ({}/{}). Delete old tenets before adding new ones.",
                    current, max
                )
            }
            TenetStoreError::Serialize(s) => write!(f, "Serialization error: {}", s),
            TenetStoreError::Io(s) => write!(f, "IO error: {}", s),
        }
    }
}

impl std::error::Error for TenetStoreError {}

// Simple stop word filtering to avoid matching generic words like "不要" "操作"
fn is_stop_word(word: &str) -> bool {
    const STOP_WORDS: &[&str] = &[
        "不要", "禁止", "必须", "应该", "可以", "需要", "避免", "操作", "修改", "删除", "创建",
        "使用", "调用", "执行", "the", "this", "that", "with", "from", "into", "than",
    ];
    STOP_WORDS.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capacity_limit() {
        let mut store = TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets.json"));
        // Clear existing
        store.tenets.clear();
        for i in 0..MAX_TENETS + 1 {
            let t = Tenet {
                id: format!("t{}", i),
                content: format!("test {}", i),
                project_path: None,
                source: TenetSource::UserManual,
                priority: TenetPriority::High,
                enforce_level: EnforceLevel::Warning,
                immutable: true,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                active: true,
            };
            let result = store.add(t);
            if i < MAX_TENETS {
                assert!(result.is_ok(), "Item {} should succeed", i + 1);
            } else {
                assert!(result.is_err(), "Item {} should fail", i + 1);
            }
        }
    }

    #[test]
    fn test_check_tool_match() {
        let mut store =
            TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets_check.json"));
        store.tenets.clear();
        store
            .add(Tenet {
                id: "t1".to_string(),
                content: "不要操作注册表".to_string(),
                project_path: None,
                source: TenetSource::UserManual,
                priority: TenetPriority::Critical,
                enforce_level: EnforceLevel::Block,
                immutable: true,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                active: true,
            })
            .unwrap();

        // Tool name fuzzy match (CJK keywords matching CJK tool name)
        let alerts = store.check("注册表读取", &serde_json::json!({}));
        assert!(!alerts.is_empty(), "should match registry tenet");
        assert_eq!(alerts[0].enforce_level, EnforceLevel::Block);
    }

    #[test]
    fn test_format_for_prompt() {
        let mut store =
            TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets_prompt.json"));
        store.tenets.clear();
        store
            .add(Tenet {
                id: "t1".to_string(),
                content: "改动文件前先备份".to_string(),
                project_path: None,
                source: TenetSource::UserManual,
                priority: TenetPriority::High,
                enforce_level: EnforceLevel::Warning,
                immutable: true,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                active: true,
            })
            .unwrap();

        let prompt = store.format_for_prompt(None);
        assert!(prompt.is_some());
        let text = prompt.unwrap();
        assert!(text.contains("User-set immutable principles"));
        assert!(text.contains("[Important][Warning]"));
        assert!(text.contains("改动文件前先备份"));
    }

    fn scoped_tenet(id: &str, content: &str, project: Option<&str>) -> Tenet {
        Tenet {
            id: id.to_string(),
            content: content.to_string(),
            project_path: project.map(str::to_string),
            source: TenetSource::UserManual,
            priority: TenetPriority::High,
            enforce_level: EnforceLevel::Warning,
            immutable: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            active: true,
        }
    }

    /// 只注入「通用 + 当前项目」：其他项目的原则绝不能泄漏。
    #[test]
    fn format_for_prompt_scopes_to_general_and_current_project() {
        let mut store =
            TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets_scope.json"));
        store.tenets.clear();
        for (id, content, project) in [
            ("g", "通用规则", None),
            ("a", "项目 A 规则", Some(r"E:\proj\A")),
            ("b", "项目 B 规则", Some(r"E:\proj\B")),
        ] {
            store.add(scoped_tenet(id, content, project)).unwrap();
        }

        let text = store.format_for_prompt(Some(r"E:\proj\A")).unwrap();
        assert!(text.contains("通用规则"));
        assert!(text.contains("项目 A 规则"));
        assert!(!text.contains("项目 B 规则"), "不得注入其它项目的原则");
        assert!(text.contains("通用原则"));
        assert!(text.contains("当前项目原则"));

        let general_only = store.format_for_prompt(None).unwrap();
        assert!(general_only.contains("通用规则"));
        assert!(!general_only.contains("项目 A 规则"));
        assert!(!general_only.contains("项目 B 规则"));
    }

    /// Windows 下大小写/尾分隔符不同仍视为同一项目。
    #[test]
    fn format_for_prompt_matches_project_path_case_insensitively_on_windows() {
        let mut store =
            TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets_case.json"));
        store.tenets.clear();
        store
            .add(scoped_tenet("a", "项目规则", Some(r"E:\Proj\A")))
            .unwrap();

        let text = store
            .format_for_prompt(Some(r"e:\proj\a\"))
            .unwrap_or_default();
        if cfg!(windows) {
            assert!(text.contains("项目规则"));
        } else {
            assert!(!text.contains("项目规则"));
        }
    }

    #[test]
    fn update_changes_content_and_priority_but_preserves_scope() {
        let mut store =
            TenetStore::with_path(std::env::temp_dir().join("nuphus_test_tenets_update.json"));
        store.tenets.clear();
        store
            .add(scoped_tenet("a", "旧内容", Some(r"E:\proj\A")))
            .unwrap();

        assert!(store
            .update("a", "新内容", TenetPriority::Low, EnforceLevel::Block)
            .unwrap());
        let updated = store.get("a").unwrap();
        assert_eq!(updated.content, "新内容");
        assert_eq!(updated.priority, TenetPriority::Low);
        assert_eq!(updated.enforce_level, EnforceLevel::Block);
        assert_eq!(updated.project_path.as_deref(), Some(r"E:\proj\A"));

        assert!(!store
            .update("missing", "x", TenetPriority::High, EnforceLevel::Warning)
            .unwrap());
    }

    /// 旧版 tenets.json 没有 project_path 字段，必须按通用原则加载。
    #[test]
    fn legacy_tenet_without_project_path_loads_as_general() {
        let json = r#"[{
            "id": "legacy",
            "content": "旧原则",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }]"#;
        let tenets: Vec<Tenet> = serde_json::from_str(json).expect("旧数据必须可反序列化");
        assert_eq!(tenets.len(), 1);
        assert!(tenets[0].project_path.is_none());
        assert!(tenets[0].active);
    }
}
