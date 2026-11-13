// Smoke test for the wasm32 browser REPL, run under Node.
//
// Build first:
//   wasm-pack build crates/cljrs-wasm --target nodejs --out-dir pkg-node --release
// Then:
//   node crates/cljrs-wasm/tests/node-smoke.cjs crates/cljrs-wasm/pkg-node
//
// Exercises paths that only fail on wasm32 — e.g. a garbage collection that
// called `std::time::Instant::now()` trapped with "time not implemented on
// this platform" (issue #475).  Exits non-zero on any error or mismatch.
const path = require("path");
const pkgDir = path.resolve(process.argv[2] || path.join(__dirname, "..", "pkg-node"));
const { Repl } = require(path.join(pkgDir, "cljrs_wasm.js"));

const cases = [
  // Enough allocation to trigger a GC cycle.
  ["(count (vec (repeat 100000 0)))", "100000"],
  ["(defn f [x y] (reduce + (map (fn [a] a) [x y])))", "#'user/f"],
  ["(defn g [x] (first (reduce (fn [[best bd] j] [j (f x j)]) [0 0] (range 12))))", "#'user/g"],
  ["(reduce + (map (fn [x] (f x (g x))) (range 240)))", "31320"],
  ["(time (+ 1 2))", "3"],
];

(async () => {
  const repl = new Repl();
  // Let the REPL's background init (spawned on the microtask queue) settle.
  await new Promise((resolve) => setTimeout(resolve, 100));
  let failed = false;
  for (const [form, expected] of cases) {
    const started = Date.now();
    const r = await repl.eval(form);
    const ok = !r.is_error && r.result === expected;
    console.log(`${ok ? "ok  " : "FAIL"} ${form} => ${r.result} (${Date.now() - started} ms)`);
    if (r.output) console.log(r.output.trimEnd());
    if (!ok) failed = true;
  }
  process.exit(failed ? 1 : 0);
})().catch((err) => {
  console.error(err);
  process.exit(1);
});
