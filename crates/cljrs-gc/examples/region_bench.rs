//! Region scope cost: create a region, allocate a few objects, drop it.
//!
//! Run with `cargo run --release -p cljrs-gc --example region_bench`.
//! Each region charges its chunk to the thread's isolate account and returns
//! it on drop, so this measures the governor's region accounting overhead.
//! `REGION_BENCH_ITERS` (default 2,000,000) and `REGION_BENCH_THREADS`
//! (comma-separated, default `1,8`) override the run size.

use std::time::Instant;

use cljrs_gc::region::Region;

fn iters() -> usize {
    std::env::var("REGION_BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000_000)
}

fn run() -> f64 {
    let iters = iters();
    let start = Instant::now();
    for i in 0..iters {
        let mut region = Region::new();
        for j in 0..4 {
            std::hint::black_box(region.alloc((i + j) as i64));
        }
        drop(region);
    }
    start.elapsed().as_nanos() as f64 / iters as f64
}

fn main() {
    let threads: Vec<usize> = match std::env::var("REGION_BENCH_THREADS") {
        Ok(v) => v.split(',').filter_map(|t| t.parse().ok()).collect(),
        Err(_) => vec![1, 8],
    };
    for threads in threads {
        let handles: Vec<_> = (0..threads).map(|_| std::thread::spawn(run)).collect();
        let ns: f64 = handles.into_iter().map(|h| h.join().unwrap()).sum::<f64>() / threads as f64;
        println!("{threads} thread(s): {ns:.1} ns per region scope");
    }
}
