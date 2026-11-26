//! Run with: cargo test --release -p haven-highlight --test perf -- --ignored --nocapture

use std::time::Instant;

#[test]
#[ignore = "perf budget; run in release"]
fn highlights_200kb_rust_quickly() {
    let unit = include_str!("../src/lib.rs");
    let mut src = String::new();
    while src.len() < 200_000 {
        src.push_str(unit);
    }
    // Warm the grammar/query compile (one-time cost per language).
    haven_highlight::highlight(haven_highlight::Lang::Rust, "fn a() {}", None).unwrap();
    let t = Instant::now();
    let lines = haven_highlight::highlight(haven_highlight::Lang::Rust, &src, None).unwrap();
    let took = t.elapsed();
    println!(
        "{} bytes / {} lines highlighted in {took:?}",
        src.len(),
        lines.len()
    );
    assert!(took.as_millis() < 120, "{took:?}");
}
