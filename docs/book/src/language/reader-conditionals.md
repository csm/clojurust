# Reader conditionals

Reader conditionals allow a single source file to contain code for multiple
Clojure platforms. clojurust evaluates the `:rust` branch.

## Syntax

### Non-splicing form

```clojure
#?(:rust   expr-rust
   :clj    expr-jvm
   :cljs   expr-clojurescript
   :default expr-fallback)
```

Exactly one branch is selected at read time based on the current platform. For
clojurust, `:rust` is matched first. If no `:rust` key is present, `:default`
is used. If neither is present, the entire form is skipped (reads as nothing).

The selected branch is a single expression; the whole `#?(...)` form evaluates
to that expression.

```clojure
(def platform #?(:rust   "clojurust"
                 :clj    "JVM Clojure"
                 :cljs   "ClojureScript"
                 :default "unknown"))
```

### Splicing form

```clojure
#?@(:rust   [a b c]
    :clj    [x y z]
    :default [])
```

The splicing form `#?@(...)` selects a **vector** from the active platform and
splices its elements into the surrounding form. It is only valid inside a list,
vector, map, or set literal.

```clojure
;; Adds platform-specific items to a vector
(def features [#?@(:rust   [:gc :cranelift]
                   :clj    [:jvm :hotspot]
                   :default [])])
; => [:gc :cranelift]  (on clojurust)
```

```clojure
;; Platform-specific require in an ns form
(ns myapp.core
  (:require [clojure.string :as str]
            #?@(:rust [[:clojurust.system :as sys]]
                :clj  [[:java.lang.System :as sys]])))
```

## File-extension behaviour

| Extension | Platform dispatch |
|---|---|
| `.cljrs` | Always `:rust`. Reader conditionals are still supported but `:rust` is always the active branch. |
| `.cljc` | Cross-platform. Reader conditional branches are stored as-is; the evaluator selects `:rust`. |

## Notes

- The **reader** stores all branches of a `#?(...)` form; only the evaluator
  discards non-matching branches. This means reader-conditional forms can be
  inspected programmatically without losing the other branches.
- Order within a reader conditional matters: keys are checked left-to-right.
  `:default` should come last.
- Unlike Clojure, there is no `:cljr` (ClojureCLR) platform; the clojurust key
  is `:rust`.
