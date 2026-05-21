# cljrs eval

Evaluate a single Clojure expression and print the result.

```
cljrs eval '<EXPR>'
```

The expression is evaluated in a fresh environment with the standard library
loaded. If the result is non-`nil`, it is printed to stdout. A `nil` result
produces no output.

## Arguments

| Argument | Description |
|---|---|
| `<EXPR>` | The expression to evaluate, as a string |

## Examples

```
cljrs eval '(+ 1 2)'
# → 3

cljrs eval '(map str (range 5))'
# → ("0" "1" "2" "3" "4")

cljrs eval '(println "hello")'
# prints: hello
# (println returns nil so no value line is printed)
```

## Notes

`eval` does not accept `--src-path`. If you need to `require` namespaces
from a source tree, use [`run`](run.md) with a small script file instead.
