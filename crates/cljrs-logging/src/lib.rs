//! Feature-gated logging for clojurust.
//!
//! Provides per-feature debug and trace logging controlled at runtime via CLI flags.
//! Features are arbitrary strings (e.g. "gc", "jit", "reader") — any feature name
//! is accepted even if no code ever logs with it.
//!
//! # Usage
//!
//! ```ignore
//! // Set features from CLI flags
//! cljrs_logging::set_feature_level("gc", Level::Debug);
//! cljrs_logging::set_feature_level("jit", Level::Trace);
//!
//! // In code:
//! feat_debug!("gc", "starting collection, heap_size={}", heap_size);
//! feat_trace!("gc", "visiting object at {:p}", ptr);
//! ```

use std::collections::HashMap;
use std::sync::RwLock;

static FEATURE_LEVELS: RwLock<Option<HashMap<String, Level>>> = RwLock::new(None);

/// Log level for a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// No logging (default for unregistered features).
    Off = 0,
    /// Debug-level messages.
    Debug = 1,
    /// Trace-level messages (most verbose).
    Trace = 2,
}

/// Set the logging level for a single feature.
pub fn set_feature_level(feature: &str, level: Level) {
    let mut guard = FEATURE_LEVELS.write().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(feature.to_string(), level);
}

/// Get the logging level for a feature. Returns `Level::Off` if the feature
/// has not been configured.
pub fn feature_level(feature: &str) -> Level {
    let guard = FEATURE_LEVELS.read().unwrap();
    guard
        .as_ref()
        .and_then(|m| m.get(feature).copied())
        .unwrap_or(Level::Off)
}

/// Returns true if the given feature is enabled at least at `level`.
#[inline]
pub fn is_enabled(feature: &str, level: Level) -> bool {
    feature_level(feature) >= level
}

/// Parse a `-X` flag value like `"debug:gc,jit"` or `"trace:reader"` and
/// register the appropriate feature levels.
///
/// Format: `<level>:<feature1>,<feature2>,...`
///
/// Returns `Err` with a message if the format is invalid.
pub fn parse_x_flag(value: &str) -> Result<(), String> {
    let (level_str, features_str) = value
        .split_once(':')
        .ok_or_else(|| format!("expected <level>:<features>, got: {value}"))?;

    let level = match level_str {
        "debug" => Level::Debug,
        "trace" => Level::Trace,
        other => return Err(format!("unknown level '{other}', expected 'debug' or 'trace'")),
    };

    for feature in features_str.split(',') {
        let feature = feature.trim();
        if feature.is_empty() {
            continue;
        }
        set_feature_level(feature, level);
    }
    Ok(())
}

/// Log a message at debug level for a feature. Only prints if the feature
/// is enabled at debug level or higher.
///
/// ```ignore
/// feat_debug!("gc", "heap size: {}", size);
/// ```
#[macro_export]
macro_rules! feat_debug {
    ($feature:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
        if $crate::is_enabled($feature, $crate::Level::Debug) {
            eprint!("[DEBUG:{}] ", $feature);
            eprintln!($fmt $(, $arg)*);
        }
    };
}

/// Log a message at trace level for a feature.
///
/// ```ignore
/// feat_trace!("gc", "visiting {:p}", ptr);
/// ```
#[macro_export]
macro_rules! feat_trace {
    ($feature:expr, $fmt:literal $(, $arg:expr)* $(,)?) => {
        if $crate::is_enabled($feature, $crate::Level::Trace) {
            eprint!("[TRACE:{}] ", $feature);
            eprintln!($fmt $(, $arg)*);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_x_flag() {
        parse_x_flag("debug:gc,jit").unwrap();
        assert_eq!(feature_level("gc"), Level::Debug);
        assert_eq!(feature_level("jit"), Level::Debug);
        assert_eq!(feature_level("reader"), Level::Off);
    }

    #[test]
    fn test_parse_x_flag_trace() {
        parse_x_flag("trace:reader").unwrap();
        assert_eq!(feature_level("reader"), Level::Trace);
        assert!(is_enabled("reader", Level::Debug));
        assert!(is_enabled("reader", Level::Trace));
    }

    #[test]
    fn test_parse_x_flag_invalid() {
        assert!(parse_x_flag("bogus").is_err());
        assert!(parse_x_flag("warn:gc").is_err());
    }
}
