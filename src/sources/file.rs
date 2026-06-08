//! File-based configuration source.
//!
//! Watches configuration files for changes and parses them into ConfigValue maps.
//! Supports JSON and TOML formats.

use crate::{ConfigError, ConfigSource, ConfigValue};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

/// A configuration source that reads from a file.
///
/// # Supported Formats
///
/// - `.json` — JSON files
/// - `.toml` — TOML files
///
/// # Example
///
/// ```rust,no_run
/// use configwatch::sources::file::FileSource;
/// use std::path::PathBuf;
///
/// let source = FileSource::new(PathBuf::from("config.toml"));
/// let values = source.read().expect("failed to read config");
/// ```
pub struct FileSource {
    path: PathBuf,
    last_modified: std::sync::Mutex<Option<SystemTime>>,
    changed: AtomicBool,
}

impl FileSource {
    /// Create a new FileSource for the given path.
    ///
    /// The file format is determined by the file extension.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_modified: std::sync::Mutex::new(None),
            changed: AtomicBool::new(false),
        }
    }

    /// Get the path of the configuration file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn check_modified(&self) -> bool {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        let modified = match metadata.modified() {
            Ok(m) => m,
            Err(_) => return false,
        };

        let mut last_mod = self.last_modified.lock().unwrap();
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
}

impl ConfigSource for FileSource {
    fn name(&self) -> &str {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
    }

    fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        let content = std::fs::read_to_string(&self.path)?;

        let result = match self.path.extension().and_then(|e| e.to_str()) {
            Some("json") => parse_json(&content),
            Some("toml") => parse_toml(&content),
            Some(ext) => Err(ConfigError::Parse(format!(
                "Unsupported file format: .{}",
                ext
            ))),
            None => Err(ConfigError::Parse(
                "Could not determine file format from extension".to_string(),
            )),
        }?;

        self.changed.store(false, Ordering::Relaxed);
        Ok(result)
    }

    fn has_changed(&self) -> bool {
        self.check_modified()
    }
}

fn parse_json(content: &str) -> Result<HashMap<String, ConfigValue>, ConfigError> {
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| ConfigError::Parse(e.to_string()))?;

    let mut result = HashMap::new();
    if let serde_json::Value::Object(map) = value {
        for (key, val) in map {
            result.insert(key, json_to_config_value(val));
        }
    }

    Ok(result)
}

fn parse_toml(content: &str) -> Result<HashMap<String, ConfigValue>, ConfigError> {
    let value: toml::Value =
        toml::from_str(content).map_err(|e| ConfigError::Parse(e.to_string()))?;

    let mut result = HashMap::new();
    if let toml::Value::Table(table) = value {
        for (key, val) in table {
            result.insert(key, toml_to_config_value(val));
        }
    }

    Ok(result)
}

fn json_to_config_value(value: serde_json::Value) -> ConfigValue {
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
            ConfigValue::Array(arr.into_iter().map(json_to_config_value).collect())
        }
        serde_json::Value::Object(map) => {
            let config_map = map
                .into_iter()
                .map(|(k, v)| (k, json_to_config_value(v)))
                .collect();
            ConfigValue::Map(config_map)
        }
        serde_json::Value::Null => ConfigValue::Null,
    }
}

fn toml_to_config_value(value: toml::Value) -> ConfigValue {
    match value {
        toml::Value::String(s) => ConfigValue::String(s),
        toml::Value::Integer(i) => ConfigValue::Integer(i),
        toml::Value::Float(f) => ConfigValue::Float(f),
        toml::Value::Boolean(b) => ConfigValue::Boolean(b),
        toml::Value::Datetime(dt) => ConfigValue::String(dt.to_string()),
        toml::Value::Array(arr) => {
            ConfigValue::Array(arr.into_iter().map(toml_to_config_value).collect())
        }
        toml::Value::Table(table) => {
            let config_map = table
                .into_iter()
                .map(|(k, v)| (k, toml_to_config_value(v)))
                .collect();
            ConfigValue::Map(config_map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_read_json() {
        let mut file = NamedTempFile::with_suffix(".json").unwrap();
        writeln!(
            file,
            r#"{{
            "database": {{
                "host": "localhost",
                "port": 5432
            }},
            "debug": true,
            "name": "myapp"
        }}"#
        )
        .unwrap();

        let source = FileSource::new(file.path().to_path_buf());
        let values = source.read().unwrap();

        assert_eq!(values.get("name").and_then(|v| v.as_str()), Some("myapp"));
        assert_eq!(values.get("debug").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_read_toml() {
        let mut file = NamedTempFile::with_suffix(".toml").unwrap();
        writeln!(
            file,
            r#"
name = "myapp"
debug = true

[database]
host = "localhost"
port = 5432
"#
        )
        .unwrap();

        let source = FileSource::new(file.path().to_path_buf());
        let values = source.read().unwrap();

        assert_eq!(values.get("name").and_then(|v| v.as_str()), Some("myapp"));
        assert_eq!(values.get("debug").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn test_unsupported_format() {
        let mut file = NamedTempFile::with_suffix(".yaml").unwrap();
        writeln!(file, "key: value").unwrap();

        let source = FileSource::new(file.path().to_path_buf());
        assert!(source.read().is_err());
    }
}
