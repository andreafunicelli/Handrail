//! Round trip against a real S3-compatible server. Runs when `HANDRAIL_S3_TEST` names an
//! endpoint (CI starts MinIO): `HANDRAIL_S3_TEST=http://127.0.0.1:9000`, with
//! `HANDRAIL_S3_KEY`, `HANDRAIL_S3_SECRET` and the existing bucket `HANDRAIL_S3_BUCKET`.

use aht_core::Error;
use aht_s3::{S3Client, S3Config};
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn round_trip_against_real_s3() {
    let Ok(endpoint) = std::env::var("HANDRAIL_S3_TEST") else {
        eprintln!("HANDRAIL_S3_TEST not set; skipping");
        return;
    };
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} must be set"));
    let bucket = var("HANDRAIL_S3_BUCKET");
    let c = S3Client::new(S3Config {
        endpoint,
        region: "us-east-1".into(),
        access_key_id: var("HANDRAIL_S3_KEY"),
        secret_access_key: var("HANDRAIL_S3_SECRET"),
        path_style: true,
    })
    .unwrap();

    assert!(c
        .list_buckets()
        .await
        .unwrap()
        .iter()
        .any(|b| b.name == bucket));
    let base = format!("handrail-test-{}/", std::process::id());
    c.create_folder(&bucket, &base).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("up.bin");
    let data: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &data).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let key = format!("{base}a b+c.bin");
    let sent = c
        .upload(&bucket, &key, &src, move |d, _| s.lock().unwrap().push(d))
        .await
        .unwrap();
    assert_eq!(sent, data.len() as u64);
    assert_eq!(seen.lock().unwrap().last(), Some(&(data.len() as u64)));

    let listing = c.list(&bucket, &base).await.unwrap();
    assert_eq!(listing.objects.len(), 1);
    assert_eq!(listing.objects[0].name, "a b+c.bin");
    assert_eq!(listing.objects[0].size, data.len() as u64);
    let root = c.list(&bucket, "").await.unwrap();
    assert!(root
        .folders
        .contains(&base.trim_end_matches('/').to_string()));

    let moved = format!("{base}sub/moved.bin");
    c.copy(&bucket, &key, &moved).await.unwrap();
    c.delete(&bucket, &key).await.unwrap();
    let back = dir.path().join("down.bin");
    c.download(&bucket, &moved, &back, |_, _| {}).await.unwrap();
    assert_eq!(std::fs::read(&back).unwrap(), data);

    let missing = dir.path().join("missing.bin");
    let err = c
        .download(&bucket, &format!("{base}nope"), &missing, |_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(!missing.exists());

    assert!(c.delete_folder(&bucket, &base).await.is_err(), "not empty");
    c.delete(&bucket, &moved).await.unwrap();
    c.delete_folder(&bucket, &base).await.unwrap();
    assert!(c.list(&bucket, &base).await.unwrap().objects.is_empty());
}
