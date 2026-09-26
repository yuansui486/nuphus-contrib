//! Edition-specific paths. No HOME/USERPROFILE rewriting, no migration of the
//! original application's files, and no fallback to original model credentials.
use std::path::PathBuf;

pub const WORKBENCH: bool = cfg!(feature = "workbench");

type ProductGuard = Box<dyn Fn() -> std::result::Result<String, String> + Send + Sync>;
static PRODUCT_GUARD: std::sync::OnceLock<ProductGuard> = std::sync::OnceLock::new();
tokio::task_local! { pub static PRODUCT_EPOCH: String; }

pub fn install_product_guard(guard: ProductGuard) -> std::result::Result<(), String> {
    PRODUCT_GUARD
        .set(guard)
        .map_err(|_| "Product guard already installed".into())
}

/// Ordinary upstream builds retain their behavior. Workbench never defaults to allowed.
pub fn require_product() -> std::result::Result<(), String> {
    if !WORKBENCH {
        return Ok(());
    }
    #[cfg(test)]
    if PRODUCT_GUARD.get().is_none() {
        return Ok(());
    }
    let epoch = PRODUCT_GUARD
        .get()
        .ok_or("请先启动灵雀并登录；命令行请使用灵雀 MCP。")?()?;
    if PRODUCT_EPOCH
        .try_with(|expected| expected != &epoch)
        .unwrap_or(false)
    {
        return Err("登录身份已变化，旧任务已停止。".into());
    }
    Ok(())
}

pub fn data_name() -> &'static str {
    if WORKBENCH {
        "nuphus-workbench"
    } else {
        "Nuphus"
    }
}

pub fn config_name() -> &'static str {
    if WORKBENCH {
        "nuphus-workbench"
    } else {
        "nuphus"
    }
}

pub fn home_name() -> &'static str {
    if WORKBENCH {
        ".nuphus-workbench"
    } else {
        ".nuphus"
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(config_name())
}

pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .or_else(|_| dirs::home_dir().ok_or(std::env::VarError::NotPresent))
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(home_name())
}

pub fn workbench_data_dir() -> PathBuf {
    std::env::var_os("NUPHUS_WORKBENCH_DATA_DIR")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("nuphus-workbench")
        })
}
