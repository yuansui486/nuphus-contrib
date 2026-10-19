//! MCP server configuration loader.
//!
//! 两层配置合并（本机层优先，同名覆盖）：
//! 1. `plugin/mcp/servers.yaml` —— 随仓基础配置（入库、可随发布版携带）
//! 2. `<数据目录>/mcp/servers.local.yaml` —— **本机私有配置**（不入库、不随发布版携带），
//!    用于接入仅本机可用的 server；数据目录见 `crate::utils::nuphus_data_dir()`。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Per-server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 启动命令
    pub command: String,
    /// 启动参数
    #[serde(default)]
    pub args: Vec<String>,
    /// 环境变量
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// 调用超时 ms，默认 30000
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    /// Nuphus 启动时自动拉起？
    #[serde(default)]
    pub auto_start: bool,
}

fn default_timeout() -> u64 {
    30000
}

/// Top-level MCP configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, ServerConfig>,
}

/// 基础配置文件路径：`plugin/mcp/servers.yaml`（随仓、可随发布版携带）。
fn base_config_path() -> PathBuf {
    crate::utils::workspace_root()
        .join("plugin")
        .join("mcp")
        .join("servers.yaml")
}

/// 本机私有配置文件路径：`<数据目录>/mcp/servers.local.yaml`。
///
/// 位于用户数据目录（非仓库、非包内），因此**不会入库、不会随发布版携带**；
/// 只在本机生效，与基础配置合并读取。
fn local_config_path() -> PathBuf {
    crate::utils::nuphus_data_dir()
        .join("mcp")
        .join("servers.local.yaml")
}

/// 解析单个配置文件；文件不存在（或为空）返回 `Ok(None)`，不是错误。
fn parse_config_file(path: &Path) -> Result<Option<McpConfig>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(None),
        Ok(yaml) => serde_yaml::from_str::<McpConfig>(&yaml)
            .map(Some)
            .map_err(|e| format!("Failed to parse {}: {}", path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read {}: {}", path.display(), e)),
    }
}

/// 合并两层配置：`local` 覆盖 `base` 的同名 server，其余保持各自来源。
fn merge_configs(mut base: McpConfig, local: McpConfig) -> McpConfig {
    for (key, cfg) in local.servers {
        base.servers.insert(key, cfg);
    }
    base
}

/// Load MCP configuration: base `plugin/mcp/servers.yaml` merged with the
/// machine-local override `<data_dir>/mcp/servers.local.yaml`.
/// Returns empty config if neither file exists.
pub fn load_config() -> Result<McpConfig, String> {
    let base = parse_config_file(&base_config_path())?.unwrap_or_default();
    let local_path = local_config_path();
    let local = parse_config_file(&local_path)?.unwrap_or_default();
    if !local.servers.is_empty() {
        tracing::info!(
            "[mcp] merged {} machine-local server(s) from {}",
            local.servers.len(),
            local_path.display()
        );
    }
    Ok(merge_configs(base, local))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn srv(command: &str) -> ServerConfig {
        ServerConfig {
            command: command.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            timeout_ms: 30000,
            auto_start: false,
        }
    }

    fn config_with(entries: &[(&str, &str)]) -> McpConfig {
        let mut cfg = McpConfig::default();
        for (k, v) in entries {
            cfg.servers.insert((*k).to_string(), srv(v));
        }
        cfg
    }

    #[test]
    fn merge_local_overrides_same_key_and_keeps_others() {
        let base = config_with(&[("a", "base-a"), ("b", "base-b")]);
        let local = config_with(&[("b", "local-b"), ("c", "local-c")]);
        let merged = merge_configs(base, local);
        assert_eq!(merged.servers.len(), 3);
        assert_eq!(merged.servers["a"].command, "base-a");
        assert_eq!(merged.servers["b"].command, "local-b");
        assert_eq!(merged.servers["c"].command, "local-c");
    }

    #[test]
    fn parse_missing_file_is_none() {
        let p = std::env::temp_dir().join("nuphus_mcp_config_missing.local.yaml");
        let _ = std::fs::remove_file(&p);
        assert!(parse_config_file(&p).unwrap().is_none());
    }

    #[test]
    fn parse_empty_or_malformed_file_handling() {
        let p = std::env::temp_dir().join("nuphus_mcp_config_bad.local.yaml");
        std::fs::write(&p, "   \n").unwrap();
        assert!(parse_config_file(&p).unwrap().is_none());
        std::fs::write(&p, "servers: [oops").unwrap();
        assert!(parse_config_file(&p).is_err());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_wellformed_local_style_yaml() {
        let p = std::env::temp_dir().join("nuphus_mcp_config_ok.local.yaml");
        std::fs::write(
            &p,
            "servers:\n  demo:\n    command: \"npx\"\n    args: [\"-y\", \"pkg\"]\n    timeout_ms: 120000\n",
        )
        .unwrap();
        let cfg = parse_config_file(&p).unwrap().unwrap();
        assert_eq!(cfg.servers["demo"].command, "npx");
        assert_eq!(cfg.servers["demo"].args.len(), 2);
        assert_eq!(cfg.servers["demo"].timeout_ms, 120000);
        let _ = std::fs::remove_file(&p);
    }
}
