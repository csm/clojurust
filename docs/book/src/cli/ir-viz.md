# cljrs ir-viz

Render the optimised intermediate representation (IR) for a source file to a
self-contained HTML page.

```
cljrs ir-viz [OPTIONS] <FILE>
```

The HTML output shows the source side-by-side with the IR, with regions
colour-coded by the bump-allocation optimiser's results. Allocations that did
not make it into a region are annotated with their escape verdict and the
blamed use site.

This subcommand is primarily a debugging aid for the IR optimisation pipeline.

## Arguments

| Argument | Description |
|---|---|
| `<FILE>` | Source file to lower to IR |

## Options

### `-o, --out <FILE>`

Output path for the HTML file. If omitted, the output is written alongside
the source file with an `.ir.html` extension:

```
src/myapp/core.cljrs  →  src/myapp/core.cljrs.ir.html
```

### `--src-path <DIR>`

Add `DIR` to the source path for `require` resolution. May be repeated.

### `--quiet`

Suppress the `[ir-viz] wrote …` progress line on stderr.

## Example

```
cljrs ir-viz src/myapp/core.cljrs
# writes: src/myapp/core.cljrs.ir.html

cljrs ir-viz src/myapp/core.cljrs --out /tmp/core.html --quiet
```

Open the resulting HTML file in a browser to explore the IR.

## Interpreting the output

- **Green regions** — allocations placed in a bump-allocation region; they do
  not incur GC heap pressure.
- **Red / yellow annotations** — allocations that escaped the region, labelled
  with the reason (returned, captured by closure, stored in heap object, etc.).
- Clicking a source line highlights the corresponding IR instructions and vice
  versa.
