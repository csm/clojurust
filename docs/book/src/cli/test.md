# cljrs test

Run `clojure.test` test namespaces.

```
cljrs test [OPTIONS] [NAMESPACES...]
```

Loads each namespace, calls `clojure.test/run-tests` on it, and prints a
summary of passes, failures, and errors. Exits with code `0` if all tests
pass, `1` if any fail or error, and `2` if no test namespaces are found.

## Arguments

| Argument | Description |
|---|---|
| `[NAMESPACES...]` | Namespace names to test (e.g. `myapp.core-test`). If omitted, all namespaces in `--src-path` directories are discovered automatically. |

## Options

### `--src-path <DIR>`

Source directory to search for test namespaces. May be repeated. Namespace
discovery translates file paths to namespace names by replacing path separators
with `.` and underscores with `-`.

```
cljrs test --src-path test
```

### `-v, --verbose`

Print each passing assertion as well as failures. Useful for identifying which
test is hanging.

### `--gc-soft-limit-mb <MB>` / `--gc-hard-limit-mb <MB>`

GC memory limits. See [`run`](run.md) for details.

## Namespace discovery

When no explicit namespaces are given, `cljrs test` walks all `--src-path`
directories and converts every `.cljrs` and `.cljc` file to a namespace name:

```
test/myapp/core_test.cljrs  →  myapp.core-test
test/myapp/util_test.cljc   →  myapp.util-test
```

## Output format

```
Ran 12 tests containing 48 assertions across 2 namespace(s) in 0.3s.
48 passed, 0 failed, 0 errors.

All tests passed.
══════════════════════════════════════════════════════════════
```

On failure, a breakdown by namespace is printed before the summary line.

## Examples

```
# Run all tests discovered under test/
cljrs test --src-path test

# Run specific namespaces
cljrs test --src-path test myapp.core-test myapp.util-test

# Verbose output
cljrs test --src-path test --verbose
```
