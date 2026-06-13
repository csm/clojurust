# cljrs-lsp

**Purpose** — A Language Server Protocol (LSP) server for clojurust source
(`.cljrs` / `.cljc`), giving editors live parse diagnostics and a document-symbol
outline.

**Status** — Phase 12 (REPL & Tooling). Implemented, **v1 / syntactic only**: it
uses `cljrs-reader` and does *not* invoke the evaluator. Built on `tower-lsp`.
Deferred to a later semantic tier: hover, completion, go-to-definition,
references, INCREMENTAL text sync, and semantic tokens.

## Capabilities (v1)

- `textDocument/publishDiagnostics` — reader/parse errors, recovered per
  top-level form so multiple errors surface at once.
- `textDocument/documentSymbol` — outline of `ns`, `def`, `defonce`, `defn`,
  `defn-`, `defmacro`, `defmulti`, `deftest`, `definline`, `defprotocol`,
  `defrecord`, `deftype`, `defstruct` (including definitions inside reader
  conditionals).
- Text sync: FULL. Position encoding: negotiates `utf-8`, falls back to `utf-16`.

## File layout

| File | Purpose |
|---|---|
| `src/lib.rs` | Crate root; module wiring; public re-exports. |
| `src/main.rs` | `cljrs-lsp` binary — runs the server over stdio. |
| `src/backend.rs` | `Backend` + the `tower_lsp::LanguageServer` impl; stdio entry points. |
| `src/document.rs` | `Document` (text + version + cached symbols). |
| `src/line_index.rs` | `LineIndex` / `OffsetEncoding`: byte offset ⇄ LSP `Position`/`Range`. |
| `src/recovery.rs` | Lexer-based top-level form chunker for multi-error recovery. |
| `src/diagnostics.rs` | `CljxError` → LSP `Diagnostic`. |
| `src/symbols.rs` | Parsed `Form` tree → `DocumentSymbol`s. |
| `src/analysis.rs` | `run` — orchestrates recovery → parse → diagnostics + symbols (the semantic seam). |
| `tests/smoke.rs` | In-process backend smoke test (open → diagnostics → symbols). |

## Public API

- `cljrs_lsp::Backend` — the language server; construct with `Backend::new(client)`.
- `async fn cljrs_lsp::run_stdio()` — serve over stdin/stdout (needs a runtime).
- `fn cljrs_lsp::run_stdio_blocking() -> std::io::Result<()>` — build a runtime
  and serve; used by the `cljrs lsp` subcommand.
- `cljrs_lsp::LineIndex` / `cljrs_lsp::OffsetEncoding` — position conversion.

## Running

```bash
cljrs lsp          # via the main CLI (recommended)
cljrs-lsp          # the standalone binary
```

Point your editor's generic LSP client at one of the above for `*.cljrs` /
`*.cljc` files.

## Design notes

- **Error recovery without touching the reader.** `Parser::parse_all` aborts on
  the first error, so `recovery.rs` splits the buffer into top-level form chunks
  using only the `Lexer` (which already skips strings/comments/chars), then
  parses each chunk independently. This yields multiple diagnostics *and*
  symbols for the well-formed forms. Recovery stops at the first *lexer-fatal*
  error (e.g. an unterminated string); everything before it is still analyzed.
- **`dashmap`** backs the document store so concurrently-dispatched handlers
  never hold a lock across an `.await`. **No `ropey`** in v1 — FULL sync re-parses
  the whole buffer, so a `String` suffices; `Document` keeps that swappable.
- **Semantic seam.** `analysis::run` is the only place documents are analyzed.
  A v2 would give it an optional `&cljrs_eval::GlobalEnv` (from
  `cljrs_eval::standard_env` / `standard_env_with_paths`) to back hover,
  completion, and go-to-definition via each `Var`'s `:file`/`:line` metadata.
