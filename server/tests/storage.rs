//! Port of tests/integration/storage.test.ts. Upstream runs it against
//! `LocalDriver` only; here the same cases run against the local driver
//! always and against the S3 driver when an S3-compatible server is given,
//! plus what only S3 has (multipart, abort, presigned URLs, key prefix):
//!
//! ```text
//! MODLIGHT_TEST_S3_ENDPOINT=http://127.0.0.1:59425 \
//! MODLIGHT_TEST_S3_ACCESS_KEY_ID=... MODLIGHT_TEST_S3_SECRET_ACCESS_KEY=... \
//!   cargo test --test storage
//! ```
//!
//! Optional: `MODLIGHT_TEST_S3_REGION` (default `auto`, like upstream),
//! `MODLIGHT_TEST_S3_FORCE_PATH_STYLE` (default `true`). Every S3 test creates
//! its own bucket and deletes it afterwards.

mod s3_common;

use std::panic::AssertUnwindSafe;

use bytes::Bytes;
use futures_util::{FutureExt, StreamExt};
use modlight_server::storage::{ByteRange, ByteStream, LocalDriver, StorageDriver, is_not_found};
use rand::RngCore;
use s3_common::{TestBucket, byte_at, check_pattern, collect, http_get, pattern_stream};

fn once(data: Vec<u8>) -> ByteStream {
    Box::pin(futures_util::stream::once(async move { Ok(Bytes::from(data)) }))
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut v);
    v
}

// ── The upstream suite, for any driver ──────────────────────────────────────

async fn write_then_read_buffer_round_trips(driver: &dyn StorageDriver) {
    let key = "u1/round-trip.bin";
    let payload = random_bytes(4096);
    driver.write(key, once(payload.clone())).await.unwrap();
    let out = driver.read_buffer(key).await.unwrap();
    assert_eq!(out.as_ref(), payload.as_slice());
}

async fn stat_returns_the_correct_size(driver: &dyn StorageDriver) {
    let key = "u1/sized.bin";
    driver.write(key, once(random_bytes(1234))).await.unwrap();
    assert_eq!(driver.stat(key).await.unwrap().map(|s| s.size), Some(1234));
}

async fn stat_returns_null_for_a_missing_key(driver: &dyn StorageDriver) {
    assert_eq!(driver.stat("u1/does-not-exist.bin").await.unwrap(), None);
}

async fn exists_reports_present_and_absent_keys(driver: &dyn StorageDriver) {
    let key = "u2/present.bin";
    assert!(!driver.exists(key).await.unwrap());
    driver.write(key, once(b"hello".to_vec())).await.unwrap();
    assert!(driver.exists(key).await.unwrap());
    assert!(!driver.exists("u2/absent.bin").await.unwrap());
}

async fn read_stream_with_a_byte_range_returns_exactly_that_slice(driver: &dyn StorageDriver) {
    let key = "u3/ranged.bin";
    let payload = random_bytes(2048);
    driver.write(key, once(payload.clone())).await.unwrap();
    // range is inclusive on both ends (see ByteRange in types.ts).
    let got = collect(driver.read_stream(key, Some(ByteRange { start: 100, end: 299 })).await.unwrap()).await;
    assert_eq!(got, &payload[100..300]);
}

async fn read_stream_without_a_range_returns_the_whole_object(driver: &dyn StorageDriver) {
    let key = "u3/whole.bin";
    let payload = random_bytes(777);
    driver.write(key, once(payload.clone())).await.unwrap();
    assert_eq!(collect(driver.read_stream(key, None).await.unwrap()).await, payload);
}

async fn ingest_local_file_moves_a_temp_file_into_place(driver: &dyn StorageDriver) {
    // Stage a source file outside the storage root, so this exercises the
    // move and not an in-root no-op.
    let stage = tempfile::tempdir().unwrap();
    let src = stage.path().join("incoming.bin");
    let payload = random_bytes(8192);
    std::fs::write(&src, &payload).unwrap();
    let key = "u4/ingested.bin";
    driver.ingest_local_file(&src, key).await.unwrap();
    // Source must be gone (rename, copy + unlink, or upload + rm).
    assert_eq!(std::fs::metadata(&src).unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert_eq!(driver.read_buffer(key).await.unwrap().as_ref(), payload.as_slice());
    assert!(driver.exists(key).await.unwrap());
}

async fn delete_removes_the_object(driver: &dyn StorageDriver) {
    let key = "u5/to-delete.bin";
    driver.write(key, once(b"delete me".to_vec())).await.unwrap();
    assert!(driver.exists(key).await.unwrap());
    driver.delete(key).await.unwrap();
    assert!(!driver.exists(key).await.unwrap());
    assert_eq!(driver.stat(key).await.unwrap(), None);
}

async fn delete_is_a_no_op_for_a_missing_key(driver: &dyn StorageDriver) {
    driver.delete("u5/never-existed.bin").await.unwrap();
}

async fn delete_prefix_removes_everything_under_the_prefix_and_nothing_beside_it(driver: &dyn StorageDriver) {
    // This is how a file's cached conversions go away with it: one call, without
    // the caller having to remember which variants were ever produced.
    driver.write("derived/aaa/one.png", once(b"1".to_vec())).await.unwrap();
    driver.write("derived/aaa/two.pdf", once(b"2".to_vec())).await.unwrap();
    driver.write("derived/bbb/three.webp", once(b"3".to_vec())).await.unwrap();
    driver.write("u1/original.png", once(b"4".to_vec())).await.unwrap();
    driver.delete_prefix("derived/aaa/").await.unwrap();
    assert!(!driver.exists("derived/aaa/one.png").await.unwrap());
    assert!(!driver.exists("derived/aaa/two.pdf").await.unwrap());
    assert!(driver.exists("derived/bbb/three.webp").await.unwrap());
    assert!(driver.exists("u1/original.png").await.unwrap());
}

async fn delete_prefix_with_nothing_under_it_is_success(driver: &dyn StorageDriver) {
    driver.delete_prefix("derived/never-existed/").await.unwrap();
}

/// Every case of the upstream suite that is not about the filesystem.
async fn shared_suite(driver: &dyn StorageDriver) {
    write_then_read_buffer_round_trips(driver).await;
    stat_returns_the_correct_size(driver).await;
    stat_returns_null_for_a_missing_key(driver).await;
    exists_reports_present_and_absent_keys(driver).await;
    read_stream_with_a_byte_range_returns_exactly_that_slice(driver).await;
    read_stream_without_a_range_returns_the_whole_object(driver).await;
    ingest_local_file_moves_a_temp_file_into_place(driver).await;
    delete_removes_the_object(driver).await;
    delete_is_a_no_op_for_a_missing_key(driver).await;
    delete_prefix_removes_everything_under_the_prefix_and_nothing_beside_it(driver).await;
    delete_prefix_with_nothing_under_it_is_success(driver).await;
}

// ── LocalDriver ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn local_driver_upstream_suite() {
    let root = tempfile::tempdir().unwrap();
    let driver = LocalDriver::new(root.path().to_str().unwrap());
    shared_suite(&driver).await;

    // A path-traversal key throws and does not write outside the root.
    let err = driver.write("../escape.bin", once(b"nope".to_vec())).await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("traversal"), "{err}");
    assert!(!root.path().parent().unwrap().join("escape.bin").exists());
    // A deeper dot-dot escape also throws.
    let err = driver.read_buffer("a/b/../../../etc/passwd").await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("traversal"), "{err}");

    // deletePrefix refuses to empty the storage root...
    for prefix in ["/", ""] {
        let err = driver.delete_prefix(prefix).await.unwrap_err();
        assert!(err.to_string().contains("storage root"), "{err}");
    }
    // ...and cannot be talked into deleting outside it.
    for prefix in ["../", "derived/../../etc/"] {
        let err = driver.delete_prefix(prefix).await.unwrap_err();
        assert!(err.to_string().to_lowercase().contains("traversal"), "{err}");
    }

    // A missing object is "already gone" for the media/trash deletions.
    assert!(is_not_found(&driver.read_buffer("u1/missing.bin").await.unwrap_err()));
    assert!(is_not_found(&driver.read_stream("u1/missing.bin", None).await.err().unwrap()));
}

// ── S3Driver ────────────────────────────────────────────────────────────────

/// Runs `body` against a fresh bucket and removes the bucket afterwards, also
/// when `body` panics. Skips (passes) without an S3 server.
macro_rules! with_bucket {
    ($prefix:expr, |$b:ident| $body:expr) => {{
        let Some($b) = TestBucket::create($prefix).await else {
            eprintln!("MODLIGHT_TEST_S3_ENDPOINT is not set: skipping the S3 storage tests");
            return;
        };
        let outcome = AssertUnwindSafe(async { $body }).catch_unwind().await;
        $b.destroy().await;
        if let Err(panic) = outcome {
            std::panic::resume_unwind(panic);
        }
    }};
}

#[tokio::test]
async fn s3_driver_upstream_suite() {
    with_bucket!(None, |b| {
        shared_suite(&b.driver).await;
        // The S3 driver's own refusal for the whole bucket.
        for prefix in ["/", ""] {
            let err = b.driver.delete_prefix(prefix).await.unwrap_err();
            assert_eq!(err.to_string(), "deletePrefix: refusing to empty the bucket");
        }
        // NoSuchKey is "already gone".
        let err = b.driver.read_buffer("u1/missing.bin").await.unwrap_err();
        assert!(is_not_found(&err), "{err:#}");
        assert_eq!(err.to_string(), "The specified key does not exist.");
        assert!(is_not_found(&b.driver.read_stream("u1/missing.bin", None).await.err().unwrap()));
        // An empty object.
        b.driver.write("u1/empty.bin", once(Vec::new())).await.unwrap();
        assert_eq!(b.driver.stat("u1/empty.bin").await.unwrap().map(|s| s.size), Some(0));
        assert!(b.driver.read_buffer("u1/empty.bin").await.unwrap().is_empty());
    });
}

#[tokio::test]
async fn s3_keys_carry_the_prefix() {
    with_bucket!(Some("/tenant/"), |b| {
        b.driver.write("u1/a.txt", once(b"abc".to_vec())).await.unwrap();
        assert_eq!(b.raw_keys().await, vec!["tenant/u1/a.txt".to_string()]);
        assert_eq!(b.driver.read_buffer("u1/a.txt").await.unwrap().as_ref(), b"abc");
        b.driver.write("derived/x/1.webp", once(b"1".to_vec())).await.unwrap();
        b.driver.delete_prefix("derived/x").await.unwrap();
        assert_eq!(b.raw_keys().await, vec!["tenant/u1/a.txt".to_string()]);
    });
}

const MIB: u64 = 1024 * 1024;

#[tokio::test]
async fn s3_write_is_one_put_up_to_a_part_and_multipart_above() {
    with_bucket!(None, |b| {
        // Exactly one part (64 MiB): a single PutObject (no "-N" ETag).
        let total = 64 * MIB;
        b.driver.write("big/exact.bin", pattern_stream(total, 1_000_003)).await.unwrap();
        let head = b.head("big/exact.bin").await;
        assert_eq!(head.content_length(), Some(total as i64));
        assert!(!head.e_tag().unwrap().contains('-'), "{:?}", head.e_tag());
        assert_eq!(head.content_type(), Some("application/octet-stream"));

        // 150 MiB: three parts (64 + 64 + 22) through the concurrent uploaders.
        let total = 150 * MIB + 5;
        b.driver.write("big/multi.bin", pattern_stream(total, 777_777)).await.unwrap();
        let head = b.head("big/multi.bin").await;
        assert_eq!(head.content_length(), Some(total as i64));
        assert!(head.e_tag().unwrap().ends_with("-3\""), "{:?}", head.e_tag());
        // CRC32 declared on CreateMultipartUpload, as lib-storage does.
        assert!(head.checksum_crc32().is_some(), "{head:?}");
        check_pattern(b.driver.read_stream("big/multi.bin", None).await.unwrap(), 0, total).await;
        // A range across the first part boundary.
        let range = ByteRange { start: 64 * MIB - 10, end: 64 * MIB + 9 };
        let got = collect(b.driver.read_stream("big/multi.bin", Some(range)).await.unwrap()).await;
        assert_eq!(got.len(), 20);
        assert!(got.iter().enumerate().all(|(i, v)| *v == byte_at(range.start + i as u64)));
        assert!(b.pending_multipart_uploads().await.is_empty());
    });
}

#[tokio::test]
async fn s3_ingest_reads_the_parts_from_the_file() {
    with_bucket!(None, |b| {
        let stage = tempfile::tempdir().unwrap();
        let src = stage.path().join("tus-payload");
        let total = 130 * MIB + 1;
        let mut data = Vec::with_capacity(total as usize);
        data.extend((0..total).map(byte_at));
        std::fs::write(&src, &data).unwrap();
        drop(data);
        b.driver.ingest_local_file(&src, "u9/video.mp4").await.unwrap();
        assert!(!src.exists());
        let head = b.head("u9/video.mp4").await;
        assert_eq!(head.content_length(), Some(total as i64));
        assert!(head.e_tag().unwrap().ends_with("-3\""), "{:?}", head.e_tag());
        check_pattern(b.driver.read_stream("u9/video.mp4", None).await.unwrap(), 0, total).await;

        // A missing temp file fails like createReadStream (ENOENT).
        let err = b.driver.ingest_local_file(&stage.path().join("gone"), "u9/x").await.unwrap_err();
        assert!(is_not_found(&err));
    });
}

#[tokio::test]
async fn s3_failed_write_aborts_the_multipart_upload() {
    with_bucket!(None, |b| {
        let good = pattern_stream(70 * MIB, 1 << 20);
        let failing: ByteStream =
            Box::pin(good.chain(futures_util::stream::once(async { Err(std::io::Error::other("client went away")) })));
        let err = b.driver.write("big/aborted.bin", failing).await.unwrap_err();
        assert_eq!(err.to_string(), "client went away");
        assert!(b.pending_multipart_uploads().await.is_empty(), "the multipart upload must be aborted");
        assert_eq!(b.driver.stat("big/aborted.bin").await.unwrap(), None);
    });
}

#[tokio::test]
async fn s3_presigned_urls_are_served() {
    with_bucket!(Some("p"), |b| {
        let payload = random_bytes(5000);
        b.driver.write("u1/file name+1.bin", once(payload.clone())).await.unwrap();
        let url = b.driver.presign_get("u1/file name+1.bin", 3600).await.unwrap().unwrap();
        assert!(url.contains("/p/u1/file%20name%2B1.bin?"), "{url}");
        assert!(url.contains("X-Amz-Expires=3600&"), "{url}");
        let (status, body) = http_get(&url, None).await;
        assert_eq!(status, 200);
        assert_eq!(body, payload);
        let (status, body) = http_get(&url, Some("bytes=10-19")).await;
        assert_eq!(status, 206);
        assert_eq!(body, &payload[10..20]);
        // An expired URL is refused by the server.
        let past = chrono::Utc::now() - chrono::Duration::hours(2);
        let expired = b.driver.presign_get_at("u1/file name+1.bin", 60, past).await.unwrap();
        assert_eq!(http_get(&expired, None).await.0, 403);
    });
}

#[tokio::test]
async fn s3_delete_prefix_pages_through_more_than_a_thousand_keys() {
    with_bucket!(None, |b| {
        let keys: Vec<String> = (0..1005).map(|i| format!("derived/many/{i:04}.webp")).collect();
        for chunk in keys.chunks(64) {
            futures_util::future::join_all(chunk.iter().map(|k| b.driver.write(k, once(vec![1])))).await;
        }
        b.driver.write("derived/other/keep.webp", once(vec![2])).await.unwrap();
        assert_eq!(b.raw_keys().await.len(), 1006);
        b.driver.delete_prefix("derived/many/").await.unwrap();
        assert_eq!(b.raw_keys().await, vec!["derived/other/keep.webp".to_string()]);
    });
}

#[tokio::test]
async fn s3_read_head_takes_only_the_window() {
    with_bucket!(None, |b| {
        let mut payload = b"%PDF-1.7".to_vec();
        payload.extend(random_bytes(3 * 1024 * 1024));
        b.driver.write("u1/big.pdf", once(payload.clone())).await.unwrap();
        let head = modlight_server::storage::read_head(&b.driver, "u1/big.pdf", 4096).await;
        assert_eq!(head.as_ref(), &payload[..4096]);
        b.driver.write("u1/small.txt", once(b"hello".to_vec())).await.unwrap();
        assert_eq!(modlight_server::storage::read_head(&b.driver, "u1/small.txt", 4096).await.as_ref(), b"hello");
        assert!(modlight_server::storage::read_head(&b.driver, "u1/does-not-exist.bin", 4096).await.is_empty());
    });
}

#[tokio::test]
async fn s3_stream_reads_survive_a_slow_reader() {
    // The SDK's stalled-stream protection must not fail a download whose
    // reader pauses (a paused video): only a stalled server is a stall.
    with_bucket!(None, |b| {
        let total = 8 * MIB;
        b.driver.write("u1/slow.bin", pattern_stream(total, 1 << 20)).await.unwrap();
        let mut stream = b.driver.read_stream("u1/slow.bin", None).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        let mut seen = first.len() as u64;
        while let Some(chunk) = stream.next().await {
            seen += chunk.unwrap().len() as u64;
        }
        assert_eq!(seen, total);
    });
}
