//! Helpers for the tests that need an S3-compatible server (MinIO in Docker
//! during development): a throwaway bucket per test, deterministic payloads
//! that can be checked at any offset without keeping them in memory, and a
//! bare HTTP GET for presigned URLs.
#![allow(dead_code)]

use aws_sdk_s3::Client;
use aws_sdk_s3::operation::head_object::HeadObjectOutput;
use aws_sdk_s3::types::{ChecksumMode, Delete, ObjectIdentifier};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use modlight_server::storage::ByteStream;
use modlight_server::storage::s3::{S3Config, S3Driver};

/// The connection settings from `MODLIGHT_TEST_S3_*`, or `None` (skip).
pub fn config_from_env(bucket: &str, prefix: Option<&str>) -> Option<S3Config> {
    let endpoint = std::env::var("MODLIGHT_TEST_S3_ENDPOINT").ok().filter(|v| !v.is_empty())?;
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} must be set with MODLIGHT_TEST_S3_ENDPOINT"));
    Some(S3Config {
        endpoint: Some(endpoint),
        region: std::env::var("MODLIGHT_TEST_S3_REGION").ok().filter(|v| !v.is_empty()).unwrap_or("auto".into()),
        bucket: bucket.to_string(),
        access_key_id: var("MODLIGHT_TEST_S3_ACCESS_KEY_ID"),
        secret_access_key: var("MODLIGHT_TEST_S3_SECRET_ACCESS_KEY"),
        force_path_style: std::env::var("MODLIGHT_TEST_S3_FORCE_PATH_STYLE").as_deref() != Ok("false"),
        prefix: prefix.map(str::to_string),
    })
}

pub fn unique_bucket_name() -> String {
    format!("modlight-test-{}-{:08x}", std::process::id(), rand::random::<u32>())
}

pub struct TestBucket {
    pub driver: S3Driver,
    pub config: S3Config,
    pub client: Client,
}

impl TestBucket {
    /// A new, empty bucket, or `None` when no S3 server is configured.
    pub async fn create(prefix: Option<&str>) -> Option<TestBucket> {
        let config = config_from_env(&unique_bucket_name(), prefix)?;
        let driver = S3Driver::new(config.clone());
        let client = driver.client().clone();
        client.create_bucket().bucket(&config.bucket).send().await.expect("create the test bucket");
        Some(TestBucket { driver, config, client })
    }

    /// Every key in the bucket (raw, prefix included), sorted.
    pub async fn raw_keys(&self) -> Vec<String> {
        list_keys(&self.client, &self.config.bucket).await
    }

    /// HeadObject on a storage key (the driver's prefix applied), with
    /// checksums requested.
    pub async fn head(&self, key: &str) -> HeadObjectOutput {
        self.head_raw(&self.driver.key(key)).await
    }

    pub async fn head_raw(&self, raw_key: &str) -> HeadObjectOutput {
        self.client
            .head_object()
            .bucket(&self.config.bucket)
            .key(raw_key)
            .checksum_mode(ChecksumMode::Enabled)
            .send()
            .await
            .unwrap_or_else(|e| panic!("HeadObject {raw_key}: {e:?}"))
    }

    /// (key, upload id) of every multipart upload still open in the bucket.
    pub async fn pending_multipart_uploads(&self) -> Vec<(String, String)> {
        let out = self
            .client
            .list_multipart_uploads()
            .bucket(&self.config.bucket)
            .send()
            .await
            .expect("ListMultipartUploads");
        out.uploads()
            .iter()
            .map(|u| (u.key().unwrap_or_default().to_string(), u.upload_id().unwrap_or_default().to_string()))
            .collect()
    }

    /// Empty and delete the bucket.
    pub async fn destroy(self) {
        empty_bucket(&self.client, &self.config.bucket).await;
        if let Err(e) = self.client.delete_bucket().bucket(&self.config.bucket).send().await {
            eprintln!("could not delete test bucket {}: {e:?}", self.config.bucket);
        }
    }
}

pub async fn list_keys(client: &Client, bucket: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut token = None;
    loop {
        let out =
            client.list_objects_v2().bucket(bucket).set_continuation_token(token).send().await.expect("ListObjectsV2");
        keys.extend(out.contents().iter().filter_map(|o| o.key().map(str::to_string)));
        token = out.next_continuation_token().map(str::to_string);
        if out.is_truncated() != Some(true) || token.is_none() {
            break;
        }
    }
    keys.sort();
    keys
}

pub async fn empty_bucket(client: &Client, bucket: &str) {
    if let Ok(out) = client.list_multipart_uploads().bucket(bucket).send().await {
        for u in out.uploads() {
            let _ = client
                .abort_multipart_upload()
                .bucket(bucket)
                .key(u.key().unwrap_or_default())
                .upload_id(u.upload_id().unwrap_or_default())
                .send()
                .await;
        }
    }
    let keys = list_keys(client, bucket).await;
    for chunk in keys.chunks(1000) {
        let objects = chunk.iter().map(|k| ObjectIdentifier::builder().key(k).build().unwrap()).collect::<Vec<_>>();
        let delete = Delete::builder().set_objects(Some(objects)).quiet(true).build().unwrap();
        client.delete_objects().bucket(bucket).delete(delete).send().await.expect("DeleteObjects");
    }
}

/// The byte at absolute offset `i` of every pattern payload.
pub fn byte_at(i: u64) -> u8 {
    (i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8
}

/// `total` pattern bytes starting at offset `start`, in chunks of `chunk`.
pub fn pattern_stream_from(start: u64, total: u64, chunk: usize) -> ByteStream {
    let end = start + total;
    Box::pin(futures_util::stream::unfold(start, move |pos| async move {
        if pos >= end {
            return None;
        }
        let n = (end - pos).min(chunk as u64);
        let data: Vec<u8> = (pos..pos + n).map(byte_at).collect();
        Some((Ok(Bytes::from(data)), pos + n))
    }))
}

pub fn pattern_stream(total: u64, chunk: usize) -> ByteStream {
    pattern_stream_from(0, total, chunk)
}

/// Read `stream` and check it is exactly `total` pattern bytes from `start`.
pub async fn check_pattern(mut stream: ByteStream, start: u64, total: u64) {
    let mut pos = start;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("read");
        for (i, v) in chunk.iter().enumerate() {
            let at = pos + i as u64;
            assert_eq!(*v, byte_at(at), "byte {at} differs");
        }
        pos += chunk.len() as u64;
    }
    assert_eq!(pos - start, total, "length");
}

pub async fn collect(mut stream: ByteStream) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(chunk) = stream.next().await {
        out.extend_from_slice(&chunk.expect("read"));
    }
    out
}

/// A plain HTTP/1.1 GET of `url` exactly as written (no re-encoding).
pub async fn http_get(url: &str, range: Option<&str>) -> (u16, Vec<u8>) {
    let after_scheme = url.find("://").expect("absolute URL") + 3;
    let path_start = url[after_scheme..].find('/').map(|i| after_scheme + i).expect("path");
    let authority = &url[after_scheme..path_start];
    let stream = tokio::net::TcpStream::connect(authority).await.expect("connect");
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream)).await.expect("handshake");
    tokio::spawn(conn);
    let mut req = http::Request::get(&url[path_start..]).header("host", authority);
    if let Some(range) = range {
        req = req.header("range", range);
    }
    let res = sender.send_request(req.body(http_body_util::Empty::<Bytes>::new()).unwrap()).await.expect("request");
    let status = res.status().as_u16();
    let body = res.into_body().collect().await.expect("body").to_bytes().to_vec();
    (status, body)
}
