# New built-in functions

clojurust adds a small number of built-in functions that have no direct
equivalent in standard Clojure. Most exist because Clojure code would normally
reach these capabilities through Java interop, which is not available in
clojurust.

---

## System / time

### `(sleep ms)`

Pause the current thread for `ms` milliseconds.

```clojure
(sleep 100)   ; sleep 100 ms
```

Clojure equivalent: `(Thread/sleep ms)`.

---

### `(nanotime)`

Return the number of nanoseconds since the Unix epoch as a `Long`.

```clojure
(let [start (nanotime)
      _     (do-work)
      end   (nanotime)]
  (println "elapsed ns:" (- end start)))
```

Clojure equivalent: `(System/nanoTime)` (note: Clojure's version is relative to
an arbitrary origin; clojurust's is Unix epoch-relative).

---

## String utilities

### `(char-code c)`

Return the Unicode code point of character `c` as a `Long`.

```clojure
(char-code \A)    ; => 65
(char-code \λ)    ; => 955
```

Clojure equivalent: `(int c)`.

---

### `(char-at s i)`

Return the character at index `i` in string `s`.

```clojure
(char-at "hello" 1)   ; => \e
```

Clojure equivalent: `(.charAt s i)`.

---

### `(string->list s)`

Convert string `s` to a list of its characters.

```clojure
(string->list "abc")   ; => (\a \b \c)
```

Clojure equivalent: `(seq s)` (returns a seq, not a list, but behaves the same
in most contexts).

---

### `(number->string n)`

Convert number `n` to its string representation.

```clojure
(number->string 42)     ; => "42"
(number->string 3.14)   ; => "3.14"
```

Clojure equivalent: `(str n)`.

---

### `(string->number s)` / `(string->number s base)`

Parse string `s` as a number, returning `nil` if the string is not a valid
number. The optional `base` argument specifies the radix (default 10).

```clojure
(string->number "42")      ; => 42
(string->number "3.14")    ; => 3.14
(string->number "ff" 16)   ; => 255
(string->number "nope")    ; => nil
```

Clojure equivalent: no single equivalent; typically `(Integer/parseInt s)`,
`(Double/parseDouble s)`, or a try/catch around those.

---

## BigDecimal precision

### `(push-precision! n)` / `(push-precision! n mode)`

Push a BigDecimal precision context onto the thread-local precision stack.
Subsequent BigDecimal operations are rounded to `n` significant digits using
`mode` (default `HALF_UP`).

Available rounding modes: `CEILING`, `FLOOR`, `HALF_UP`, `HALF_DOWN`,
`HALF_EVEN`, `UP`, `DOWN`, `UNNECESSARY`.

```clojure
(push-precision! 4)
(/ 1M 3M)    ; => 0.3333 (4 significant digits)
(pop-precision!)
```

This is the lower-level mechanism underlying the `with-precision` macro (which
is preferred in normal code). Use `push-precision!` / `pop-precision!` only
when you need to manage the precision stack manually across multiple calls.

Clojure equivalent: `with-precision` (macro).

---

### `(pop-precision!)`

Pop the most recently pushed BigDecimal precision context.

---

## Persistent queue

### `(queue)` / `(queue capacity)`

Create a new empty persistent queue. The optional `capacity` argument is a size
hint for the initial allocation.

```clojure
(def q (queue))
(def q2 (conj q :a :b :c))
(peek q2)   ; => :a
(pop  q2)   ; => queue with [:b :c]
```

Clojure equivalent: `clojure.lang.PersistentQueue/EMPTY` (a Java static field).

---

## Mutable `ArrayList`

These functions provide a mutable, GC-managed resizable array backed by a Rust
`Vec`. They are intended for performance-sensitive code that builds up a
collection before converting it to an immutable value, or for interoperating
with Rust code that expects a mutable sequence.

An `ArrayList` value is a `NativeObject`; it is not a Clojure collection and
cannot be used with `seq`, `conj`, `map`, etc. directly. Convert it with
`array-list-to-array` first.

### `(array-list)` / `(array-list capacity)`

Create a new empty `ArrayList`. With a `Long` argument, pre-allocates storage
for `capacity` elements.

```clojure
(def al (array-list 16))
```

---

### `(array-list-push al v)`

Append value `v` to the end of `al`. Returns `al` (mutates in place).

```clojure
(array-list-push al :x)
(array-list-push al :y)
```

---

### `(array-list-remove al i)`

Remove and return the element at index `i`. Later elements shift left.

```clojure
(array-list-remove al 0)   ; removes and returns first element
```

---

### `(array-list-length al)`

Return the number of elements in `al` as a `Long`.

```clojure
(array-list-length al)   ; => 2
```

---

### `(array-list-to-array al)`

Convert `al` to an immutable object array (`Value::ObjectArray`). The
`ArrayList` is unaffected.

```clojure
(def arr (array-list-to-array al))
(alength arr)   ; => 2
```

---

### `(array-list-clear al)`

Remove all elements from `al`. Returns `al`.

```clojure
(array-list-clear al)
```

---

## Rust interop

### `(native-object? x)`

Return `true` if `x` is a `NativeObject` (a Rust value wrapped for use in
clojurust), `false` otherwise.

```clojure
(native-object? (array-list))   ; => true
(native-object? [1 2 3])        ; => false
```

---

### `(native-type x)`

Return the type-tag string of `NativeObject` `x`, or `nil` if `x` is not a
`NativeObject`.

```clojure
(native-type (array-list))   ; => "ArrayList"
(native-type 42)             ; => nil
```

The type tag is set by the Rust code that implements the `NativeObject` trait
and is the primary mechanism for dispatching on native types from clojurust
code.
