//! Port of `lib/storage/s3.ts`: S3-compatible storage (AWS S3, MinIO, R2, B2,
//! Spaces). NOT PORTED YET beyond its configuration: the uploads agent
//! implements the operations (multipart streaming upload, ranged GetObject,
//! HeadObject, DeleteObject(s), ListObjectsV2 for prefixes, presigned GET).
//!
//! Until then every operation fails with a clear error, so `/api/health`
//! reports `storage: fail` on an S3 install instead of pretending.

use std::path::Path;

use anyhow::bail;
use bytes::Bytes;
use futures_util::future::BoxFuture;

use super::{ByteRange, ByteStream, StatResult, StorageDriver};

#[derive(Debug, Clone)]
pub struct S3Config {
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub force_path_style: bool,
    /// Optional key prefix inside the bucket.
    pub prefix: Option<String>,
}

pub struct S3Driver {
    pub config: S3Config,
}

impl S3Driver {
    /// Read the S3_* variables as lib/storage/index.ts does, with its
    /// "Missing required env var X for S3 storage driver" refusal.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> anyhow::Result<S3Driver> {
        let non_empty = |k: &str| var(k).filter(|v| !v.is_empty());
        let required =
            |k: &str| non_empty(k).ok_or_else(|| anyhow::anyhow!("Missing required env var {k} for S3 storage driver"));
        Ok(S3Driver {
            config: S3Config {
                endpoint: non_empty("S3_ENDPOINT"),
                region: non_empty("S3_REGION").unwrap_or_else(|| "auto".to_string()),
                bucket: required("S3_BUCKET")?,
                access_key_id: required("S3_ACCESS_KEY_ID")?,
                secret_access_key: required("S3_SECRET_ACCESS_KEY")?,
                force_path_style: var("S3_FORCE_PATH_STYLE").as_deref() == Some("true"),
                prefix: non_empty("S3_PREFIX"),
            },
        })
    }
}

fn not_ported<T>() -> anyhow::Result<T> {
    bail!("the S3 storage driver is not ported to Rust yet (lib/storage/s3.ts)")
}

impl StorageDriver for S3Driver {
    fn name(&self) -> &'static str {
        "s3"
    }
    fn write<'a>(&'a self, _path: &'a str, _body: ByteStream) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { not_ported() })
    }
    fn ingest_local_file<'a>(&'a self, _tmp: &'a Path, _path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { not_ported() })
    }
    fn read_stream<'a>(
        &'a self,
        _path: &'a str,
        _range: Option<ByteRange>,
    ) -> BoxFuture<'a, anyhow::Result<ByteStream>> {
        Box::pin(async { not_ported() })
    }
    fn read_buffer<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, anyhow::Result<Bytes>> {
        Box::pin(async { not_ported() })
    }
    fn stat<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, anyhow::Result<Option<StatResult>>> {
        Box::pin(async { not_ported() })
    }
    fn delete<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { not_ported() })
    }
    fn delete_prefix<'a>(&'a self, _prefix: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { not_ported() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_variables_are_refused_like_upstream() {
        let err = S3Driver::from_env(|k| (k == "S3_BUCKET").then(|| "b".to_string())).err().unwrap();
        assert_eq!(err.to_string(), "Missing required env var S3_ACCESS_KEY_ID for S3 storage driver");
    }
}
