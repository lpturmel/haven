//! Performance budgets. Run with: cargo test --release -p haven-core --test perf -- --ignored --nocapture

use std::time::Instant;

use haven_core::sort::{SortKey, SortSpec, sort_entries};

#[test]
#[ignore = "perf budget; run in release"]
fn list_and_sort_large_directory() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20_000 {
        std::fs::write(dir.path().join(format!("file_{i}.txt")), b"x").unwrap();
    }
    let t = Instant::now();
    let mut entries = haven_core::list_dir(dir.path(), false).unwrap();
    let list = t.elapsed();
    let t = Instant::now();
    sort_entries(&mut entries, &SortSpec::default());
    let name_sort = t.elapsed();
    let spec = SortSpec {
        keys: vec![(SortKey::Modified, true), (SortKey::Name, false)],
        folders_first: true,
    };
    let t = Instant::now();
    sort_entries(&mut entries, &spec);
    let multi = t.elapsed();
    println!("20k entries: list {list:?}, name sort {name_sort:?}, multi-key sort {multi:?}");
    assert_eq!(entries.len(), 20_000);
    assert!(list.as_millis() < 250, "listing took {list:?}");
    assert!(name_sort.as_millis() < 30 && multi.as_millis() < 30);
}
