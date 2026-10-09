use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(untagged)]
pub enum Error {
    #[error("{0}")]
    State(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(String),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Serialize, Deserialize)]
pub struct Preferences {
    pub automatic: bool,
    pub last_check: i64,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            last_check: 0,
        }
    }
}

pub struct PreferencesStore(PathBuf);
impl PreferencesStore {
    pub fn new(root: &Path) -> Self {
        Self(root.join("update-preferences.json"))
    }
    pub fn update_preferences(&self) -> Result<Preferences> {
        match std::fs::read(&self.0) {
            Ok(bytes) if bytes.len() <= 4096 => {
                serde_json::from_slice(&bytes).map_err(super::error)
            }
            Ok(_) => Err(Error::Invalid(
                "更新偏好文件异常，请检查本机数据目录。".into(),
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
            Err(e) => Err(super::error(e)),
        }
    }
    pub fn save_update_preferences(&self, preferences: &Preferences) -> Result<()> {
        let parent = self
            .0
            .parent()
            .ok_or_else(|| Error::State("更新数据目录无效".into()))?;
        std::fs::create_dir_all(parent).map_err(super::error)?;
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(super::error)?;
        serde_json::to_writer(file.as_file_mut(), preferences).map_err(super::error)?;
        file.as_file().sync_all().map_err(super::error)?;
        file.persist(&self.0).map_err(super::error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_survive_restart_without_changing_other_data() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("existing-project.sqlite");
        std::fs::write(&marker, b"untouched").unwrap();
        let store = PreferencesStore::new(root.path());
        assert!(store.update_preferences().unwrap().automatic);
        store
            .save_update_preferences(&Preferences {
                automatic: false,
                last_check: 123,
            })
            .unwrap();
        let restored = PreferencesStore::new(root.path())
            .update_preferences()
            .unwrap();
        assert!(!restored.automatic);
        assert_eq!(restored.last_check, 123);
        assert_eq!(std::fs::read(marker).unwrap(), b"untouched");
    }
}
