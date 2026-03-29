//! Auto-generated AOT test harness for clojurust.
//!
//! Discovers and runs all clojure.test tests in the bundled namespaces.

use cljrs_value::Value;

fn main() {
    // Initialize the standard environment.
    let globals = cljrs_stdlib::standard_env();

    // Register bundled dependency sources so require can find them
    // without needing source files on disk.
    globals.register_builtin_source("clojure.core-test.abs", include_str!("bundled_0.cljrs"));
    globals.register_builtin_source("clojure.core-test.aclone", include_str!("bundled_1.cljrs"));
    globals.register_builtin_source("clojure.core-test.add-watch", include_str!("bundled_2.cljrs"));
    globals.register_builtin_source("clojure.core-test.ancestors", include_str!("bundled_3.cljrs"));
    globals.register_builtin_source("clojure.core-test.and", include_str!("bundled_4.cljrs"));
    globals.register_builtin_source("clojure.core-test.any-qmark", include_str!("bundled_5.cljrs"));
    globals.register_builtin_source("clojure.core-test.assoc", include_str!("bundled_6.cljrs"));
    globals.register_builtin_source("clojure.core-test.assoc-bang", include_str!("bundled_7.cljrs"));
    globals.register_builtin_source("clojure.core-test.associative-qmark", include_str!("bundled_8.cljrs"));
    globals.register_builtin_source("clojure.core-test.atom", include_str!("bundled_9.cljrs"));
    globals.register_builtin_source("clojure.core-test.bigdec", include_str!("bundled_10.cljrs"));
    globals.register_builtin_source("clojure.core-test.bigint", include_str!("bundled_11.cljrs"));
    globals.register_builtin_source("clojure.core-test.binding", include_str!("bundled_12.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-and", include_str!("bundled_13.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-and-not", include_str!("bundled_14.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-clear", include_str!("bundled_15.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-flip", include_str!("bundled_16.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-not", include_str!("bundled_17.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-or", include_str!("bundled_18.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-set", include_str!("bundled_19.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-shift-left", include_str!("bundled_20.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-shift-right", include_str!("bundled_21.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-test", include_str!("bundled_22.cljrs"));
    globals.register_builtin_source("clojure.core-test.bit-xor", include_str!("bundled_23.cljrs"));
    globals.register_builtin_source("clojure.core-test.boolean", include_str!("bundled_24.cljrs"));
    globals.register_builtin_source("clojure.core-test.boolean-qmark", include_str!("bundled_25.cljrs"));
    globals.register_builtin_source("clojure.core-test.bound-fn", include_str!("bundled_26.cljrs"));
    globals.register_builtin_source("clojure.core-test.bound-fn-star", include_str!("bundled_27.cljrs"));
    globals.register_builtin_source("clojure.core-test.butlast", include_str!("bundled_28.cljrs"));
    globals.register_builtin_source("clojure.core-test.byte", include_str!("bundled_29.cljrs"));
    globals.register_builtin_source("clojure.core-test.case", include_str!("bundled_30.cljrs"));
    globals.register_builtin_source("clojure.core-test.char", include_str!("bundled_31.cljrs"));
    globals.register_builtin_source("clojure.core-test.char-qmark", include_str!("bundled_32.cljrs"));
    globals.register_builtin_source("clojure.core-test.coll-qmark", include_str!("bundled_33.cljrs"));
    globals.register_builtin_source("clojure.core-test.comment", include_str!("bundled_34.cljrs"));
    globals.register_builtin_source("clojure.core-test.compare", include_str!("bundled_35.cljrs"));
    globals.register_builtin_source("clojure.core-test.conj", include_str!("bundled_36.cljrs"));
    globals.register_builtin_source("clojure.core-test.conj-bang", include_str!("bundled_37.cljrs"));
    globals.register_builtin_source("clojure.core-test.cons", include_str!("bundled_38.cljrs"));
    globals.register_builtin_source("clojure.core-test.constantly", include_str!("bundled_39.cljrs"));
    globals.register_builtin_source("clojure.core-test.contains-qmark", include_str!("bundled_40.cljrs"));
    globals.register_builtin_source("clojure.core-test.count", include_str!("bundled_41.cljrs"));
    globals.register_builtin_source("clojure.core-test.counted-qmark", include_str!("bundled_42.cljrs"));
    globals.register_builtin_source("clojure.core-test.cycle", include_str!("bundled_43.cljrs"));
    globals.register_builtin_source("clojure.core-test.dec", include_str!("bundled_44.cljrs"));
    globals.register_builtin_source("clojure.core-test.decimal-qmark", include_str!("bundled_45.cljrs"));
    globals.register_builtin_source("clojure.core-test.denominator", include_str!("bundled_46.cljrs"));
    globals.register_builtin_source("clojure.core-test.derive", include_str!("bundled_47.cljrs"));
    globals.register_builtin_source("clojure.core-test.descendants", include_str!("bundled_48.cljrs"));
    globals.register_builtin_source("clojure.core-test.disj", include_str!("bundled_49.cljrs"));
    globals.register_builtin_source("clojure.core-test.disj-bang", include_str!("bundled_50.cljrs"));
    globals.register_builtin_source("clojure.core-test.dissoc", include_str!("bundled_51.cljrs"));
    globals.register_builtin_source("clojure.core-test.dissoc-bang", include_str!("bundled_52.cljrs"));
    globals.register_builtin_source("clojure.core-test.doseq", include_str!("bundled_53.cljrs"));
    globals.register_builtin_source("clojure.core-test.double", include_str!("bundled_54.cljrs"));
    globals.register_builtin_source("clojure.core-test.double-qmark", include_str!("bundled_55.cljrs"));
    globals.register_builtin_source("clojure.core-test.drop", include_str!("bundled_56.cljrs"));
    globals.register_builtin_source("clojure.core-test.drop-last", include_str!("bundled_57.cljrs"));
    globals.register_builtin_source("clojure.core-test.drop-while", include_str!("bundled_58.cljrs"));
    globals.register_builtin_source("clojure.core-test.empty", include_str!("bundled_59.cljrs"));
    globals.register_builtin_source("clojure.core-test.empty-qmark", include_str!("bundled_60.cljrs"));
    globals.register_builtin_source("clojure.core-test.eq", include_str!("bundled_61.cljrs"));
    globals.register_builtin_source("clojure.core-test.even-qmark", include_str!("bundled_62.cljrs"));
    globals.register_builtin_source("clojure.core-test.false-qmark", include_str!("bundled_63.cljrs"));
    globals.register_builtin_source("clojure.core-test.ffirst", include_str!("bundled_64.cljrs"));
    globals.register_builtin_source("clojure.core-test.find", include_str!("bundled_65.cljrs"));
    globals.register_builtin_source("clojure.core-test.first", include_str!("bundled_66.cljrs"));
    globals.register_builtin_source("clojure.core-test.float", include_str!("bundled_67.cljrs"));
    globals.register_builtin_source("clojure.core-test.float-qmark", include_str!("bundled_68.cljrs"));
    globals.register_builtin_source("clojure.core-test.fn-qmark", include_str!("bundled_69.cljrs"));
    globals.register_builtin_source("clojure.core-test.fnext", include_str!("bundled_70.cljrs"));
    globals.register_builtin_source("clojure.core-test.fnil", include_str!("bundled_71.cljrs"));
    globals.register_builtin_source("clojure.core-test.format", include_str!("bundled_72.cljrs"));
    globals.register_builtin_source("clojure.core-test.get", include_str!("bundled_73.cljrs"));
    globals.register_builtin_source("clojure.core-test.get-in", include_str!("bundled_74.cljrs"));
    globals.register_builtin_source("clojure.core-test.gt", include_str!("bundled_75.cljrs"));
    globals.register_builtin_source("clojure.core-test.hash-map", include_str!("bundled_76.cljrs"));
    globals.register_builtin_source("clojure.core-test.hash-set", include_str!("bundled_77.cljrs"));
    globals.register_builtin_source("clojure.core-test.ident-qmark", include_str!("bundled_78.cljrs"));
    globals.register_builtin_source("clojure.core-test.identical-qmark", include_str!("bundled_79.cljrs"));
    globals.register_builtin_source("clojure.core-test.ifn-qmark", include_str!("bundled_80.cljrs"));
    globals.register_builtin_source("clojure.core-test.inc", include_str!("bundled_81.cljrs"));
    globals.register_builtin_source("clojure.core-test.int", include_str!("bundled_82.cljrs"));
    globals.register_builtin_source("clojure.core-test.int-qmark", include_str!("bundled_83.cljrs"));
    globals.register_builtin_source("clojure.core-test.integer-qmark", include_str!("bundled_84.cljrs"));
    globals.register_builtin_source("clojure.core-test.interleave", include_str!("bundled_85.cljrs"));
    globals.register_builtin_source("clojure.core-test.intern", include_str!("bundled_86.cljrs"));
    globals.register_builtin_source("clojure.core-test.interpose", include_str!("bundled_87.cljrs"));
    globals.register_builtin_source("clojure.core-test.juxt", include_str!("bundled_88.cljrs"));
    globals.register_builtin_source("clojure.core-test.key", include_str!("bundled_89.cljrs"));
    globals.register_builtin_source("clojure.core-test.keys", include_str!("bundled_90.cljrs"));
    globals.register_builtin_source("clojure.core-test.keyword", include_str!("bundled_91.cljrs"));
    globals.register_builtin_source("clojure.core-test.keyword-qmark", include_str!("bundled_92.cljrs"));
    globals.register_builtin_source("clojure.core-test.last", include_str!("bundled_93.cljrs"));
    globals.register_builtin_source("clojure.core-test.list-qmark", include_str!("bundled_94.cljrs"));
    globals.register_builtin_source("clojure.core-test.long", include_str!("bundled_95.cljrs"));
    globals.register_builtin_source("clojure.core-test.lt", include_str!("bundled_96.cljrs"));
    globals.register_builtin_source("clojure.core-test.lt-eq", include_str!("bundled_97.cljrs"));
    globals.register_builtin_source("clojure.core-test.make-hierarchy", include_str!("bundled_98.cljrs"));
    globals.register_builtin_source("clojure.core-test.map-qmark", include_str!("bundled_99.cljrs"));
    globals.register_builtin_source("clojure.core-test.mapcat", include_str!("bundled_100.cljrs"));
    globals.register_builtin_source("clojure.core-test.max", include_str!("bundled_101.cljrs"));
    globals.register_builtin_source("clojure.core-test.merge", include_str!("bundled_102.cljrs"));
    globals.register_builtin_source("clojure.core-test.min", include_str!("bundled_103.cljrs"));
    globals.register_builtin_source("clojure.core-test.min-key", include_str!("bundled_104.cljrs"));
    globals.register_builtin_source("clojure.core-test.minus", include_str!("bundled_105.cljrs"));
    globals.register_builtin_source("clojure.core-test.mod", include_str!("bundled_106.cljrs"));
    globals.register_builtin_source("clojure.core-test.name", include_str!("bundled_107.cljrs"));
    globals.register_builtin_source("clojure.core-test.namespace", include_str!("bundled_108.cljrs"));
    globals.register_builtin_source("clojure.core-test.nan-qmark", include_str!("bundled_109.cljrs"));
    globals.register_builtin_source("clojure.core-test.neg-int-qmark", include_str!("bundled_110.cljrs"));
    globals.register_builtin_source("clojure.core-test.neg-qmark", include_str!("bundled_111.cljrs"));
    globals.register_builtin_source("clojure.core-test.next", include_str!("bundled_112.cljrs"));
    globals.register_builtin_source("clojure.core-test.nfirst", include_str!("bundled_113.cljrs"));
    globals.register_builtin_source("clojure.core-test.nil-qmark", include_str!("bundled_114.cljrs"));
    globals.register_builtin_source("clojure.core-test.nnext", include_str!("bundled_115.cljrs"));
    globals.register_builtin_source("clojure.core-test.not", include_str!("bundled_116.cljrs"));
    globals.register_builtin_source("clojure.core-test.not-empty", include_str!("bundled_117.cljrs"));
    globals.register_builtin_source("clojure.core-test.not-eq", include_str!("bundled_118.cljrs"));
    globals.register_builtin_source("clojure.core-test.nth", include_str!("bundled_119.cljrs"));
    globals.register_builtin_source("clojure.core-test.nthnext", include_str!("bundled_120.cljrs"));
    globals.register_builtin_source("clojure.core-test.nthrest", include_str!("bundled_121.cljrs"));
    globals.register_builtin_source("clojure.core-test.num", include_str!("bundled_122.cljrs"));
    globals.register_builtin_source("clojure.core-test.number-qmark", include_str!("bundled_123.cljrs"));
    globals.register_builtin_source("clojure.core-test.number-range", include_str!("bundled_124.cljrs"));
    globals.register_builtin_source("clojure.core-test.numerator", include_str!("bundled_125.cljrs"));
    globals.register_builtin_source("clojure.core-test.odd-qmark", include_str!("bundled_126.cljrs"));
    globals.register_builtin_source("clojure.core-test.or", include_str!("bundled_127.cljrs"));
    globals.register_builtin_source("clojure.core-test.parents", include_str!("bundled_128.cljrs"));
    globals.register_builtin_source("clojure.core-test.parse-boolean", include_str!("bundled_129.cljrs"));
    globals.register_builtin_source("clojure.core-test.parse-double", include_str!("bundled_130.cljrs"));
    globals.register_builtin_source("clojure.core-test.parse-long", include_str!("bundled_131.cljrs"));
    globals.register_builtin_source("clojure.core-test.parse-uuid", include_str!("bundled_132.cljrs"));
    globals.register_builtin_source("clojure.core-test.partial", include_str!("bundled_133.cljrs"));
    globals.register_builtin_source("clojure.core-test.peek", include_str!("bundled_134.cljrs"));
    globals.register_builtin_source("clojure.core-test.persistent-bang", include_str!("bundled_135.cljrs"));
    globals.register_builtin_source("clojure.core-test.plus", include_str!("bundled_136.cljrs"));
    globals.register_builtin_source("clojure.core-test.plus-squote", include_str!("bundled_137.cljrs"));
    globals.register_builtin_source("clojure.core-test.pop", include_str!("bundled_138.cljrs"));
    globals.register_builtin_source("clojure.core-test.pop-bang", include_str!("bundled_139.cljrs"));
    globals.register_builtin_source("clojure.core-test.portability", include_str!("bundled_140.cljrs"));
    globals.register_builtin_source("clojure.core-test.pos-int-qmark", include_str!("bundled_141.cljrs"));
    globals.register_builtin_source("clojure.core-test.pos-qmark", include_str!("bundled_142.cljrs"));
    globals.register_builtin_source("clojure.core-test.pr-str", include_str!("bundled_143.cljrs"));
    globals.register_builtin_source("clojure.core-test.print-str", include_str!("bundled_144.cljrs"));
    globals.register_builtin_source("clojure.core-test.println-str", include_str!("bundled_145.cljrs"));
    globals.register_builtin_source("clojure.core-test.prn-str", include_str!("bundled_146.cljrs"));
    globals.register_builtin_source("clojure.core-test.qualified-ident-qmark", include_str!("bundled_147.cljrs"));
    globals.register_builtin_source("clojure.core-test.qualified-keyword-qmark", include_str!("bundled_148.cljrs"));
    globals.register_builtin_source("clojure.core-test.qualified-symbol-qmark", include_str!("bundled_149.cljrs"));
    globals.register_builtin_source("clojure.core-test.quot", include_str!("bundled_150.cljrs"));
    globals.register_builtin_source("clojure.core-test.rand", include_str!("bundled_151.cljrs"));
    globals.register_builtin_source("clojure.core-test.rand-int", include_str!("bundled_152.cljrs"));
    globals.register_builtin_source("clojure.core-test.rand-nth", include_str!("bundled_153.cljrs"));
    globals.register_builtin_source("clojure.core-test.random-sample", include_str!("bundled_154.cljrs"));
    globals.register_builtin_source("clojure.core-test.random-uuid", include_str!("bundled_155.cljrs"));
    globals.register_builtin_source("clojure.core-test.ratio-qmark", include_str!("bundled_156.cljrs"));
    globals.register_builtin_source("clojure.core-test.rational-qmark", include_str!("bundled_157.cljrs"));
    globals.register_builtin_source("clojure.core-test.rationalize", include_str!("bundled_158.cljrs"));
    globals.register_builtin_source("clojure.core-test.realized-qmark", include_str!("bundled_159.cljrs"));
    globals.register_builtin_source("clojure.core-test.reduce", include_str!("bundled_160.cljrs"));
    globals.register_builtin_source("clojure.core-test.rem", include_str!("bundled_161.cljrs"));
    globals.register_builtin_source("clojure.core-test.remove-watch", include_str!("bundled_162.cljrs"));
    globals.register_builtin_source("clojure.core-test.repeat", include_str!("bundled_163.cljrs"));
    globals.register_builtin_source("clojure.core-test.rest", include_str!("bundled_164.cljrs"));
    globals.register_builtin_source("clojure.core-test.reverse", include_str!("bundled_165.cljrs"));
    globals.register_builtin_source("clojure.core-test.reversible-qmark", include_str!("bundled_166.cljrs"));
    globals.register_builtin_source("clojure.core-test.rseq", include_str!("bundled_167.cljrs"));
    globals.register_builtin_source("clojure.core-test.second", include_str!("bundled_168.cljrs"));
    globals.register_builtin_source("clojure.core-test.select-keys", include_str!("bundled_169.cljrs"));
    globals.register_builtin_source("clojure.core-test.seq", include_str!("bundled_170.cljrs"));
    globals.register_builtin_source("clojure.core-test.seq-qmark", include_str!("bundled_171.cljrs"));
    globals.register_builtin_source("clojure.core-test.seqable-qmark", include_str!("bundled_172.cljrs"));
    globals.register_builtin_source("clojure.core-test.sequential-qmark", include_str!("bundled_173.cljrs"));
    globals.register_builtin_source("clojure.core-test.set", include_str!("bundled_174.cljrs"));
    globals.register_builtin_source("clojure.core-test.set-qmark", include_str!("bundled_175.cljrs"));
    globals.register_builtin_source("clojure.core-test.short", include_str!("bundled_176.cljrs"));
    globals.register_builtin_source("clojure.core-test.shuffle", include_str!("bundled_177.cljrs"));
    globals.register_builtin_source("clojure.core-test.simple-ident-qmark", include_str!("bundled_178.cljrs"));
    globals.register_builtin_source("clojure.core-test.simple-keyword-qmark", include_str!("bundled_179.cljrs"));
    globals.register_builtin_source("clojure.core-test.simple-symbol-qmark", include_str!("bundled_180.cljrs"));
    globals.register_builtin_source("clojure.core-test.slash", include_str!("bundled_181.cljrs"));
    globals.register_builtin_source("clojure.core-test.some", include_str!("bundled_182.cljrs"));
    globals.register_builtin_source("clojure.core-test.some-fn", include_str!("bundled_183.cljrs"));
    globals.register_builtin_source("clojure.core-test.some-qmark", include_str!("bundled_184.cljrs"));
    globals.register_builtin_source("clojure.core-test.sort", include_str!("bundled_185.cljrs"));
    globals.register_builtin_source("clojure.core-test.sort-by", include_str!("bundled_186.cljrs"));
    globals.register_builtin_source("clojure.core-test.sorted-qmark", include_str!("bundled_187.cljrs"));
    globals.register_builtin_source("clojure.core-test.special-symbol-qmark", include_str!("bundled_188.cljrs"));
    globals.register_builtin_source("clojure.core-test.star", include_str!("bundled_189.cljrs"));
    globals.register_builtin_source("clojure.core-test.star-squote", include_str!("bundled_190.cljrs"));
    globals.register_builtin_source("clojure.core-test.str", include_str!("bundled_191.cljrs"));
    globals.register_builtin_source("clojure.core-test.string-qmark", include_str!("bundled_192.cljrs"));
    globals.register_builtin_source("clojure.core-test.subs", include_str!("bundled_193.cljrs"));
    globals.register_builtin_source("clojure.core-test.subvec", include_str!("bundled_194.cljrs"));
    globals.register_builtin_source("clojure.core-test.symbol", include_str!("bundled_195.cljrs"));
    globals.register_builtin_source("clojure.core-test.symbol-qmark", include_str!("bundled_196.cljrs"));
    globals.register_builtin_source("clojure.core-test.take", include_str!("bundled_197.cljrs"));
    globals.register_builtin_source("clojure.core-test.take-last", include_str!("bundled_198.cljrs"));
    globals.register_builtin_source("clojure.core-test.take-nth", include_str!("bundled_199.cljrs"));
    globals.register_builtin_source("clojure.core-test.take-while", include_str!("bundled_200.cljrs"));
    globals.register_builtin_source("clojure.core-test.taps", include_str!("bundled_201.cljrs"));
    globals.register_builtin_source("clojure.core-test.true-qmark", include_str!("bundled_202.cljrs"));
    globals.register_builtin_source("clojure.core-test.underive", include_str!("bundled_203.cljrs"));
    globals.register_builtin_source("clojure.core-test.unsigned-bit-shift-right", include_str!("bundled_204.cljrs"));
    globals.register_builtin_source("clojure.core-test.update", include_str!("bundled_205.cljrs"));
    globals.register_builtin_source("clojure.core-test.uuid-qmark", include_str!("bundled_206.cljrs"));
    globals.register_builtin_source("clojure.core-test.val", include_str!("bundled_207.cljrs"));
    globals.register_builtin_source("clojure.core-test.vals", include_str!("bundled_208.cljrs"));
    globals.register_builtin_source("clojure.core-test.var-qmark", include_str!("bundled_209.cljrs"));
    globals.register_builtin_source("clojure.core-test.vec", include_str!("bundled_210.cljrs"));
    globals.register_builtin_source("clojure.core-test.vector", include_str!("bundled_211.cljrs"));
    globals.register_builtin_source("clojure.core-test.vector-qmark", include_str!("bundled_212.cljrs"));
    globals.register_builtin_source("clojure.core-test.when", include_str!("bundled_213.cljrs"));
    globals.register_builtin_source("clojure.core-test.when-first", include_str!("bundled_214.cljrs"));
    globals.register_builtin_source("clojure.core-test.when-let", include_str!("bundled_215.cljrs"));
    globals.register_builtin_source("clojure.core-test.when-not", include_str!("bundled_216.cljrs"));
    globals.register_builtin_source("clojure.core-test.with-out-str", include_str!("bundled_217.cljrs"));
    globals.register_builtin_source("clojure.core-test.with-precision", include_str!("bundled_218.cljrs"));
    globals.register_builtin_source("clojure.core-test.zero-qmark", include_str!("bundled_219.cljrs"));
    globals.register_builtin_source("clojure.core-test.zipmap", include_str!("bundled_220.cljrs"));
    globals.register_builtin_source("clojure.data-test.diff", include_str!("bundled_221.cljrs"));
    globals.register_builtin_source("clojure.edn-test.read-string", include_str!("bundled_222.cljrs"));
    globals.register_builtin_source("clojure.string-test.blank-qmark", include_str!("bundled_223.cljrs"));
    globals.register_builtin_source("clojure.string-test.capitalize", include_str!("bundled_224.cljrs"));
    globals.register_builtin_source("clojure.string-test.ends-with-qmark", include_str!("bundled_225.cljrs"));
    globals.register_builtin_source("clojure.string-test.escape", include_str!("bundled_226.cljrs"));
    globals.register_builtin_source("clojure.string-test.lower-case", include_str!("bundled_227.cljrs"));
    globals.register_builtin_source("clojure.string-test.reverse", include_str!("bundled_228.cljrs"));
    globals.register_builtin_source("clojure.string-test.starts-with-qmark", include_str!("bundled_229.cljrs"));
    globals.register_builtin_source("clojure.string-test.upper-case", include_str!("bundled_230.cljrs"));
    globals.register_builtin_source("clojure.walk-test.walk", include_str!("bundled_231.cljrs"));
    globals.register_builtin_source("clojure.zip-test.zip", include_str!("bundled_232.cljrs"));
    let mut env = cljrs_eval::Env::new(globals, "user");

    // Push an eval context so rt_call can dispatch through the interpreter.
    cljrs_eval::callback::push_eval_context(&env);

    // Load clojure.test if not already loaded
    let _ = cljrs_eval::eval(
        &cljrs_reader::Parser::new(
            "(require 'clojure.test)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0],
        &mut env
    );

    // Load all test namespaces
    (|| {
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.abs)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.aclone)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.add-watch)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.ancestors)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.and)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.any-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.assoc)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.assoc-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.associative-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.atom)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bigdec)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bigint)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.binding)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-and)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-and-not)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-clear)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-flip)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-not)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-or)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-set)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-shift-left)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-shift-right)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-test)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bit-xor)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.boolean)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.boolean-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bound-fn)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.bound-fn-star)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.butlast)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.byte)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.case)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.char)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.char-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.coll-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.comment)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.compare)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.conj)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.conj-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.cons)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.constantly)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.contains-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.count)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.counted-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.cycle)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.dec)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.decimal-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.denominator)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.derive)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.descendants)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.disj)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.disj-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.dissoc)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.dissoc-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.doseq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.double)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.double-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.drop)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.drop-last)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.drop-while)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.empty)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.empty-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.eq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.even-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.false-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.ffirst)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.find)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.first)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.float)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.float-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.fn-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.fnext)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.fnil)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.format)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.get)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.get-in)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.gt)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.hash-map)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.hash-set)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.ident-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.identical-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.ifn-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.inc)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.int)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.int-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.integer-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.interleave)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.intern)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.interpose)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.juxt)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.key)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.keys)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.keyword)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.keyword-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.last)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.list-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.long)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.lt)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.lt-eq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.make-hierarchy)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.map-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.mapcat)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.max)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.merge)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.min)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.min-key)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.minus)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.mod)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.name)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.namespace)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nan-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.neg-int-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.neg-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.next)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nfirst)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nil-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nnext)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.not)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.not-empty)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.not-eq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nth)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nthnext)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.nthrest)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.num)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.number-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.number-range)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.numerator)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.odd-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.or)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.parents)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.parse-boolean)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.parse-double)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.parse-long)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.parse-uuid)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.partial)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.peek)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.persistent-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.plus)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.plus-squote)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.pop)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.pop-bang)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.portability)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.pos-int-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.pos-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.pr-str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.print-str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.println-str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.prn-str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.qualified-ident-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.qualified-keyword-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.qualified-symbol-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.quot)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rand)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rand-int)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rand-nth)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.random-sample)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.random-uuid)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.ratio-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rational-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rationalize)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.realized-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.reduce)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rem)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.remove-watch)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.repeat)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rest)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.reverse)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.reversible-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.rseq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.second)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.select-keys)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.seq)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.seq-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.seqable-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.sequential-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.set)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.set-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.short)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.shuffle)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.simple-ident-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.simple-keyword-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.simple-symbol-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.slash)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.some)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.some-fn)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.some-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.sort)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.sort-by)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.sorted-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.special-symbol-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.star)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.star-squote)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.string-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.subs)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.subvec)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.symbol)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.symbol-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.take)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.take-last)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.take-nth)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.take-while)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.taps)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.true-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.underive)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.unsigned-bit-shift-right)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.update)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.uuid-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.val)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.vals)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.var-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.vec)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.vector)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.vector-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.when)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.when-first)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.when-let)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.when-not)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.with-out-str)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.with-precision)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.zero-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.core-test.zipmap)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.data-test.diff)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.edn-test.read-string)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.blank-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.capitalize)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.ends-with-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.escape)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.lower-case)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.reverse)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.starts-with-qmark)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.string-test.upper-case)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.walk-test.walk)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
        let _ = cljrs_eval::eval(&cljrs_reader::Parser::new(
            "(require 'clojure.zip-test.zip)".to_string(),
            "<test-harness>".to_string()
        ).parse_all().unwrap()[0], &mut env);
    })();

    // Run tests for each namespace separately
    let mut total_pass = 0i64;
    let mut total_fail = 0i64;
    let mut total_error = 0i64;
    let mut total_test_count = 0i64;

    for ns_str in vec![
        "clojure.core-test.abs".to_string(),
        "clojure.core-test.aclone".to_string(),
        "clojure.core-test.add-watch".to_string(),
        "clojure.core-test.ancestors".to_string(),
        "clojure.core-test.and".to_string(),
        "clojure.core-test.any-qmark".to_string(),
        "clojure.core-test.assoc".to_string(),
        "clojure.core-test.assoc-bang".to_string(),
        "clojure.core-test.associative-qmark".to_string(),
        "clojure.core-test.atom".to_string(),
        "clojure.core-test.bigdec".to_string(),
        "clojure.core-test.bigint".to_string(),
        "clojure.core-test.binding".to_string(),
        "clojure.core-test.bit-and".to_string(),
        "clojure.core-test.bit-and-not".to_string(),
        "clojure.core-test.bit-clear".to_string(),
        "clojure.core-test.bit-flip".to_string(),
        "clojure.core-test.bit-not".to_string(),
        "clojure.core-test.bit-or".to_string(),
        "clojure.core-test.bit-set".to_string(),
        "clojure.core-test.bit-shift-left".to_string(),
        "clojure.core-test.bit-shift-right".to_string(),
        "clojure.core-test.bit-test".to_string(),
        "clojure.core-test.bit-xor".to_string(),
        "clojure.core-test.boolean".to_string(),
        "clojure.core-test.boolean-qmark".to_string(),
        "clojure.core-test.bound-fn".to_string(),
        "clojure.core-test.bound-fn-star".to_string(),
        "clojure.core-test.butlast".to_string(),
        "clojure.core-test.byte".to_string(),
        "clojure.core-test.case".to_string(),
        "clojure.core-test.char".to_string(),
        "clojure.core-test.char-qmark".to_string(),
        "clojure.core-test.coll-qmark".to_string(),
        "clojure.core-test.comment".to_string(),
        "clojure.core-test.compare".to_string(),
        "clojure.core-test.conj".to_string(),
        "clojure.core-test.conj-bang".to_string(),
        "clojure.core-test.cons".to_string(),
        "clojure.core-test.constantly".to_string(),
        "clojure.core-test.contains-qmark".to_string(),
        "clojure.core-test.count".to_string(),
        "clojure.core-test.counted-qmark".to_string(),
        "clojure.core-test.cycle".to_string(),
        "clojure.core-test.dec".to_string(),
        "clojure.core-test.decimal-qmark".to_string(),
        "clojure.core-test.denominator".to_string(),
        "clojure.core-test.derive".to_string(),
        "clojure.core-test.descendants".to_string(),
        "clojure.core-test.disj".to_string(),
        "clojure.core-test.disj-bang".to_string(),
        "clojure.core-test.dissoc".to_string(),
        "clojure.core-test.dissoc-bang".to_string(),
        "clojure.core-test.doseq".to_string(),
        "clojure.core-test.double".to_string(),
        "clojure.core-test.double-qmark".to_string(),
        "clojure.core-test.drop".to_string(),
        "clojure.core-test.drop-last".to_string(),
        "clojure.core-test.drop-while".to_string(),
        "clojure.core-test.empty".to_string(),
        "clojure.core-test.empty-qmark".to_string(),
        "clojure.core-test.eq".to_string(),
        "clojure.core-test.even-qmark".to_string(),
        "clojure.core-test.false-qmark".to_string(),
        "clojure.core-test.ffirst".to_string(),
        "clojure.core-test.find".to_string(),
        "clojure.core-test.first".to_string(),
        "clojure.core-test.float".to_string(),
        "clojure.core-test.float-qmark".to_string(),
        "clojure.core-test.fn-qmark".to_string(),
        "clojure.core-test.fnext".to_string(),
        "clojure.core-test.fnil".to_string(),
        "clojure.core-test.format".to_string(),
        "clojure.core-test.get".to_string(),
        "clojure.core-test.get-in".to_string(),
        "clojure.core-test.gt".to_string(),
        "clojure.core-test.hash-map".to_string(),
        "clojure.core-test.hash-set".to_string(),
        "clojure.core-test.ident-qmark".to_string(),
        "clojure.core-test.identical-qmark".to_string(),
        "clojure.core-test.ifn-qmark".to_string(),
        "clojure.core-test.inc".to_string(),
        "clojure.core-test.int".to_string(),
        "clojure.core-test.int-qmark".to_string(),
        "clojure.core-test.integer-qmark".to_string(),
        "clojure.core-test.interleave".to_string(),
        "clojure.core-test.intern".to_string(),
        "clojure.core-test.interpose".to_string(),
        "clojure.core-test.juxt".to_string(),
        "clojure.core-test.key".to_string(),
        "clojure.core-test.keys".to_string(),
        "clojure.core-test.keyword".to_string(),
        "clojure.core-test.keyword-qmark".to_string(),
        "clojure.core-test.last".to_string(),
        "clojure.core-test.list-qmark".to_string(),
        "clojure.core-test.long".to_string(),
        "clojure.core-test.lt".to_string(),
        "clojure.core-test.lt-eq".to_string(),
        "clojure.core-test.make-hierarchy".to_string(),
        "clojure.core-test.map-qmark".to_string(),
        "clojure.core-test.mapcat".to_string(),
        "clojure.core-test.max".to_string(),
        "clojure.core-test.merge".to_string(),
        "clojure.core-test.min".to_string(),
        "clojure.core-test.min-key".to_string(),
        "clojure.core-test.minus".to_string(),
        "clojure.core-test.mod".to_string(),
        "clojure.core-test.name".to_string(),
        "clojure.core-test.namespace".to_string(),
        "clojure.core-test.nan-qmark".to_string(),
        "clojure.core-test.neg-int-qmark".to_string(),
        "clojure.core-test.neg-qmark".to_string(),
        "clojure.core-test.next".to_string(),
        "clojure.core-test.nfirst".to_string(),
        "clojure.core-test.nil-qmark".to_string(),
        "clojure.core-test.nnext".to_string(),
        "clojure.core-test.not".to_string(),
        "clojure.core-test.not-empty".to_string(),
        "clojure.core-test.not-eq".to_string(),
        "clojure.core-test.nth".to_string(),
        "clojure.core-test.nthnext".to_string(),
        "clojure.core-test.nthrest".to_string(),
        "clojure.core-test.num".to_string(),
        "clojure.core-test.number-qmark".to_string(),
        "clojure.core-test.number-range".to_string(),
        "clojure.core-test.numerator".to_string(),
        "clojure.core-test.odd-qmark".to_string(),
        "clojure.core-test.or".to_string(),
        "clojure.core-test.parents".to_string(),
        "clojure.core-test.parse-boolean".to_string(),
        "clojure.core-test.parse-double".to_string(),
        "clojure.core-test.parse-long".to_string(),
        "clojure.core-test.parse-uuid".to_string(),
        "clojure.core-test.partial".to_string(),
        "clojure.core-test.peek".to_string(),
        "clojure.core-test.persistent-bang".to_string(),
        "clojure.core-test.plus".to_string(),
        "clojure.core-test.plus-squote".to_string(),
        "clojure.core-test.pop".to_string(),
        "clojure.core-test.pop-bang".to_string(),
        "clojure.core-test.portability".to_string(),
        "clojure.core-test.pos-int-qmark".to_string(),
        "clojure.core-test.pos-qmark".to_string(),
        "clojure.core-test.pr-str".to_string(),
        "clojure.core-test.print-str".to_string(),
        "clojure.core-test.println-str".to_string(),
        "clojure.core-test.prn-str".to_string(),
        "clojure.core-test.qualified-ident-qmark".to_string(),
        "clojure.core-test.qualified-keyword-qmark".to_string(),
        "clojure.core-test.qualified-symbol-qmark".to_string(),
        "clojure.core-test.quot".to_string(),
        "clojure.core-test.rand".to_string(),
        "clojure.core-test.rand-int".to_string(),
        "clojure.core-test.rand-nth".to_string(),
        "clojure.core-test.random-sample".to_string(),
        "clojure.core-test.random-uuid".to_string(),
        "clojure.core-test.ratio-qmark".to_string(),
        "clojure.core-test.rational-qmark".to_string(),
        "clojure.core-test.rationalize".to_string(),
        "clojure.core-test.realized-qmark".to_string(),
        "clojure.core-test.reduce".to_string(),
        "clojure.core-test.rem".to_string(),
        "clojure.core-test.remove-watch".to_string(),
        "clojure.core-test.repeat".to_string(),
        "clojure.core-test.rest".to_string(),
        "clojure.core-test.reverse".to_string(),
        "clojure.core-test.reversible-qmark".to_string(),
        "clojure.core-test.rseq".to_string(),
        "clojure.core-test.second".to_string(),
        "clojure.core-test.select-keys".to_string(),
        "clojure.core-test.seq".to_string(),
        "clojure.core-test.seq-qmark".to_string(),
        "clojure.core-test.seqable-qmark".to_string(),
        "clojure.core-test.sequential-qmark".to_string(),
        "clojure.core-test.set".to_string(),
        "clojure.core-test.set-qmark".to_string(),
        "clojure.core-test.short".to_string(),
        "clojure.core-test.shuffle".to_string(),
        "clojure.core-test.simple-ident-qmark".to_string(),
        "clojure.core-test.simple-keyword-qmark".to_string(),
        "clojure.core-test.simple-symbol-qmark".to_string(),
        "clojure.core-test.slash".to_string(),
        "clojure.core-test.some".to_string(),
        "clojure.core-test.some-fn".to_string(),
        "clojure.core-test.some-qmark".to_string(),
        "clojure.core-test.sort".to_string(),
        "clojure.core-test.sort-by".to_string(),
        "clojure.core-test.sorted-qmark".to_string(),
        "clojure.core-test.special-symbol-qmark".to_string(),
        "clojure.core-test.star".to_string(),
        "clojure.core-test.star-squote".to_string(),
        "clojure.core-test.str".to_string(),
        "clojure.core-test.string-qmark".to_string(),
        "clojure.core-test.subs".to_string(),
        "clojure.core-test.subvec".to_string(),
        "clojure.core-test.symbol".to_string(),
        "clojure.core-test.symbol-qmark".to_string(),
        "clojure.core-test.take".to_string(),
        "clojure.core-test.take-last".to_string(),
        "clojure.core-test.take-nth".to_string(),
        "clojure.core-test.take-while".to_string(),
        "clojure.core-test.taps".to_string(),
        "clojure.core-test.true-qmark".to_string(),
        "clojure.core-test.underive".to_string(),
        "clojure.core-test.unsigned-bit-shift-right".to_string(),
        "clojure.core-test.update".to_string(),
        "clojure.core-test.uuid-qmark".to_string(),
        "clojure.core-test.val".to_string(),
        "clojure.core-test.vals".to_string(),
        "clojure.core-test.var-qmark".to_string(),
        "clojure.core-test.vec".to_string(),
        "clojure.core-test.vector".to_string(),
        "clojure.core-test.vector-qmark".to_string(),
        "clojure.core-test.when".to_string(),
        "clojure.core-test.when-first".to_string(),
        "clojure.core-test.when-let".to_string(),
        "clojure.core-test.when-not".to_string(),
        "clojure.core-test.with-out-str".to_string(),
        "clojure.core-test.with-precision".to_string(),
        "clojure.core-test.zero-qmark".to_string(),
        "clojure.core-test.zipmap".to_string(),
        "clojure.data-test.diff".to_string(),
        "clojure.edn-test.read-string".to_string(),
        "clojure.string-test.blank-qmark".to_string(),
        "clojure.string-test.capitalize".to_string(),
        "clojure.string-test.ends-with-qmark".to_string(),
        "clojure.string-test.escape".to_string(),
        "clojure.string-test.lower-case".to_string(),
        "clojure.string-test.reverse".to_string(),
        "clojure.string-test.starts-with-qmark".to_string(),
        "clojure.string-test.upper-case".to_string(),
        "clojure.walk-test.walk".to_string(),
        "clojure.zip-test.zip".to_string(),
    ].iter() {
        let run_result = cljrs_eval::eval(
            &cljrs_reader::Parser::new(
                format!("(clojure.test/run-tests '{})", ns_str)
                    .to_string(),
                "<run-tests>".to_string()
            ).parse_all().unwrap()[0],
            &mut env
        );
        if let Ok(Value::Map(m)) = run_result {
            let mut pass = 0i64;
            let mut fail = 0i64;
            let mut error = 0i64;
            let mut test_count = 0i64;
            m.for_each(|k, v| {
                if let (Value::Keyword(kw), Value::Long(count)) = (k, v) {
                    match kw.get().name.as_ref() {
                        "pass" => pass = *count,
                        "fail" => fail = *count,
                        "error" => error = *count,
                        "test" => test_count = *count,
                        _ => {}
                    }
                }
            });
            total_pass += pass;
            total_fail += fail;
            total_error += error;
            total_test_count += test_count;
        }
    }

    // Flush output before exiting
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("Ran {} tests containing {} assertions.", total_test_count, total_pass + total_fail + total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    println!("{} passed, {} failed, {} errors.", total_pass, total_fail, total_error);
    std::io::Write::flush(&mut std::io::stdout()).unwrap();
    if total_fail > 0 || total_error > 0 {
        std::process::exit(1);
    }

    // Pop the eval context.
    cljrs_eval::callback::pop_eval_context();
}