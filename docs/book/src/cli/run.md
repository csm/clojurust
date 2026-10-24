# cljrs run

Interpret a `.cljrs` or `.cljc` source file.

```
cljrs run [OPTIONS] <FILE>
```

All top-level forms in `FILE` are evaluated in order. The return value of the
last form is discarded; side effects (output, file writes, etc.) are the
intended mechanism for a `run` program to produce observable results.

## Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Path to the source file (`.cljrs` or `.cljc`) |

## Options

### `--src-path <DIR>`

Add `DIR` to the list of directories searched when resolving `require`. May be
repeated to add multiple directories.

```
cljrs run --src-path src --src-path lib my-program.cljrs
```

Paths declared in `:paths` of the nearest `cljrs.edn` are appended automatically
after CLI paths.

### `--gc-soft-limit-mb <MB>`

Soft memory limit for the GC in megabytes. When one isolate's heap exceeds
this value, a collection is triggered at the next safepoint. Each isolate heap
applies the limit independently, so *N* isolates can together hold about *N*
times this value.

The flag does not change the process-wide memory governor's soft limit. That
limit comes from `--gc-hard-limit-mb` (75% of it), from
`CLJRS_MEMORY_SOFT_LIMIT_MB`, or from the platform default. When managed
memory across all isolates reaches it, the governor asks the allocating
isolate (and the largest heap) to collect.

Given alone, it leaves the hard limit at its default (raised to the soft limit
if the soft limit is larger).

### `--gc-hard-limit-mb <MB>`

Process-wide managed-memory hard limit in megabytes. It is **not enforced
yet**: the memory governor runs in observe-only mode, and allocations above the
limit succeed. `--gc-stats` reports how many allocations went over it. The
governor's soft limit is 75% of this value. Managed memory covers isolate GC
heaps, region chunks, shared byte blobs and `shared-atom` values, queued
isolate-channel messages, static data, and JIT code; it is not the process
resident set. The flag takes precedence over
`CLJRS_MEMORY_HARD_LIMIT_MB` and `CLJRS_MEMORY_SOFT_LIMIT_MB`.

A zero hard limit, or a soft limit above the hard limit, is rejected.

## Examples

```
# Run a file in the current directory
cljrs run hello.cljrs

# Run with a source path for namespace resolution
cljrs run --src-path src src/myapp/core.cljrs

# Run and write GC stats to stderr on exit
cljrs --gc-stats run my-program.cljrs
```
