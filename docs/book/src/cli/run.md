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

Soft memory limit for the GC in megabytes. When live heap exceeds this value,
a collection is triggered at the next safepoint.

### `--gc-hard-limit-mb <MB>`

Hard memory limit for the GC in megabytes. When live heap exceeds this value,
a collection is forced immediately.

## Examples

```
# Run a file in the current directory
cljrs run hello.cljrs

# Run with a source path for namespace resolution
cljrs run --src-path src src/myapp/core.cljrs

# Run and write GC stats to stderr on exit
cljrs --gc-stats run my-program.cljrs
```
