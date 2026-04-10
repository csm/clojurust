# cljrs-logging

Feature-gated debug/trace logging for clojurust.

## Status

Implemented. Provides runtime-configurable per-feature logging at debug and trace levels.

## File layout

- `src/lib.rs` — global feature level state, `parse_x_flag`, `feat_debug!` / `feat_trace!` macros

## Public API

- `enum Level` — `Off`, `Debug`, `Trace` (ordered by verbosity)
- `fn set_feature_level(feature: &str, level: Level)` — set the log level for a feature
- `fn feature_level(feature: &str) -> Level` — query the current level for a feature
- `fn is_enabled(feature: &str, level: Level) -> bool` — check if a feature is enabled at a level
- `fn parse_x_flag(value: &str) -> Result<(), String>` — parse a `-X debug:gc,jit` style flag string
- `feat_debug!(feature, fmt, args...)` — log at debug level for a feature (to stderr)
- `feat_trace!(feature, fmt, args...)` — log at trace level for a feature (to stderr)
