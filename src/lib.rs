//! # ConfigWatch
//!
//! A file and environment config watcher with hot-reload for Rust applications.
//!
//! ## Features
//!
//! - Watch configuration files (JSON, TOML) for changes
//! - Monitor environment variables for updates
//! - Subscribe to configuration changes via channels
//! - Thread-safe configuration access
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use configwatch::{ConfigWatcher, sources::file::FileSource};
//! use std::path::PathBuf;
//!
//! let mut watcher = ConfigWatcher::new();
//! watcher.add_source(FileSource::new(PathBuf::from("config.toml")));
//!
//! let rx = watcher.subscribe();
//! watcher.start().expect("failed to start watcher");
//!
//! // In another thread, receive config updates
//! while let Ok(update) = rx.recv() {
//!     println!("Config changed: {:?}", update);
//! }
//! ```

pub mod sources;
pub mod watcher;

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Errors that can occur during config watching operations.
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Watch error: {0}")]
    Watch(String),

    #[error("Source not found: {0}")]
    SourceNotFound(String),

    #[error("Channel closed")]
    ChannelClosed,
}

/// Represents a configuration value that can be of various types.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<ConfigValue>),
    Map(HashMap<String, ConfigValue>),
    Null,
}

impl ConfigValue {
    /// Try to get the value as a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ConfigValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get the value as an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            ConfigValue::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// Try to get the value as a float.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ConfigValue::Float(f) => Some(*f),
            ConfigValue::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }

    /// Try to get the value as a boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConfigValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Check if the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, ConfigValue::Null)
    }
}

impl From<String> for ConfigValue {
    fn from(s: String) -> Self {
        ConfigValue::String(s)
    }
}

impl From<&str> for ConfigValue {
    fn from(s: &str) -> Self {
        ConfigValue::String(s.to_string())
    }
}

impl From<i64> for ConfigValue {
    fn from(i: i64) -> Self {
        ConfigValue::Integer(i)
    }
}

impl From<f64> for ConfigValue {
    fn from(f: f64) -> Self {
        ConfigValue::Float(f)
    }
}

impl From<bool> for ConfigValue {
    fn from(b: bool) -> Self {
        ConfigValue::Boolean(b)
    }
}

/// Represents a configuration update notification.
#[derive(Debug, Clone)]
pub struct ConfigUpdate {
    /// The name of the source that changed.
    pub source: String,
    /// The updated configuration values.
    pub values: HashMap<String, ConfigValue>,
    /// Timestamp of the update.
    pub timestamp: std::time::Instant,
}

/// Trait for configuration sources (files, env vars, etc.).
pub trait ConfigSource: Send + Sync {
    /// Returns the name of this source.
    fn name(&self) -> &str;

    /// Reads the current configuration values from this source.
    fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError>;

    /// Returns true if this source has changed since the last read.
    fn has_changed(&self) -> bool;
}

/// The main configuration watcher that coordinates multiple sources.
pub struct ConfigWatcher {
    sources: Vec<Arc<dyn ConfigSource>>,
    current_config: Arc<RwLock<HashMap<String, ConfigValue>>>,
    subscribers: Vec<crossbeam_channel::Sender<ConfigUpdate>>,
    watcher_handle: Option<std::thread::JoinHandle<()>>,
    shutdown: Arc<std::sync::atomic::AtomicBool>,
}

impl ConfigWatcher {
    /// Create a new ConfigWatcher.
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            current_config: Arc::new(RwLock::new(HashMap::new())),
            subscribers: Vec::new(),
            watcher_handle: None,
            shutdown: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Add a configuration source to watch.
    pub fn add_source<S: ConfigSource + 'static>(&mut self, source: S) {
        self.sources.push(Arc::new(source));
    }

    /// Subscribe to configuration changes.
    /// Returns a receiver that will get ConfigUpdate messages.
    pub fn subscribe(&mut self) -> crossbeam_channel::Receiver<ConfigUpdate> {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.subscribers.push(tx);
        rx
    }

    /// Start watching for configuration changes.
    pub fn start(&mut self) -> Result<(), ConfigError> {
        // Initial read of all sources
        for source in &self.sources {
            match source.read() {
                Ok(values) => {
                    let mut config = self.current_config.write();
                    for (key, value) in values {
                        config.insert(key, value);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to read source '{}': {}", source.name(), e);
                }
            }
        }

        // Start the watcher thread
        let sources = self.sources.clone();
        let current_config = self.current_config.clone();
        let subscribers = self.subscribers.clone();
        let shutdown = self.shutdown.clone();

        let handle = std::thread::spawn(move || {
            Self::watch_loop(sources, current_config, subscribers, shutdown);
        });

        self.watcher_handle = Some(handle);
        Ok(())
    }

    /// Stop watching for configuration changes.
    pub fn stop(&mut self) {
        self.shutdown.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(handle) = self.watcher_handle.take() {
            let _ = handle.join();
        }
    }

    /// Get the current configuration values.
    pub fn get_config(&self) -> HashMap<String, ConfigValue> {
        self.current_config.read().clone()
    }

    /// Get a specific configuration value by key.
    pub fn get(&self, key: &str) -> Option<ConfigValue> {
        self.current_config.read().get(key).cloned()
    }

    /// Force a refresh of all sources.
    pub fn refresh(&self) -> Result<(), ConfigError> {
        let mut updates = HashMap::new();

        for source in &self.sources {
            match source.read() {
                Ok(values) => {
                    for (key, value) in values {
                        updates.insert(key, value);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to refresh source '{}': {}", source.name(), e);
                }
            }
        }

        let mut config = self.current_config.write();
        *config = updates;

        Ok(())
    }

    fn watch_loop(
        sources: Vec<Arc<dyn ConfigSource>>,
        current_config: Arc<RwLock<HashMap<String, ConfigValue>>>,
        subscribers: Vec<crossbeam_channel::Sender<ConfigUpdate>>,
        shutdown: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let poll_interval = Duration::from_millis(500);

        while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(poll_interval);

            for source in &sources {
                if source.has_changed() {
                    match source.read() {
                        Ok(values) => {
                            let update = ConfigUpdate {
                                source: source.name().to_string(),
                                values: values.clone(),
                                timestamp: std::time::Instant::now(),
                            };

                            // Update current config
                            {
                                let mut config = current_config.write();
                                for (key, value) in &values {
                                    config.insert(key.clone(), value.clone());
                                }
                            }

                            // Notify subscribers
                            for tx in &subscribers {
                                let _ = tx.send(update.clone());
                            }
                        }
                        Err(e) => {
                            log::error!("Error reading source '{}': {}", source.name(), e);
                        }
                    }
                }
            }
        }
    }
}

impl Default for ConfigWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MockSource {
        name: String,
        values: parking_lot::Mutex<HashMap<String, ConfigValue>>,
        changed: std::sync::atomic::AtomicBool,
    }

    impl MockSource {
        fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                values: parking_lot::Mutex::new(HashMap::new()),
                changed: std::sync::atomic::AtomicBool::new(false),
            }
        }

        fn set_value(&self, key: &str, value: ConfigValue) {
            self.values.lock().insert(key.to_string(), value);
            self.changed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    impl ConfigSource for MockSource {
        fn name(&self) -> &str {
            &self.name
        }

        fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
            self.changed.store(false, std::sync::atomic::Ordering::Relaxed);
            Ok(self.values.lock().clone())
        }

        fn has_changed(&self) -> bool {
            self.changed.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[test]
    fn test_config_value_conversions() {
        let val = ConfigValue::from("hello");
        assert_eq!(val.as_str(), Some("hello"));

        let val = ConfigValue::from(42i64);
        assert_eq!(val.as_i64(), Some(42));

        let val = ConfigValue::from(true);
        assert_eq!(val.as_bool(), Some(true));

        let val = ConfigValue::Null;
        assert!(val.is_null());
    }

    #[test]
    fn test_config_watcher_subscribe() {
        let mut watcher = ConfigWatcher::new();
        let source = MockSource::new("test");
        source.set_value("key1", ConfigValue::from("value1"));

        watcher.add_source(source);
        let rx = watcher.subscribe();

        watcher.start().unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let config = watcher.get_config();
        assert_eq!(config.get("key1").and_then(|v| v.as_str()), Some("value1"));

        watcher.stop();
    }
}
