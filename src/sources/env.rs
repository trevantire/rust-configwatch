//! Environment variable configuration source.
//!
//! Monitors environment variables for changes and provides them as configuration values.

use crate::{ConfigError, ConfigSource, ConfigValue};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// A configuration source that reads from environment variables.
///
/// Supports optional prefix filtering and custom separator for nested keys.
///
/// # Example
///
/// ```rust,no_run
/// use configwatch::sources::env::EnvSource;
///
/// // Watch all MYAPP_* environment variables
/// let source = EnvSource::new("MYAPP");
///
/// // Read current values
/// let values = source.read().unwrap();
/// ```
pub struct EnvSource {
    prefix: String,
    separator: String,
    last_values: RwLock<HashMap<String, ConfigValue>>,
    changed: AtomicBool,
}

impl EnvSource {
    /// Create a new EnvSource that watches variables with the given prefix.
    ///
    /// The prefix is used to filter environment variables. For example,
    /// a prefix of "MYAPP" will watch variables like `MYAPP_HOST`, `MYAPP_PORT`, etc.
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            separator: "_".to_string(),
            last_values: RwLock::new(HashMap::new()),
            changed: AtomicBool::new(false),
        }
    }

    /// Set the separator used for nested keys.
    ///
    /// Default separator is "_". For example, with prefix "MYAPP" and
    /// separator "__", the variable `MYAPP__DATABASE__HOST` would become
    /// the nested key `database.host`.
    pub fn with_separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }

    /// Get the configured prefix.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    fn env_key_to_config_key(&self, env_key: &str) -> Option<String> {
        let prefix_with_sep = format!("{}{}", self.prefix, self.separator);
        if env_key.starts_with(&prefix_with_sep) {
            let key = &env_key[prefix_with_sep.len()..];
            Some(key.to_lowercase())
        } else if env_key.starts_with(&self.prefix) {
            let key = &env_key[self.prefix.len()..];
            if key.starts_with(&self.separator) {
                Some(key[self.separator.len()..].to_lowercase())
            } else {
                None
            }
        } else {
            None
        }
    }

    fn parse_value(s: &str) -> ConfigValue {
        // Try to parse as different types
        if s.eq_ignore_ascii_case("true") {
            return ConfigValue::Boolean(true);
        }
        if s.eq_ignore_ascii_case("false") {
            return ConfigValue::Boolean(false);
        }

        // Try integer
        if let Ok(i) = s.parse::<i64>() {
            return ConfigValue::Integer(i);
        }

        // Try float
        if let Ok(f) = s.parse::<f64>() {
            return ConfigValue::Float(f);
        }

        // Default to string
        ConfigValue::String(s.to_string())
    }
}

impl ConfigSource for EnvSource {
    fn name(&self) -> &str {
        "env"
    }

    fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        let mut values = HashMap::new();

        for (key, value) in std::env::vars() {
            if let Some(config_key) = self.env_key_to_config_key(&key) {
                values.insert(config_key, Self::parse_value(&value));
            }
        }

        // Check if values changed
        let last = self.last_values.read();
        let changed = *last != values;
        drop(last);

        if changed {
            *self.last_values.write() = values.clone();
            self.changed.store(true, Ordering::Relaxed);
        }

        Ok(values)
    }

    fn has_changed(&self) -> bool {
        // Re-read to check for changes
        let _ = self.read();
        self.changed.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_key_parsing() {
        let source = EnvSource::new("TESTAPP");

        assert_eq!(
            source.env_key_to_config_key("TESTAPP_HOST"),
            Some("host".to_string())
        );
        assert_eq!(
            source.env_key_to_config_key("TESTAPP_PORT"),
            Some("port".to_string())
        );
        assert_eq!(
            source.env_key_to_config_key("OTHER_VAR"),
            None
        );
    }

    #[test]
    fn test_value_parsing() {
        assert_eq!(EnvSource::parse_value("true"), ConfigValue::Boolean(true));
        assert_eq!(EnvSource::parse_value("false"), ConfigValue::Boolean(false));
        assert_eq!(EnvSource::parse_value("42"), ConfigValue::Integer(42));
        assert_eq!(EnvSource::parse_value("3.14"), ConfigValue::Float(3.14));
        assert_eq!(
            EnvSource::parse_value("hello"),
            ConfigValue::String("hello".to_string())
        );
    }

    #[test]
    fn test_read_env_vars() {
        std::env::set_var("CONFIGWATCH_TEST_VALUE", "42");
        std::env::set_var("CONFIGWATCH_TEST_NAME", "test");

        let source = EnvSource::new("CONFIGWATCH");
        let values = source.read().unwrap();

        assert_eq!(
            values.get("test_value").and_then(|v| v.as_i64()),
            Some(42)
        );
        assert_eq!(
            values.get("test_name").and_then(|v| v.as_str()),
            Some("test")
        );

        // Cleanup
        std::env::remove_var("CONFIGWATCH_TEST_VALUE");
        std::env::remove_var("CONFIGWATCH_TEST_NAME");
    }

    #[test]
    fn test_custom_separator() {
        std::env::set_var("MYAPP__DB__HOST", "localhost");

        let source = EnvSource::new("MYAPP").with_separator("__");
        let values = source.read().unwrap();

        assert_eq!(
            values.get("db__host").and_then(|v| v.as_str()),
            Some("localhost")
        );

        std::env::remove_var("MYAPP__DB__HOST");
    }
}
