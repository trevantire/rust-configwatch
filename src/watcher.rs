//! File system watcher implementation for monitoring configuration file changes.

use crate::{ConfigError, ConfigSource, ConfigValue};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

/// Watches a file or directory for changes using polling.
///
/// This module provides a cross-platform file watcher that polls for changes
/// at regular intervals. For production use with inotify/FSEvents, consider
/// using the `notify` crate directly.
pub struct FileWatcher {
    path: PathBuf,
    last_modified: Mutex<Option<SystemTime>>,
    last_content_hash: Mutex<u64>,
    changed: AtomicBool,
}

impl FileWatcher {
    /// Create a new file watcher for the given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            last_modified: Mutex::new(None),
            last_content_hash: Mutex::new(0),
            changed: AtomicBool::new(false),
        }
    }

    /// Check if the file has been modified since the last check.
    pub fn check_for_changes(&self) -> bool {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        let modified = match metadata.modified() {
            Ok(m) => m,
            Err(_) => return false,
        };

        let mut last_mod = self.last_modified.lock();
        if let Some(last) = *last_mod {
            if modified > last {
                *last_mod = Some(modified);
                self.changed.store(true, Ordering::Relaxed);
                return true;
            }
        } else {
            *last_mod = Some(modified);
        }

        false
    }

    /// Read the file content and compute a hash for change detection.
    pub fn read_content_hash(&self) -> Result<u64, ConfigError> {
        let content = std::fs::read(&self.path)?;
        let hash = self.compute_hash(&content);

        let mut last_hash = self.last_content_hash.lock();
        if hash != *last_hash {
            *last_hash = hash;
            self.changed.store(true, Ordering::Relaxed);
        }

        Ok(hash)
    }

    fn compute_hash(&self, data: &[u8]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        data.hash(&mut hasher);
        hasher.finish()
    }

    /// Get the path being watched.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// A configuration source that reads from a file.
///
/// Supports JSON and TOML formats, detected by file extension.
pub struct FileWatcherSource {
    name: String,
    path: PathBuf,
    watcher: FileWatcher,
}

impl FileWatcherSource {
    /// Create a new file watcher source.
    pub fn new(name: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        Self {
            name: name.into(),
            watcher: FileWatcher::new(&path),
            path,
        }
    }

    fn parse_file(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        let content = std::fs::read_to_string(&self.path)?;

        match self.path.extension().and_then(|e| e.to_str()) {
            Some("json") => Self::parse_json(&content),
            Some("toml") => Self::parse_toml(&content),
            Some(ext) => Err(ConfigError::Parse(format!(
                "Unsupported file format: {}",
                ext
            ))),
            None => Err(ConfigError::Parse(
                "Could not determine file format".to_string(),
            )),
        }
    }

    fn parse_json(content: &str) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        let value: serde_json::Value =
            serde_json::from_str(content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(Self::json_to_config(value))
    }

    fn parse_toml(content: &str) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        let value: toml::Value =
            toml::from_str(content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        Ok(Self::toml_to_config(value))
    }

    fn json_to_config(value: serde_json::Value) -> HashMap<String, ConfigValue> {
        let mut result = HashMap::new();
        if let serde_json::Value::Object(map) = value {
            for (key, val) in map {
                result.insert(key, Self::json_value_to_config(val));
            }
        }
        result
    }

    fn json_value_to_config(value: serde_json::Value) -> ConfigValue {
        match value {
            serde_json::Value::String(s) => ConfigValue::String(s),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    ConfigValue::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    ConfigValue::Float(f)
                } else {
                    ConfigValue::Null
                }
            }
            serde_json::Value::Bool(b) => ConfigValue::Boolean(b),
            serde_json::Value::Array(arr) => {
                ConfigValue::Array(arr.into_iter().map(Self::json_value_to_config).collect())
            }
            serde_json::Value::Object(map) => {
                let config_map = map
                    .into_iter()
                    .map(|(k, v)| (k, Self::json_value_to_config(v)))
                    .collect();
                ConfigValue::Map(config_map)
            }
            serde_json::Value::Null => ConfigValue::Null,
        }
    }

    fn toml_to_config(value: toml::Value) -> HashMap<String, ConfigValue> {
        let mut result = HashMap::new();
        if let toml::Value::Table(table) = value {
            for (key, val) in table {
                result.insert(key, Self::toml_value_to_config(val));
            }
        }
        result
    }

    fn toml_value_to_config(value: toml::Value) -> ConfigValue {
        match value {
            toml::Value::String(s) => ConfigValue::String(s),
            toml::Value::Integer(i) => ConfigValue::Integer(i),
            toml::Value::Float(f) => ConfigValue::Float(f),
            toml::Value::Boolean(b) => ConfigValue::Boolean(b),
            toml::Value::Datetime(dt) => ConfigValue::String(dt.to_string()),
            toml::Value::Array(arr) => {
                ConfigValue::Array(arr.into_iter().map(Self::toml_value_to_config).collect())
            }
            toml::Value::Table(table) => {
                let config_map = table
                    .into_iter()
                    .map(|(k, v)| (k, Self::toml_value_to_config(v)))
                    .collect();
                ConfigValue::Map(config_map)
            }
        }
    }
}

impl ConfigSource for FileWatcherSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        self.parse_file()
    }

    fn has_changed(&self) -> bool {
        self.watcher.check_for_changes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_json_parsing() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, r#"{{"name": "test", "port": 8080, "debug": true}}"#).unwrap();

        let source = FileWatcherSource::new("test", file.path());
        let values = source.read().unwrap();

        assert_eq!(values.get("name").and_then(|v| v.as_str()), Some("test"));
        assert_eq!(values.get("port").and_then(|v| v.as_i64()), Some(8080));
        assert_eq!(values.get("debug").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_toml_parsing() {
        let mut file = NamedTempFile::with_suffix(".toml").unwrap();
        writeln!(file, "name = \"test\"\nport = 8080\ndebug = true").unwrap();

        let source = FileWatcherSource::new("test", file.path());
        let values = source.read().unwrap();

        assert_eq!(values.get("name").and_then(|v| v.as_str()), Some("test"));
        assert_eq!(values.get("port").and_then(|v| v.as_i64()), Some(8080));
        assert_eq!(values.get("debug").and_then(|v| v.as_bool()), Some(true));
    }
}
