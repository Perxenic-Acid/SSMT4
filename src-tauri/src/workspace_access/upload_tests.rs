use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn mock_service(
    responses: Vec<(u16, String, &'static str)>,
) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body, headers) in responses {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut data = Vec::new();
            let request = loop {
                let mut chunk = [0_u8; 4096];
                let count = socket.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
                if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&data[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|length| length.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + length {
                        break header.lines().next().unwrap().to_string();
                    }
                }
            };
            requests.push(request);
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (url, task)
}

fn archive_info() -> ArchiveInfo {
    ArchiveInfo {
        path: std::env::temp_dir().join(format!(
            "ssmt-upload-protocol-{}-{}.ssmtws",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )),
        sha256: "a".repeat(64),
        size: 3,
        uncompressed_size: 3,
        file_count: 1,
    }
}

fn metadata(info: &ArchiveInfo) -> PublicMetadataV1 {
    make_public_metadata(
        &WorkspaceAccessPublishRequest {
            worker_url: String::new(),
            proxy_port: None,
            workspace_path: String::new(),
            game_preset: "GIMI".into(),
            workspace_name: "Protocol".into(),
            description: None,
            attribution: Default::default(),
            captured_at: None,
            game_build: None,
            supersedes: None,
            workspace_aliases: None,
            archive_path: None,
        },
        PortableWorkspaceMetadataV1 {
            schema_version: 1,
            game_preset: "GIMI".into(),
            workspace_name: "Protocol".into(),
            lods: Vec::new(),
        },
        Some(info),
    )
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

#[tokio::test]
async fn reusing_a_published_object_succeeds_without_reading_or_uploading_parts() {
    let (url, server) = mock_service(vec![(201, r#"{"submissionId":"reuse","entryId":"entry","status":"published","metadataPath":"games/GIMI/entry.json"}"#.into(), "")]).await;
    let info = archive_info();
    assert!(!info.path.exists());
    let mut progress = Vec::new();
    let result = publish_archive(&client(), &url, metadata(&info), info, |event| {
        progress.push(event)
    })
    .await
    .unwrap();
    assert_eq!(result.status, "published");
    assert_eq!(progress.len(), 1);
    assert_eq!(progress[0].completed_bytes, 3);
    assert_eq!(progress[0].total_bytes, 3);
    assert_eq!(server.await.unwrap(), ["POST /v1/submissions HTTP/1.1"]);
}

#[tokio::test]
async fn ordinary_parts_are_uploaded_and_completed_before_success() {
    // 分片地址在启动 mock 后才确定，使用独立 mock 存储端。
    let (storage, storage_task) =
        mock_service(vec![(200, String::new(), "ETag: \"part-etag\"\r\n")]).await;
    let parts = serde_json::json!({"parts":[{"partNumber":1,"url":format!("{storage}/part?signature=private")}]}).to_string();
    let (url, server) = mock_service(vec![
        (
            201,
            r#"{"submissionId":"new","entryId":"entry","status":"awaiting_upload"}"#.into(),
            "",
        ),
        (200, parts, ""),
        (
            200,
            r#"{"submissionId":"new","entryId":"entry","status":"published"}"#.into(),
            "",
        ),
    ])
    .await;
    let info = archive_info();
    let path = info.path.clone();
    fs::write(&path, [1, 2, 3]).unwrap();
    let mut progress = Vec::new();
    let result = publish_archive(&client(), &url, metadata(&info), info, |event| {
        progress.push(event)
    })
    .await;
    fs::remove_file(&path).unwrap();
    remove_upload_state(&path);
    assert_eq!(result.unwrap().status, "published");
    assert_eq!(
        progress
            .iter()
            .map(|value| value.completed_bytes)
            .collect::<Vec<_>>(),
        [0, 3]
    );
    assert_eq!(
        server.await.unwrap(),
        [
            "POST /v1/submissions HTTP/1.1",
            "POST /v1/submissions/new/parts HTTP/1.1",
            "POST /v1/submissions/new/complete HTTP/1.1"
        ]
    );
    assert_eq!(
        storage_task.await.unwrap(),
        ["PUT /part?signature=private HTTP/1.1"]
    );
}

#[tokio::test]
async fn final_validation_error_keeps_http_and_validation_reason() {
    let (url, server) = mock_service(vec![(
        422,
        r#"{"error":{"code":"ARCHIVE_INVALID","detail":"ARCHIVE_MANIFEST_INVALID"}}"#.into(),
        "",
    )])
    .await;
    let result = post_json::<SubmissionResponse, _>(
        &client(),
        &format!("{url}/complete"),
        &serde_json::json!({}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        result,
        "ARCHIVE_INVALID: HTTP 422; ARCHIVE_MANIFEST_INVALID"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn failed_parts_report_status_without_presigned_credentials() {
    let (url, server) = mock_service(vec![(403, String::new(), ""); UPLOAD_RETRIES]).await;
    let result = upload_part_with_retry(
        &client(),
        &format!("{url}/part?X-Amz-Signature=private"),
        vec![1],
        &AtomicBool::new(false),
    )
    .await
    .unwrap_err();
    assert!(result.contains("HTTP 403"));
    assert!(!result.contains("private"));
    assert!(!result.contains("X-Amz"));
    server.await.unwrap();
}
