//! Smoke test: runs the real `--version` / status probes of whatever is installed locally.
//! Only asserts structure, so it passes on machines without any harness.

#[tokio::test]
async fn detect_all_returns_three_entries() {
    let all = aht_harness::detect_all().await;
    assert_eq!(all.len(), 3);
    for i in &all {
        if i.path.is_none() {
            assert!(i.version.is_none() && i.logged_in.is_none());
        }
        eprintln!("{i:?}");
    }
}
