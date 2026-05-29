# configwatch

[![Crates.io](https://img.shields.io/crates/v/configwatch)](https://crates.io/crates/configwatch)
[![docs.rs](https://img.shields.io/docsrs/configwatch)](https://docs.rs/configwatch)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Build](https://github.com/trevantire/rust-configwatch/actions/workflows/ci.yml/badge.svg)](https://github.com/trevantire/rust-configwatch/actions)

A file and environment variable configuration watcher with hot-reload for Rust applications.

## Features

- **File watching** — Monitor JSON and TOML configuration files for changes
- **Environment variables** — Track prefixed environment variables (`MYAPP_HOST`, `MYAPP_PORT`, etc.)
- **Hot-reload** — Subscribe to changes via async channels, no polling required
- **Thread-safe** — Lock-free reads with `parking_lot` RwLock
- **Custom sources** — Implement `ConfigSource` to add your own backends

## Quick Start

```rust,no_run
use configwatch::{ConfigWatcher, sources::file::FileSource};
use std::path::PathBuf;

fn main() {
    let mut watcher = ConfigWatcher::new();
    watcher.add_source(FileSource::new(PathBuf::from("config.toml")));

    let rx = watcher.subscribe();
    watcher.start().expect("failed to start watcher");

    // Receive config updates on another thread
    while let Ok(update) = rx.recv() {
        println!("Source '{}' changed: {:?}", update.source, update.values);
    }
}
```

## Usage

### File Source

```rust
use configwatch::sources::file::FileSource;
use std::path::PathBuf;

// Watches a JSON or TOML file (detected by extension)
let source = FileSource::new(PathBuf::from("app.json"));
```

### Environment Source

```rust
use configwatch::sources::env::EnvSource;

// Watches all MYAPP_* environment variables
let source = EnvSource::new("MYAPP");

// Use a custom separator for nested keys
// MYAPP__DB__HOST → db.host
let source = EnvSource::new("MYAPP").with_separator("__");
```

### Custom Source

Implement the `ConfigSource` trait:

```rust
use configwatch::{ConfigSource, ConfigValue, ConfigError};
use std::collections::HashMap;

struct VaultSource {
    // your fields
}

impl ConfigSource for VaultSource {
    fn name(&self) -> &str { "vault" }

    fn read(&self) -> Result<HashMap<String, ConfigValue>, ConfigError> {
        // fetch from Vault, Consul, etc.
        Ok(HashMap::new())
    }

    fn has_changed(&self) -> bool {
        // poll or watch for changes
        false
    }
}
```

## API

### `ConfigWatcher`

| Method | Description |
|---|---|
| `new()` | Create a new watcher |
| `add_source(source)` | Register a configuration source |
| `subscribe()` | Get a `Receiver<ConfigUpdate>` for change notifications |
| `start()` | Start the background polling thread |
| `stop()` | Stop the watcher |
| `get_config()` | Get all current config values |
| `get(key)` | Get a specific config value by key |
| `refresh()` | Force re-read of all sources |

### `ConfigValue`

| Variant | Rust Type |
|---|---|
| `String(String)` | `as_str() → Option<&str>` |
| `Integer(i64)` | `as_i64() → Option<i64>` |
| `Float(f64)` | `as_f64() → Option<f64>` |
| `Boolean(bool)` | `as_bool() → Option<bool>` |
| `Array(Vec<ConfigValue>)` | — |
| `Map(HashMap<String, ConfigValue>)` | — |
| `Null` | `is_null() → bool` |

### `ConfigUpdate`

```rust
pub struct ConfigUpdate {
    pub source: String,                    // which source changed
    pub values: HashMap<String, ConfigValue>, // new values
    pub timestamp: std::time::Instant,     // when it changed
}
```

## License

MIT — see [LICENSE](LICENSE).

<!-- history: 2026-05-17 -->

<!-- history: 2026-05-21 -->

<!-- history: 2026-05-23 -->

<!-- history: 2026-05-29 -->
