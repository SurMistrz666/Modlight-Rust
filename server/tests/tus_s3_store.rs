//! The tus S3 datastore (`@tus/s3-store` port) against a real S3-compatible
//! server, configured as lib/upload/tus.ts `makeDatastore()` does. Same gate as
//! tests/storage.rs: runs only with `MODLIGHT_TEST_S3_ENDPOINT` (and the
//! `MODLIGHT_TEST_S3_ACCESS_KEY_ID` / `_SECRET_ACCESS_KEY` it needs).
//!
//! The core case is upstream's tests/integration/tus-resume.audit.test.ts
//! ("does a tus upload survive a connection that dies mid-chunk?") at the
//! store level: a PATCH body that stops early keeps what arrived, the next
//! PATCH resumes from exactly there, and the result is byte-identical.

mod s3_common;

use std::panic::AssertUnwindSafe;

use bytes::Bytes;
use futures_util::{FutureExt, StreamExt};
use modlight_server::storage::ByteStream;
use modlight_server::storage::s3::S3Config;
use modlight_server::upload::tus_server::s3_store::{S3Store, S3StoreOptions};
use modlight_server::upload::tus_server::store::{DataStore, Metadata, StoreError, Upload, UploadStorage, errors};
use s3_common::{TestBucket, check_pattern, pattern_stream_from};

const MIB: u64 = 1024 * 1024;

/// `makeDatastore()` against the test bucket, chunks staged in `tmp`.
fn modlight_store(config: &S3Config, tmp: &std::path::Path) -> S3Store {
    let c = config.clone();
    let var = move |k: &str| match k {
        "S3_BUCKET" => Some(c.bucket.clone()),
        "S3_REGION" => Some(c.region.clone()),
        "S3_ENDPOINT" => c.endpoint.clone(),
        "S3_FORCE_PATH_STYLE" => Some(c.force_path_style.to_string()),
        "S3_ACCESS_KEY_ID" => Some(c.access_key_id.clone()),
        "S3_SECRET_ACCESS_KEY" => Some(c.secret_access_key.clone()),
        _ => None,
    };
    let store = S3Store::from_env(var).expect("store");
    // Same configuration, only the chunk directory moved (to check cleanup).
    let mut options = S3StoreOptions::new(config.clone());
    options.part_size = Some(64 * MIB);
    options.expiration_period_in_milliseconds = Some(store.get_expiration());
    options.tmp_dir = Some(tmp.to_path_buf());
    S3Store::new(options)
}

fn new_id() -> String {
    format!("{:016x}{:016x}", rand::random::<u64>(), rand::random::<u64>())
}

fn metadata(pairs: &[(&str, Option<&str>)]) -> Metadata {
    pairs.iter().map(|(k, v)| (k.to_string(), v.map(str::to_string))).collect()
}

/// A body that delivers `len` pattern bytes from `start` and then stops, the
/// way the tus server hands over a request whose client went away.
fn body(start: u64, len: u64) -> ByteStream {
    pattern_stream_from(start, len, 256 * 1024 + 3)
}

async fn tagging(b: &TestBucket, key: &str) -> Vec<(String, String)> {
    let out = b.client.get_object_tagging().bucket(&b.config.bucket).key(key).send().await.expect("tagging");
    out.tag_set().iter().map(|t| (t.key().to_string(), t.value().to_string())).collect()
}

async fn info_object(b: &TestBucket, id: &str) -> (String, std::collections::HashMap<String, String>, Option<String>) {
    let out = b.client.get_object().bucket(&b.config.bucket).key(format!("{id}.info")).send().await.expect("info");
    let meta = out.metadata().cloned().unwrap_or_default();
    let content_type = out.content_type().map(str::to_string);
    let body = out.body.collect().await.unwrap().into_bytes();
    (String::from_utf8(body.to_vec()).unwrap(), meta, content_type)
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

macro_rules! with_bucket {
    (|$b:ident| $body:expr) => {{
        let Some($b) = TestBucket::create(None).await else {
            eprintln!("MODLIGHT_TEST_S3_ENDPOINT is not set: skipping the tus S3 store tests");
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
async fn interrupted_patch_keeps_its_bytes_and_resumes_byte_identical() {
    with_bucket!(|b| {
        let tmp = tempfile::tempdir().unwrap();
        let store = modlight_store(&b.config, tmp.path());
        const TOTAL: u64 = 8 * MIB;
        const CUT_AFTER: u64 = 3 * MIB;
        let id = new_id();
        let meta = metadata(&[
            ("filename", Some("audit.bin")),
            ("filetype", Some("application/octet-stream")),
            ("userId", Some("u1")),
        ]);

        // ── create ──
        let created = store.create(Upload::new(id.clone(), Some(TOTAL), 0, Some(meta.clone()))).await.unwrap();
        assert_eq!(
            created.storage,
            Some(UploadStorage { kind: "s3".into(), path: id.clone(), bucket: Some(b.config.bucket.clone()) })
        );
        let (json, user_meta, content_type) = info_object(&b, &id).await;
        assert_eq!(json, serde_json::to_string(&created).unwrap());
        assert_eq!(user_meta.get("tus-version").map(String::as_str), Some("1.0.0"));
        let upload_id = user_meta.get("upload-id").cloned().expect("upload-id");
        assert_eq!(b.pending_multipart_uploads().await, vec![(id.clone(), upload_id)]);
        assert_eq!(content_type.as_deref(), Some("application/octet-stream"));
        assert_eq!(tagging(&b, &format!("{id}.info")).await, vec![("Tus-Completed".into(), "false".into())]);
        assert_eq!(store.get_upload(&id).await.unwrap().offset, 0);

        // ── first PATCH, cut after 3 MiB (the dropped connection) ──
        assert_eq!(store.write(body(0, CUT_AFTER), &id, 0).await.unwrap(), CUT_AFTER);
        // Under the 5 MiB minimum and not the end: parked as <id>.part.
        assert_eq!(b.head_raw(&format!("{id}.part")).await.content_length(), Some(CUT_AFTER as i64));
        assert_eq!(tagging(&b, &format!("{id}.part")).await, vec![("Tus-Completed".into(), "false".into())]);

        // ── how much survived? (a fresh store: no cache, like another process) ──
        let other = modlight_store(&b.config, tmp.path());
        let head = other.get_upload(&id).await.unwrap();
        assert_eq!(head.offset, CUT_AFTER);
        assert_eq!(head.size, Some(TOTAL));
        assert_eq!(head.metadata, Some(meta.clone()));
        assert_eq!(head.creation_date, created.creation_date);

        // ── resume from exactly there ──
        assert_eq!(other.write(body(CUT_AFTER, TOTAL - CUT_AFTER), &id, CUT_AFTER).await.unwrap(), TOTAL);
        assert!(b.pending_multipart_uploads().await.is_empty(), "the multipart upload is completed");
        assert_eq!(b.raw_keys().await, vec![id.clone(), format!("{id}.info")]);
        check_pattern(store.read(&id).await.unwrap(), 0, TOTAL).await;
        // The info object stays, now tagged as completed.
        assert_eq!(tagging(&b, &format!("{id}.info")).await, vec![("Tus-Completed".into(), "true".into())]);
        let finished = store.get_upload(&id).await.unwrap();
        assert_eq!((finished.offset, finished.size), (TOTAL, Some(TOTAL)));
        // No chunk or downloaded part is left behind.
        assert_eq!(files_in(tmp.path()), Vec::<String>::new());
    });
}

#[tokio::test]
async fn large_upload_in_parts_with_parked_tail() {
    with_bucket!(|b| {
        let tmp = tempfile::tempdir().unwrap();
        let store = modlight_store(&b.config, tmp.path());
        let total = 150 * MIB + 7;
        let id = new_id();
        store
            .create(Upload::new(id.clone(), Some(total), 0, Some(metadata(&[("contentType", Some("video/mp4"))]))))
            .await
            .unwrap();

        // 70 MiB: a 64 MiB chunk and a 6 MiB one, both real parts (>= 5 MiB).
        assert_eq!(store.write(body(0, 70 * MIB), &id, 0).await.unwrap(), 70 * MIB);
        assert_eq!(store.get_upload(&id).await.unwrap().offset, 70 * MIB);
        // 2 MiB more: parked.
        assert_eq!(store.write(body(70 * MIB, 2 * MIB), &id, 70 * MIB).await.unwrap(), 72 * MIB);
        assert_eq!(store.get_upload(&id).await.unwrap().offset, 72 * MIB);
        // An offset the store does not have is the server's 409, not ours:
        // the rest, with the parked tail in front.
        assert_eq!(store.write(body(72 * MIB, total - 72 * MIB), &id, 72 * MIB).await.unwrap(), total);

        let head = b.head_raw(&id).await;
        assert_eq!(head.content_length(), Some(total as i64));
        assert!(head.e_tag().unwrap().ends_with("-4\""), "{:?}", head.e_tag());
        // `contentType` metadata becomes the object's Content-Type.
        assert_eq!(head.content_type(), Some("video/mp4"));
        check_pattern(store.read(&id).await.unwrap(), 0, total).await;
        assert_eq!(files_in(tmp.path()), Vec::<String>::new());
    });
}

#[tokio::test]
async fn deferred_length_zero_length_and_errors() {
    with_bucket!(|b| {
        let tmp = tempfile::tempdir().unwrap();
        let store = modlight_store(&b.config, tmp.path());

        // Upload-Defer-Length, then the length on a later PATCH.
        let id = new_id();
        store.create(Upload::new(id.clone(), None, 0, None)).await.unwrap();
        let (json, _, _) = info_object(&b, &id).await;
        assert!(!json.contains("\"size\""), "{json}");
        assert_eq!(store.write(body(0, MIB), &id, 0).await.unwrap(), MIB);
        store.declare_upload_length(&id, 3 * MIB).await.unwrap();
        let (json, _, _) = info_object(&b, &id).await;
        assert!(json.contains(&format!("\"size\":{}", 3 * MIB)), "{json}");
        assert_eq!(store.get_upload(&id).await.unwrap().size, Some(3 * MIB));
        assert_eq!(store.write(body(MIB, 2 * MIB), &id, MIB).await.unwrap(), 3 * MIB);
        check_pattern(store.read(&id).await.unwrap(), 0, 3 * MIB).await;

        // Upload-Length: 0 finishes with one empty part.
        let empty = new_id();
        store.create(Upload::new(empty.clone(), Some(0), 0, None)).await.unwrap();
        let none: ByteStream = Box::pin(futures_util::stream::empty());
        assert_eq!(store.write(none, &empty, 0).await.unwrap(), 0);
        assert_eq!(b.head_raw(&empty).await.content_length(), Some(0));

        // Unknown uploads: HEAD answers 404; DELETE fails with S3's own message.
        let missing = new_id();
        match store.get_upload(&missing).await {
            Err(StoreError::Tus(e)) => assert_eq!(e, errors::FILE_NOT_FOUND),
            other => panic!("{other:?}"),
        }
        let err = store.remove(&missing).await.unwrap_err();
        assert_eq!(
            err.status_and_body(),
            (500, "Something went wrong with that request\nThe specified key does not exist.\n".to_string())
        );

        // A body error (the size limiter) keeps its tus status; the open chunk
        // is dropped, nothing is stored.
        let limited = new_id();
        store.create(Upload::new(limited.clone(), Some(10 * MIB), 0, None)).await.unwrap();
        let failing: ByteStream =
            Box::pin(body(0, 3 * MIB).chain(futures_util::stream::once(async {
                Err(std::io::Error::other(errors::ERR_MAX_SIZE_EXCEEDED))
            })));
        match store.write(failing, &limited, 0).await {
            Err(StoreError::Tus(e)) => assert_eq!(e, errors::ERR_MAX_SIZE_EXCEEDED),
            other => panic!("{other:?}"),
        }
        assert_eq!(store.get_upload(&limited).await.unwrap().offset, 0);
        assert_eq!(files_in(tmp.path()), Vec::<String>::new());
    });
}

#[tokio::test]
async fn termination_and_expiration() {
    with_bucket!(|b| {
        let tmp = tempfile::tempdir().unwrap();
        let store = modlight_store(&b.config, tmp.path());

        // DELETE of an upload in progress: multipart aborted, object and info
        // deleted (a parked part is left, as upstream leaves it).
        let id = new_id();
        store.create(Upload::new(id.clone(), Some(20 * MIB), 0, None)).await.unwrap();
        store.write(body(0, MIB), &id, 0).await.unwrap();
        store.remove(&id).await.unwrap();
        assert!(b.pending_multipart_uploads().await.is_empty());
        assert_eq!(b.raw_keys().await, vec![format!("{id}.part")]);
        assert_eq!(store.get_upload(&id).await.unwrap_err().status_and_body().0, 404);
        // A finished upload: aborting its completed multipart upload is
        // NoSuchUpload on AWS (500 from the tus server) and a no-op on MinIO,
        // where the object and its info are then deleted. Either way the
        // requests are upstream's.
        let done = new_id();
        store.create(Upload::new(done.clone(), Some(10), 0, None)).await.unwrap();
        store.write(body(0, 10), &done, 0).await.unwrap();
        match store.remove(&done).await {
            Ok(()) => assert!(!b.raw_keys().await.iter().any(|k| k.starts_with(&done))),
            Err(err) => {
                let (status, body_text) = err.status_and_body();
                assert_eq!(status, 500);
                assert!(body_text.starts_with("Something went wrong with that request\n"), "{body_text}");
            }
        }

        // Expiration: uploads initiated longer ago than the period are aborted
        // and their info and part objects deleted.
        let mut options = S3StoreOptions::new(b.config.clone());
        options.expiration_period_in_milliseconds = Some(1);
        options.tmp_dir = Some(tmp.path().to_path_buf());
        let short = S3Store::new(options);
        let stale = new_id();
        short.create(Upload::new(stale.clone(), Some(MIB), 0, None)).await.unwrap();
        short.write(body(0, 1000), &stale, 0).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let deleted = short.delete_expired().await.unwrap();
        assert_eq!(deleted, 2, "info + part reported deleted");
        assert!(b.pending_multipart_uploads().await.is_empty());
        let keys = b.raw_keys().await;
        assert!(!keys.contains(&format!("{stale}.info")) && !keys.contains(&format!("{stale}.part")), "{keys:?}");
        // The store with the 24 h period leaves young uploads alone.
        let young = new_id();
        store.create(Upload::new(young.clone(), Some(MIB), 0, None)).await.unwrap();
        assert_eq!(store.delete_expired().await.unwrap(), 0);
        assert_eq!(b.pending_multipart_uploads().await.len(), 1);
    });
}

#[tokio::test]
async fn parked_tail_round_trip_through_bytes() {
    // Many tiny PATCHes (each under 5 MiB) accumulate in the parked part.
    with_bucket!(|b| {
        let tmp = tempfile::tempdir().unwrap();
        let store = modlight_store(&b.config, tmp.path());
        let id = new_id();
        let total = 6 * MIB;
        store.create(Upload::new(id.clone(), Some(total), 0, None)).await.unwrap();
        let mut offset = 0;
        for step in [MIB, 2 * MIB, MIB + 5, total - (4 * MIB + 5)] {
            let one: ByteStream = Box::pin(futures_util::stream::once({
                let data: Vec<u8> = (offset..offset + step).map(s3_common::byte_at).collect();
                async move { Ok(Bytes::from(data)) }
            }));
            offset = store.write(one, &id, offset).await.unwrap();
            assert_eq!(store.get_upload(&id).await.unwrap().offset, offset);
        }
        assert_eq!(offset, total);
        check_pattern(store.read(&id).await.unwrap(), 0, total).await;
        assert_eq!(b.raw_keys().await, vec![id.clone(), format!("{id}.info")]);
    });
}
