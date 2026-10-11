//! The client against a fake S3 endpoint: addressing, signing headers, pagination, errors.

use aht_core::Error;
use aht_s3::{S3Client, S3Config};
use wiremock::matchers::{header_exists, method, path, query_param, query_param_is_missing};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> S3Client {
    S3Client::new(S3Config {
        endpoint: server.uri(),
        region: "us-east-1".into(),
        access_key_id: "AKIDEXAMPLE".into(),
        secret_access_key: "secret".into(),
        path_style: true,
    })
    .unwrap()
}

const PAGE1: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name><Prefix>logs/</Prefix>
<IsTruncated>true</IsTruncated><NextContinuationToken>t+1</NextContinuationToken>
<Contents><Key>logs/</Key><Size>0</Size></Contents>
<Contents><Key>logs/b &amp; c.txt</Key><LastModified>2013-05-24T00:00:00.000Z</LastModified><Size>42</Size></Contents>
<CommonPrefixes><Prefix>logs/2024/</Prefix></CommonPrefixes></ListBucketResult>"#;

const PAGE2: &str = r#"<ListBucketResult><IsTruncated>false</IsTruncated>
<Contents><Key>logs/a.txt</Key><Size>7</Size></Contents>
<CommonPrefixes><Prefix>logs/2023/</Prefix></CommonPrefixes></ListBucketResult>"#;

#[tokio::test]
async fn lists_every_page_path_style() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .and(query_param("list-type", "2"))
        .and(query_param("prefix", "logs/"))
        .and(query_param_is_missing("continuation-token"))
        .and(header_exists("authorization"))
        .and(header_exists("x-amz-date"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PAGE1))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .and(query_param("continuation-token", "t+1"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PAGE2))
        .expect(1)
        .mount(&server)
        .await;
    let l = client(&server).list("b", "logs/").await.unwrap();
    assert_eq!(l.folders, ["2023", "2024"]);
    let names: Vec<_> = l.objects.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, ["a.txt", "b & c.txt"]);
    assert_eq!(l.objects[1].key, "logs/b & c.txt");
    assert_eq!(l.objects[1].size, 42);
    assert_eq!(l.objects[1].modified, Some(1_369_353_600));
}

#[tokio::test]
async fn maps_errors_and_refuses_non_empty_folders() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/b/missing.txt"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            "<Error><Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>",
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b"))
        .and(query_param("prefix", "full/"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<ListBucketResult><Contents><Key>full/</Key></Contents><Contents><Key>full/x</Key></Contents></ListBucketResult>",
        ))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    let c = client(&server);
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("missing.txt");
    let err = c
        .download("b", "missing.txt", &local, |_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(!local.exists(), "no file is left behind");
    let err = c.delete_folder("b", "full/").await.unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
}

#[tokio::test]
async fn copy_errors_inside_a_200_are_errors() {
    let server = MockServer::start().await;
    Mock::given(method("PUT"))
        .and(path("/b/to.txt"))
        .and(header_exists("x-amz-copy-source"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<Error><Code>InternalError</Code><Message>We encountered an internal error.</Message></Error>",
        ))
        .mount(&server)
        .await;
    assert!(client(&server)
        .copy("b", "from.txt", "to.txt")
        .await
        .is_err());
}

#[tokio::test]
async fn exists_and_streams_without_files() {
    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/b/here.txt"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/b/gone.txt"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/b/here.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hello s3"))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/b/up.txt"))
        .and(header_exists("content-length"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    assert!(c.exists("b", "here.txt").await.unwrap());
    assert!(!c.exists("b", "gone.txt").await.unwrap());

    // Into any writer (a server's file over SFTP in the app).
    let mut sink = Vec::new();
    let mut last = (0, 0);
    let n = c
        .download_to("b", "here.txt", &mut sink, |d, t| last = (d, t))
        .await
        .unwrap();
    assert_eq!((n, sink.as_slice()), (8, b"hello s3".as_slice()));
    assert_eq!(last, (8, 8));

    // From any reader with a known size.
    let src = std::io::Cursor::new(b"from a server".to_vec());
    assert_eq!(
        c.upload_from("b", "up.txt", src, 13, |_, _| {})
            .await
            .unwrap(),
        13
    );
    let got = &server.received_requests().await.unwrap();
    let put = got.iter().find(|r| r.method.as_str() == "PUT").unwrap();
    assert_eq!(put.body, b"from a server");
}
