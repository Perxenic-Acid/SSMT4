use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const SETTINGS_FILE_NAME: &str = "plugin-settings.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PluginSettingsState {
    #[serde(default)]
    namespaces: BTreeMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug)]
pub enum PluginSettingsError {
    Io(io::Error),
    InvalidState(String),
    InvalidKey(String),
}

impl std::fmt::Display for PluginSettingsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "plugin settings I/O error: {error}"),
            Self::InvalidState(error) => {
                write!(formatter, "invalid plugin settings state: {error}")
            }
            Self::InvalidKey(key) => write!(formatter, "invalid plugin settings key: {key}"),
        }
    }
}

impl std::error::Error for PluginSettingsError {}

impl From<io::Error> for PluginSettingsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub struct PluginSettingsStore {
    path: PathBuf,
    state: PluginSettingsState,
}

impl PluginSettingsStore {
    pub fn from_default_location() -> Result<Self, PluginSettingsError> {
        let root = crate::config::path_manager::PathManager::ssmt_global_config_folder();
        Self::new(root.join(SETTINGS_FILE_NAME))
    }

    pub fn new(path: impl Into<PathBuf>) -> Result<Self, PluginSettingsError> {
        let path = path.into();
        let state = read_state(&path)?;
        Ok(Self { path, state })
    }

    pub fn get(&self, plugin_id: &str, key: &str) -> Result<Option<Value>, PluginSettingsError> {
        validate_key(key)?;
        Ok(self
            .state
            .namespaces
            .get(plugin_id)
            .and_then(|namespace| namespace.get(key).cloned()))
    }

    pub fn set(
        &mut self,
        plugin_id: &str,
        key: &str,
        value: Value,
    ) -> Result<(), PluginSettingsError> {
        validate_key(key)?;
        self.state
            .namespaces
            .entry(plugin_id.to_string())
            .or_default()
            .insert(key.to_string(), value);
        write_state(&self.path, &self.state)
    }
}

fn validate_key(key: &str) -> Result<(), PluginSettingsError> {
    if key.is_empty()
        || key.len() > 128
        || key
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(PluginSettingsError::InvalidKey(key.to_string()));
    }
    Ok(())
}

fn read_state(path: &Path) -> Result<PluginSettingsState, PluginSettingsError> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|error| PluginSettingsError::InvalidState(error.to_string())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PluginSettingsState::default()),
        Err(error) => Err(error.into()),
    }
}

fn write_state(path: &Path, state: &PluginSettingsState) -> Result<(), PluginSettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| PluginSettingsError::InvalidState(error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "ssmt-plugin-settings-{}.json",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn persists_namespaced_values_atomically() {
        let path = path();
        let mut store = PluginSettingsStore::new(&path).unwrap();
        store
            .set("ssmt.fixture", "timeoutMs", Value::from(15000))
            .unwrap();
        drop(store);
        let restored = PluginSettingsStore::new(&path).unwrap();
        assert_eq!(
            restored.get("ssmt.fixture", "timeoutMs").unwrap(),
            Some(Value::from(15000))
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_path_like_keys() {
        let mut store = PluginSettingsStore::new(path()).unwrap();
        assert!(matches!(
            store.set("ssmt.fixture", "../secret", Value::Null),
            Err(PluginSettingsError::InvalidKey(_))
        ));
    }
}
