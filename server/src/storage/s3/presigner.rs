//! Port of what lib/storage/s3.ts `presignGet` calls:
//! `getSignedUrl(client, new GetObjectCommand({Bucket, Key}), {expiresIn})` of
//! `@aws-sdk/s3-request-presigner` 3.1058.0, which signs with the query-string
//! variant of SigV4 in `@smithy/signature-v4` 5.4.6 (`presign`).
//!
//! The URL is the one the JavaScript SDK builds, byte for byte: the endpoint
//! comes from the S3 endpoint rules (the SDK's own resolver: path style or
//! virtual host, custom endpoint, region), the key path is escaped per segment,
//! the request carries `x-id=GetObject`, the `x-amz-checksum-mode: ENABLED`
//! header the client adds when it validates response checksums, and the
//! presigner's `X-Amz-Content-Sha256: UNSIGNED-PAYLOAD`; both headers are
//! hoisted into the query, only `host` is signed, and the query is written
//! sorted by name.

use std::collections::BTreeMap;

use anyhow::{Context, bail};
use aws_sdk_s3::config::endpoint::{DefaultResolver, Params, ResolveEndpoint};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use sha2::{Digest, Sha256};

use super::S3Config;

/// `MAX_PRESIGNED_TTL`: one week.
pub const MAX_PRESIGNED_TTL: u64 = 60 * 60 * 24 * 7;
const ALGORITHM_IDENTIFIER: &str = "AWS4-HMAC-SHA256";
pub const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";
const KEY_TYPE_IDENTIFIER: &str = "aws4_request";

/// `encodeURIComponent` plus `!'()*` (`escapeUri` of @smithy/core): only
/// `A-Z a-z 0-9 - _ . ~` stay as they are.
const URI_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

/// `escapeUri(uri)`
pub fn escape_uri(s: &str) -> String {
    utf8_percent_encode(s, URI_ESCAPE).to_string()
}

/// `escapeUriPath(uri)`: every `/`-separated segment escaped (a greedy label).
pub fn escape_uri_path(s: &str) -> String {
    s.split('/').map(escape_uri).collect::<Vec<_>>().join("/")
}

/// `buildQueryString(query)`: sorted by name, names and values escaped.
pub fn build_query_string(query: &BTreeMap<String, String>) -> String {
    query.iter().map(|(k, v)| format!("{}={}", escape_uri(k), escape_uri(v))).collect::<Vec<_>>().join("&")
}

/// The request `SignatureV4.presign` receives, after `moveHeadersToQuery`.
pub struct PresignInput<'a> {
    pub method: &'a str,
    /// The `host` header: hostname plus a non-default port.
    pub host: &'a str,
    /// The path as sent: already escaped (S3 signs with `uriEscapePath: false`).
    pub path: &'a str,
    /// Query parameters, unescaped (hoisted `x-amz-*` headers included).
    pub query: BTreeMap<String, String>,
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub session_token: Option<&'a str>,
    pub region: &'a str,
    pub service: &'a str,
    pub signing_date: DateTime<Utc>,
    pub expires_in: u64,
    pub payload_hash: &'a str,
}

/// What `presign` produced: the final query (signature included) and the
/// intermediate strings, for tests.
#[derive(Debug)]
pub struct Presigned {
    pub query: BTreeMap<String, String>,
    pub canonical_request: String,
    pub string_to_sign: String,
    pub signature: String,
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// `getCanonicalQuery`: names escaped and sorted, `X-Amz-Signature` left out.
fn canonical_query(query: &BTreeMap<String, String>) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("x-amz-signature"))
        .map(|(k, v)| (escape_uri(k), escape_uri(v)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    pairs.into_iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

/// `SignatureV4.presign(request, { signingDate, expiresIn, ... })` for a
/// request whose only signed header is `host`.
pub fn presign(input: PresignInput<'_>) -> anyhow::Result<Presigned> {
    if input.expires_in > MAX_PRESIGNED_TTL {
        bail!("Signature version 4 presigned URLs must have an expiration date less than one week in the future");
    }
    let long_date = input.signing_date.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = &long_date[..8];
    let scope = format!("{short_date}/{}/{}/{KEY_TYPE_IDENTIFIER}", input.region, input.service);

    let mut query = input.query;
    if let Some(token) = input.session_token {
        query.insert("X-Amz-Security-Token".into(), token.into());
    }
    query.insert("X-Amz-Algorithm".into(), ALGORITHM_IDENTIFIER.into());
    query.insert("X-Amz-Credential".into(), format!("{}/{scope}", input.access_key_id));
    query.insert("X-Amz-Date".into(), long_date.clone());
    query.insert("X-Amz-Expires".into(), input.expires_in.to_string());
    // getCanonicalHeaders: trimmed, inner whitespace collapsed.
    let host = input.host.split_whitespace().collect::<Vec<_>>().join(" ");
    query.insert("X-Amz-SignedHeaders".into(), "host".into());

    let canonical_request = format!(
        "{}\n{}\n{}\nhost:{host}\n\nhost\n{}",
        input.method,
        input.path,
        canonical_query(&query),
        input.payload_hash
    );
    let string_to_sign = format!(
        "{ALGORITHM_IDENTIFIER}\n{long_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let mut key = format!("AWS4{}", input.secret_access_key).into_bytes();
    for signable in [short_date, input.region, input.service, KEY_TYPE_IDENTIFIER] {
        key = hmac_sha256(&key, signable.as_bytes());
    }
    let signature = hex::encode(hmac_sha256(&key, string_to_sign.as_bytes()));
    query.insert("X-Amz-Signature".into(), signature.clone());
    Ok(Presigned { query, canonical_request, string_to_sign, signature })
}

/// Where a GetObject for `key` goes, per the S3 endpoint rules.
pub struct ResolvedTarget {
    pub scheme: String,
    /// Hostname plus a non-default port (the `host` header and URL authority).
    pub host: String,
    /// Endpoint path plus the escaped key.
    pub path: String,
    pub signing_region: String,
    pub signing_name: String,
}

/// The endpoint the client would send `GetObject { Bucket, Key }` to.
pub async fn resolve_get_object(config: &S3Config, key: &str) -> anyhow::Result<ResolvedTarget> {
    let params = Params::builder()
        .bucket(config.bucket.clone())
        .key(key.to_string())
        .region(config.region.clone())
        .set_endpoint(config.endpoint.clone().filter(|e| !e.is_empty()))
        .force_path_style(config.force_path_style)
        .use_fips(false)
        .use_dual_stack(false)
        .accelerate(false)
        .use_global_endpoint(false)
        .disable_multi_region_access_points(false)
        .build()?;
    let endpoint = DefaultResolver::new()
        .resolve_endpoint(&params)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("resolving the S3 endpoint")?;
    let url = url::Url::parse(endpoint.url()).context("parsing the S3 endpoint")?;
    let hostname = url.host_str().context("the S3 endpoint has no host")?;
    let host = match url.port() {
        Some(port) => format!("{hostname}:{port}"),
        None => hostname.to_string(),
    };
    let (mut signing_region, mut signing_name) = (config.region.clone(), "s3".to_string());
    if let Some(scheme) = endpoint.auth_schemes().first() {
        if scheme.name() != "sigv4" {
            bail!("presigning with the {} auth scheme is not supported", scheme.name());
        }
        if let Some(region) = scheme.get("signingRegion").and_then(|d| d.as_string()) {
            signing_region = region.to_string();
        }
        if let Some(name) = scheme.get("signingName").and_then(|d| d.as_string()) {
            signing_name = name.to_string();
        }
    }
    // RequestBuilder.build(): the endpoint path without one trailing "/",
    // then the operation's `/{Key+}`.
    let base = url.path().strip_suffix('/').unwrap_or(url.path());
    let path = format!("{base}/{}", escape_uri_path(key));
    Ok(ResolvedTarget { scheme: url.scheme().to_string(), host, path, signing_region, signing_name })
}

/// `getSignedUrl(client, new GetObjectCommand({ Bucket, Key }), { expiresIn,
/// signingDate })`; `key` is the full object key (prefix included).
pub async fn get_signed_url_get_object(
    config: &S3Config,
    key: &str,
    expires_in: u64,
    signing_date: DateTime<Utc>,
) -> anyhow::Result<String> {
    let target = resolve_get_object(config, key).await?;
    let query = BTreeMap::from([
        ("x-id".to_string(), "GetObject".to_string()),
        ("x-amz-checksum-mode".to_string(), "ENABLED".to_string()),
        ("X-Amz-Content-Sha256".to_string(), UNSIGNED_PAYLOAD.to_string()),
    ]);
    let presigned = presign(PresignInput {
        method: "GET",
        host: &target.host,
        path: &target.path,
        query,
        access_key_id: &config.access_key_id,
        secret_access_key: &config.secret_access_key,
        session_token: None,
        region: &target.signing_region,
        service: &target.signing_name,
        signing_date,
        expires_in,
        payload_hash: UNSIGNED_PAYLOAD,
    })?;
    Ok(format!("{}://{}{}?{}", target.scheme, target.host, target.path, build_query_string(&presigned.query)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AK: &str = "AKIAIOSFODNN7EXAMPLE";
    const SK: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    fn date(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// The query-string example of the AWS SigV4 documentation ("Authenticating
    /// Requests: Using Query Parameters"): GET /test.txt from examplebucket,
    /// us-east-1, 2013-05-24T00:00:00Z, expires in 86400 s.
    #[test]
    fn aws_documentation_presign_vector() {
        let p = presign(PresignInput {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            query: BTreeMap::new(),
            access_key_id: AK,
            secret_access_key: SK,
            session_token: None,
            region: "us-east-1",
            service: "s3",
            signing_date: date("2013-05-24T00:00:00Z"),
            expires_in: 86400,
            payload_hash: UNSIGNED_PAYLOAD,
        })
        .unwrap();
        assert_eq!(
            p.canonical_request,
            "GET\n/test.txt\nX-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20130524%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20130524T000000Z&X-Amz-Expires=86400&X-Amz-SignedHeaders=host\nhost:examplebucket.s3.amazonaws.com\n\nhost\nUNSIGNED-PAYLOAD"
        );
        assert_eq!(
            p.string_to_sign,
            "AWS4-HMAC-SHA256\n20130524T000000Z\n20130524/us-east-1/s3/aws4_request\n3bfa292879f6447bbcda7001decf97f4a54dc650c8942174ae0a9121cf58ad04"
        );
        assert_eq!(p.signature, "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404");
    }

    #[test]
    fn escaping_matches_smithy() {
        assert_eq!(escape_uri("a b+c&d=e!f'g(h)i*j~k_l.m-n/o"), "a%20b%2Bc%26d%3De%21f%27g%28h%29i%2Aj~k_l.m-n%2Fo");
        assert_eq!(escape_uri("\u{105}\u{119}"), "%C4%85%C4%99");
        assert_eq!(escape_uri_path("a//b/./c d"), "a//b/./c%20d");
    }

    #[test]
    fn expiry_over_a_week_is_refused_like_signature_v4() {
        let err = presign(PresignInput {
            method: "GET",
            host: "h",
            path: "/k",
            query: BTreeMap::new(),
            access_key_id: AK,
            secret_access_key: SK,
            session_token: None,
            region: "auto",
            service: "s3",
            signing_date: Utc::now(),
            expires_in: MAX_PRESIGNED_TTL + 1,
            payload_hash: UNSIGNED_PAYLOAD,
        })
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Signature version 4 presigned URLs must have an expiration date less than one week in the future"
        );
    }

    /// URLs printed by `getSignedUrl` of @aws-sdk/s3-request-presigner 3.1058.0
    /// (with @aws-sdk/client-s3 3.1058.0) inside a throwaway container of the
    /// reference image, for the same inputs and `signingDate`.
    #[tokio::test]
    async fn byte_identical_to_the_javascript_presigner() {
        /// endpoint, forcePathStyle, region, bucket, key, expiresIn, the JavaScript URL
        type Case<'a> = (Option<&'a str>, bool, &'a str, &'a str, &'a str, u64, &'a str);
        let cases: [Case; 8] = [
            (
                Some("http://127.0.0.1:59425"),
                true,
                "auto",
                "modlight-test",
                "u1/abc.png",
                3600,
                "http://127.0.0.1:59425/modlight-test/u1/abc.png?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=65c55c0d801544533a78d301412695f94c012143b125dc8ab1c904578536ec1a&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                Some("http://127.0.0.1:59425"),
                false,
                "auto",
                "modlight-test",
                "derived/x y/\u{105}\u{119}+&=!'()*~.webp",
                3600,
                "http://127.0.0.1:59425/modlight-test/derived/x%20y/%C4%85%C4%99%2B%26%3D%21%27%28%29%2A~.webp?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=69684c3408ba432585399fb4e15cbd477bbc6b0622822ccf37c01ab843d7b234&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                Some("http://localhost:59425"),
                false,
                "us-east-1",
                "modlight-test",
                "a//b/./c",
                3600,
                "http://modlight-test.localhost:59425/a//b/./c?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=9d955947d7f9ace378144175bd8a29d55198b9cefe4f3c1d49c0f63756d3a1d8&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                Some("https://acc123.r2.cloudflarestorage.com"),
                false,
                "auto",
                "media",
                "p/u/c.mp4",
                3600,
                "https://media.acc123.r2.cloudflarestorage.com/p/u/c.mp4?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=f4cf6406c004da9e9aa7c64f2cb577dd5810b045400353ffd493bbf4b3a3d67c&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                Some("https://minio.example.com:9443/base/"),
                true,
                "auto",
                "Media_Bucket",
                "k",
                3600,
                "https://minio.example.com:9443/base/Media_Bucket/k?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=bb59673a39f20c73f6364ab7cc6accd394d1f111d43ef53f006ec143f782e4a3&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                None,
                false,
                "eu-central-1",
                "my.bucket",
                "a/b",
                3600,
                "https://s3.eu-central-1.amazonaws.com/my.bucket/a/b?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Feu-central-1%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=ac38092944979ddc47908ea606ed966f5b6b9af8b17e2b68023400b165b8c5b8&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                None,
                false,
                "auto",
                "bucket",
                "k",
                3600,
                "https://bucket.s3.auto.amazonaws.com/k?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fauto%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=3600&X-Amz-Signature=9b1d042b07efb44a2232b3910a3c0fdeca91ca2752f4a0f4c44032e1ec895bed&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
            (
                None,
                false,
                "us-east-1",
                "bucket",
                "k",
                604800,
                "https://bucket.s3.us-east-1.amazonaws.com/k?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Content-Sha256=UNSIGNED-PAYLOAD&X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261019%2Fus-east-1%2Fs3%2Faws4_request&X-Amz-Date=20261019T123456Z&X-Amz-Expires=604800&X-Amz-Signature=c11ce19b3cd2fc6f276a834c7a3ae8a43d7910cf8da9b1dbff2fd6477450362d&X-Amz-SignedHeaders=host&x-amz-checksum-mode=ENABLED&x-id=GetObject",
            ),
        ];
        for (endpoint, force_path_style, region, bucket, key, expires_in, want) in cases {
            let config = S3Config {
                endpoint: endpoint.map(str::to_string),
                region: region.into(),
                bucket: bucket.into(),
                access_key_id: AK.into(),
                secret_access_key: SK.into(),
                force_path_style,
                prefix: None,
            };
            let got = get_signed_url_get_object(&config, key, expires_in, date("2026-10-19T12:34:56Z")).await.unwrap();
            assert_eq!(got, want, "{endpoint:?} {bucket} {key}");
        }
    }
}
