//! tests/integration/derived-cache.test.ts: the derived cache on a real
//! local storage directory.

use bytes::Bytes;
use modlight_server::media::derived_cache::{
    DERIVED_PREFIX, content_hash, derived_key, purge_derived_lazy, read_derived, read_derived_stream, write_derived,
};
use modlight_server::storage::{LocalDriver, Storage};

const UPLOAD_A: &str = "11111111-1111-4111-8111-111111111111";
const UPLOAD_B: &str = "22222222-2222-4222-8222-222222222222";

fn storage(root: &std::path::Path) -> Storage {
    Storage::with_driver(std::sync::Arc::new(LocalDriver::new(&root.to_string_lossy())))
}

#[tokio::test]
async fn the_derived_cache() {
    let dir = tempfile::tempdir().unwrap();
    let s = storage(dir.path());

    // stores and returns bytes unchanged
    let body = Bytes::from_static(b"%PDF-1.7 pretend conversion output");
    let key = derived_key(UPLOAD_A, &content_hash(&body), "pdf", "pdf");
    assert!(read_derived(&s, &key).await.is_none());
    write_derived(&s, &key, body.clone()).await;
    assert_eq!(read_derived(&s, &key).await.unwrap(), body);
    let (_, size) = read_derived_stream(&s, &key).await.unwrap();
    assert_eq!(size, body.len() as u64);

    // keeps every derived file outside the storage area quota is computed from
    let x = Bytes::from_static(b"x");
    write_derived(&s, &derived_key(UPLOAD_A, &content_hash(&x), "png", "png"), x).await;
    let top: Vec<String> =
        std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(top, vec![DERIVED_PREFIX.to_string()]);

    // removes everything for one upload and nothing for its neighbour
    let (a, b) = (Bytes::from_static(b"belongs to A"), Bytes::from_static(b"belongs to B"));
    let keys_a: Vec<String> = ["png", "pdf", "webp-anim"]
        .iter()
        .map(|v| derived_key(UPLOAD_A, &content_hash(&a), v, v.split('-').next().unwrap()))
        .collect();
    let key_b = derived_key(UPLOAD_B, &content_hash(&b), "png", "png");
    for k in &keys_a {
        write_derived(&s, k, a.clone()).await;
    }
    write_derived(&s, &key_b, b).await;
    purge_derived_lazy(&s, UPLOAD_A).await;
    for k in &keys_a {
        assert!(read_derived(&s, k).await.is_none(), "{k}");
    }
    assert!(read_derived(&s, &key_b).await.is_some());

    // purging an upload that never had a cache entry is success; a miss is not an error
    purge_derived_lazy(&s, "33333333-3333-4333-8333-333333333333").await;
    assert!(read_derived(&s, "derived/nope/nothing.png").await.is_none());
    assert!(read_derived_stream(&s, "derived/nope/nothing.png").await.is_none());
}
