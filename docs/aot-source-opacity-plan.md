# Plan: Source-Opacity for AOT Binaries

## Purpose

This plan defines the source-opacity goal for AOT binaries.

The primary guarantee is:

> A strict AOT binary contains no source text supplied at compile time by the user or a source dependency.

This guarantee does not require all runtime execution to use native code. Clojure programs must keep dynamic evaluation as a language feature.

Source that arrives after process startup is runtime input. The runtime can interpret or JIT-compile that source.

The JIT is useful in an AOT binary. It lets runtime-defined functions reach native code without placing their source in the artifact.

## Terms

This document uses one name for each source class.

### Compile-time user source

Compile-time user source includes these inputs:

- The entry file.
- Project namespaces.
- Local source dependencies.
- Git source dependencies.
- Pinned versioned namespaces.
- User macro definitions.
- Generated Clojure source that represents any input in this list.

Strict mode must not place this source text in the final artifact.

### Runtime-owned source

Runtime-owned source ships as part of clojurust. Examples include the core bootstrap and standard-library namespaces.

Runtime-owned source is outside the first source-opacity guarantee. The build report must identify it as a separate source class.

This separation keeps the guarantee precise. A later performance project can compile runtime-owned source without changing this plan.

### Runtime input

Runtime input enters the process after startup. Examples include REPL forms, `eval` data, files loaded at runtime, and network input.

The artifact does not contain runtime input at compile time. The interpreter or JIT can process this input.

### Runtime data

Runtime data includes string literals, docstrings, metadata values, symbols, keywords, and quoted data from the program.

Strict mode can store runtime data in the artifact. The compiler must not store surrounding source text to represent that data.

Quoted code is runtime data when the user wrote it as a value. This classification remains valid if the program later passes that value to `eval`.

Provenance decides the classification. The compiler must not label a fallback AST as runtime data.

### Compiled code

Compiled code includes native machine code, WebAssembly, and compiler-generated tables. Compiled code is not source text.

Serialized forms are not compiled code for this guarantee. A serialized AST is another representation of the input program.

This rule applies to compiler fallbacks. It does not prohibit a quoted form that is an intentional runtime value.

## Goals

The implementation must provide these properties:

1. If any compile-time user source needs a source-carrying fallback, strict mode fails.
2. Every compile-time form receives an explicit disposition.
3. The compiler accounts for source dependencies and pinned versioned namespaces.
4. The compiler reports the origin and reason for each rejected form.
5. Runtime `eval` and runtime source loading continue to work.
6. A generated native binary installs the JIT by default.
7. The existing non-strict mode keeps its compatibility behavior.

## Non-goals

This plan does not require an interpreter-free runtime.

This plan does not require native code for every standard-library function.

This plan does not prohibit dynamic source loading.

This plan does not remove the tree-walking or IR interpreters.

This plan does not treat legitimate string literals as source leaks.

This plan does not make binary string scanning the primary proof mechanism.

The first milestone does not add strict mode to the AOT test harness. A later phase extends the same guarantee to test binaries.

## Current behavior

The native AOT compiler partitions forms into a compiled body and interpreted source. It writes interpreted source into the generated harness.

The current audit tracks three source channels:

- The entry preamble.
- A compiled namespace preamble.
- A bundled namespace.

The strict policy rejects these tracked fragments. This policy is a useful first implementation of source opacity.

The current implementation still has important gaps:

- A required namespace can fall back as one complete source file.
- Pinned versioned namespaces always use embedded source.
- Definition forms can require interpreted preambles.
- A new source-writing path can bypass the audit unless a developer updates the fragment list.
- The audit records source fragments, but it does not record a disposition for every input form.
- The test harness always embeds test source and cannot use strict mode.

Runtime bootstrap source and built-in namespaces are not compile-time user source. The report must show these sources in a separate section.

Registration through a built-in-source API does not make source runtime-owned. An extension must declare its provenance explicitly.

The current WebAssembly backend embeds no user source. It can omit unsupported units instead. Strict mode must continue to reject those omissions.

## Source disposition model

The compiler must assign one disposition to every compile-time form.

| Disposition | Meaning | Allowed in strict mode |
|---|---|---|
| `Compiled` | The compiler emitted executable code or a native initialization table. | Yes |
| `Erased` | The form affected compilation only and has no required runtime meaning. | Yes |
| `RuntimeData` | The compiler emitted only the program data that the form denotes. | Yes |
| `RuntimeOwned` | The source belongs to the clojurust runtime, not the compiled program. | Yes, with a report entry |
| `EmbeddedSource` | The harness contains source for later evaluation. | No |
| `Omitted` | The artifact does not represent the form. | No |
| `Rejected` | The compiler stopped because it cannot preserve the form without source. | Build failure |

The compiler must preserve the order and namespace context of all forms that have runtime effects.

An `Erased` disposition requires a semantic reason. For example, a compile-time-only declaration can use this disposition after all uses are resolved.

User macros are not compile-time-only. Runtime `eval` can need the macro var after startup.

## Provenance model

Each compilation unit must carry a source origin. The origin must include these fields:

- The source class.
- The namespace.
- The file or dependency identity.
- The source span.
- The top-level form head, when available.
- The dependency commit, when available.

The compiler must keep this origin through parsing, macro expansion, lowering, code generation, and harness generation.

The artifact audit must consume disposition records. It must not reconstruct coverage from generated files after compilation.

All source-carrying output must pass through one registration function. This function must add an `EmbeddedSource` disposition before it writes data.

## Strict policy

Keep `--require-fully-compiled` as the compatible CLI spelling. Define its contract as a source-opacity and artifact-completeness policy.

Under this policy, the compiler must fail for these conditions:

- A compile-time user form has the `EmbeddedSource` disposition.
- A compile-time user form has the `Omitted` disposition.
- A compilation unit has no disposition.
- A generated source channel has no provenance record.
- A source dependency cannot compile without a source fallback.

The compiler must report all known failures in one result. Each report item must include its origin and its fallback reason.

The default policy can retain source fallback. It must print the same structured report as a warning.

## Definition forms

Source opacity requires native representations for forms that currently use preambles.

### Namespace declarations

The compiler already emits structural setup for a simple `ns` form. Extend that model to all supported namespace clauses.

The generated harness must use namespace and loader APIs directly. It must not reconstruct an `ns` form as source text.

### Requires

Emit complete `RequireSpec` data in the harness. Preserve aliases, refers, version pins, and refer filters.

A required user namespace must compile as its own initialization unit. If compilation fails, strict mode must reject that namespace.

### Macros

Run macros during AOT compilation as required. Compile each macro function body and preserve its macro var.

The compiled macro must remain callable by runtime `eval`. The artifact must not contain the original macro source.

The compiler must preserve every user macro var. A separate closed-world mode can define different rules in a future plan.

### `defonce`

Add a native `defonce` initialization operation. The operation binds a var only if the var has no root value.

The operation must preserve metadata and namespace behavior. It must not evaluate embedded source.

### Protocols and datatypes

Represent protocol, record, type, reify, and extension declarations as native registration data.

Compile each method body through the normal function pipeline. Register the compiled function values in the dispatch tables.

### Multimethods

Represent `defmulti` and `defmethod` as native initialization operations. Compile dispatch and method functions through the normal function pipeline.

### Metadata and interop

Add first-class IR or runtime operations for metadata mutation, method calls, field access, and mutable field writes.

Strict mode must reject unsupported operations. It must not move the complete enclosing definition into source.

## Compilation granularity

The compiler must stop using namespace-wide source fallback in strict mode.

If form ordering permits independent compilation, compile each top-level form independently.

If the function model permits independent compilation, compile each function arity independently.

If one form fails, keep disposition records for the successful forms. Then report the failed form with its exact origin.

The non-strict policy can still embed a failed namespace for compatibility. This fallback must have one explicit audit record.

## Dependencies and versioned namespaces

Apply the same source-opacity rules to project code and source dependencies.

The compiler must compile ordinary source dependencies into namespace initialization units. It must not skip them because they came from another project.

The compiler must compile pinned versioned namespaces from the fetched snapshot. The commit identity must remain part of symbol resolution and provenance.

Native dependencies do not carry Clojure source in the artifact. Their registration data must still appear in the build report.

Runtime-owned namespaces need a separate classification. Their presence must not make the user-source guarantee fail.

## JIT in generated binaries

Generated native AOT binaries must install the JIT on their runtime by default.

The harness can call `cljrs_compiler::jit::install_on(&globals)` after it builds the runtime. The installation must occur before runtime input runs.

Add an explicit build option for environments that prohibit executable-memory generation. The option must disable the JIT without changing source opacity.

Runtime-defined functions follow the normal tier sequence:

```text
runtime source -> tree walk -> IR interpreter -> JIT-native code
```

This sequence does not weaken the AOT guarantee. The runtime source was not present when the compiler built the artifact.

The JIT must not read compile-time source from the artifact. It can compile forms and IR that the running process creates.

## Debug information

Strict mode can include file names, line numbers, symbols, and generated source maps without embedded source content.

Strict mode must reject a source map that contains `sourcesContent` from compile-time user source.

Add a separate opt-in option for users who want embedded source content for debugging. This option must conflict with `--require-fully-compiled`.

## Artifact audit

Replace the current fragment-only audit with a compilation manifest. The manifest must contain one record for every compilation unit.

Each record must contain these fields:

```text
source class
namespace
origin
span
form head
disposition
generated artifact
reason
```

The native and WebAssembly backends must consume the same manifest. Each backend can add backend-specific artifact names and rejection reasons.

If the user requests the manifest, the compiler must write it. The in-memory manifest remains mandatory for every strict build.

Binary string inspection is a defense-in-depth test. It is not the proof of source opacity.

## Implementation phases

### Phase 1: Complete the audit model

- Add source classes, origins, and form dispositions.
- Replace `SourceLeak` collection with the compilation manifest.
- Route every source write through one registered function.
- Keep the current CLI flag and default compatibility policy.
- Report runtime-owned source separately.

### Phase 2: Remove coarse fallback

- Record results per top-level form and function arity.
- Reject only the unsupported units in strict mode.
- Keep namespace-wide fallback only for non-strict compatibility.
- Add precise errors for lowering and code-generation failures.

### Phase 3: Remove user preambles

- Extend structural namespace setup.
- Emit native require data.
- Add native `defonce` initialization.
- Compile runtime-visible macros.
- Add native protocol, datatype, extension, and multimethod registration.
- Add native metadata and interop operations.

### Phase 4: Compile dependencies

- Compile local and Git source dependencies.
- Compile pinned versioned namespaces.
- Preserve commit identity in compiled symbol resolution.
- Reject source fallback from any source dependency in strict mode.
- Compile user test namespaces through the same pipeline after definition-form support is complete.
- Permit strict mode for the AOT test harness after its source channels pass the audit.

### Phase 5: Install the JIT

- Install the JIT in generated native binaries by default.
- Add an explicit no-JIT build option.
- Add a runtime-evaluation test that reaches the native JIT tier.
- Keep source-opacity results identical with and without the JIT.

### Phase 6: Harden the guarantee

- Add final-artifact canary tests for each source channel.
- Add manifest completeness property tests.
- Add negative tests for unregistered source writes.
- Make silent code-generation substitutions return errors.
- Document the guarantee in the CLI and book.

## Test plan

### Manifest tests

- Make sure that every parsed top-level form has one disposition.
- Make sure that macro expansion does not lose source provenance.
- Make sure that generated source writes create `EmbeddedSource` records.
- Make sure that strict mode rejects unknown dispositions.

### Native AOT tests

- Compile an entry namespace with a unique source canary in a comment.
- Inspect the final artifact and make sure that the canary is absent.
- Repeat the test for a required project namespace.
- Repeat the test for a local dependency.
- Repeat the test for a pinned Git dependency.
- Run the artifact and make sure that program behavior is correct.

The canary must not be a string literal, symbol, keyword, docstring, or metadata value. Those values are valid runtime data.

### Definition-form tests

- Compile and run a program that defines a runtime-visible macro.
- Call that macro from a form supplied to `eval` after startup.
- Compile and run `defonce`, protocol, record, type, reify, extension, and multimethod programs.
- Make sure that no test artifact contains the source canary.

### Runtime evaluation tests

- Build a strict AOT binary with no runtime input embedded.
- Supply a new function to the binary after startup.
- Call the function enough times to cross the JIT threshold.
- Make sure that the function reaches the native JIT tier.
- Run the same test with the JIT disabled and make sure that results remain equal.

### Compatibility tests

- Make sure that the default policy still permits source fallback.
- Make sure that each fallback prints a provenance record.
- Make sure that the WebAssembly backend rejects omitted user units in strict mode.
- Make sure that strict mode rejects embedded source maps.
- Make sure that runtime-owned source does not fail the user-source policy.

## Completion criteria

This plan is complete when all these statements are true:

1. A strict build assigns one disposition to every compile-time user form.
2. A strict native artifact contains no compile-time user source text.
3. A strict WebAssembly artifact omits no compile-time user form.
4. Unsupported user forms stop the strict build with precise provenance.
5. Project and source-dependency namespaces follow the same policy.
6. Runtime `eval` can define and run new code after startup.
7. Runtime-defined hot functions can reach JIT-native code in a generated native binary.
8. The default policy retains its current source-fallback behavior.
9. Runtime-owned source appears separately in the build report.
10. Final-artifact canary tests cover every registered source channel.

## Result

The final design keeps the Lisp programming model. Users can load and evaluate new code after startup.

The strict AOT artifact contains only compiled code, runtime data, generated tables, and runtime-owned components. It contains no compile-time user source.

The JIT complements this guarantee. It compiles runtime input without turning that input into compile-time artifact content.
