//! Edition-specific paths. No HOME/USERPROFILE rewriting, no migration of the
//! original application's files, and no fallback to original model credentials.
use std::path::PathBuf;

pub const WORKBENCH: bool = cfg!(feature = "workbench");

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
