//! The stored form of an encrypted object: chunked XChaCha20-Poly1305 (the
//! STREAM construction of Hoang, Reyhanitabar, Rogaway and Vizár, as `age`
//! uses it), and the `uploads.encryption` header that describes it.
//!
//! The plaintext is cut into [`CHUNK_SIZE`] chunks (the last one shorter; an
//! empty object is one empty chunk). Chunk `i` is sealed under the object's own
//! random key with the nonce `prefix (19 bytes) || i (u32, big endian) ||
//! final (1 byte)` and stored with its 16-byte tag, back to back, with no
//! header. The index makes reordering detectable, the final flag truncation
//! and extension, and a fresh key per object makes nonce reuse impossible. A
//! byte range is served by decrypting only the chunks that cover it.

use std::pin::Pin;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use bytes::{Bytes, BytesMut};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use futures_util::{Stream, StreamExt};
use rand::RngCore;
use rand::rngs::OsRng;
use zeroize::Zeroizing;

use crate::storage::{ByteRange, ByteStream};

/// Plaintext bytes per chunk.
pub const CHUNK_SIZE: u32 = 64 * 1024;
/// Poly1305 tag per chunk.
pub const TAG_SIZE: u64 = 16;
/// Random part of every chunk nonce.
pub const PREFIX_SIZE: usize = 19;
/// Format version written into `uploads.encryption`.
pub const HEADER_VERSION: &str = "v1";

/// Number of chunks a plaintext of `plain_size` bytes is stored in.
pub fn chunk_count(plain_size: u64, chunk: u32) -> u64 {
    if plain_size == 0 { 1 } else { plain_size.div_ceil(u64::from(chunk)) }
}

/// Stored size of a plaintext of `plain_size` bytes.
pub fn cipher_size(plain_size: u64, chunk: u32) -> u64 {
    plain_size + TAG_SIZE * chunk_count(plain_size, chunk)
}

fn nonce(prefix: &[u8; PREFIX_SIZE], index: u32, last: bool) -> XNonce {
    let mut n = [0u8; 24];
    n[..PREFIX_SIZE].copy_from_slice(prefix);
    n[PREFIX_SIZE..PREFIX_SIZE + 4].copy_from_slice(&index.to_be_bytes());
    n[23] = u8::from(last);
    XNonce::from(n)
}

/// Why a stored object could not be decrypted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CipherError {
    #[error("encrypted object is damaged or was tampered with (chunk {0})")]
    Tampered(u64),
    #[error("encrypted object is shorter than its record says")]
    Truncated,
    #[error("encrypted object is longer than its record says")]
    TooLong,
    #[error("encrypted object is too large")]
    TooLarge,
    #[error("malformed encryption header")]
    BadHeader,
}

/// Everything needed to read or write one stored object: its key, its nonce
/// prefix, its chunk size and its plaintext size. The key is wiped on drop.
pub struct ObjectCipher {
    key: Zeroizing<[u8; 32]>,
    pub prefix: [u8; PREFIX_SIZE],
    pub chunk: u32,
    pub plain_size: u64,
}

impl std::fmt::Debug for ObjectCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectCipher").field("chunk", &self.chunk).field("plain_size", &self.plain_size).finish()
    }
}

impl ObjectCipher {
    /// A fresh key and prefix for a new object of `plain_size` bytes.
    pub fn generate(plain_size: u64) -> ObjectCipher {
        let mut key = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(key.as_mut());
        let mut prefix = [0u8; PREFIX_SIZE];
        OsRng.fill_bytes(&mut prefix);
        ObjectCipher { key, prefix, chunk: CHUNK_SIZE, plain_size }
    }

    pub fn from_parts(key: Zeroizing<[u8; 32]>, prefix: [u8; PREFIX_SIZE], chunk: u32, plain_size: u64) -> Self {
        ObjectCipher { key, prefix, chunk, plain_size }
    }

    pub fn key(&self) -> &[u8; 32] {
        &self.key
    }

    fn aead(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.key.as_ref().into())
    }

    pub fn chunks(&self) -> u64 {
        chunk_count(self.plain_size, self.chunk)
    }

    pub fn cipher_size(&self) -> u64 {
        cipher_size(self.plain_size, self.chunk)
    }

    fn index(i: u64) -> Result<u32, CipherError> {
        u32::try_from(i).map_err(|_| CipherError::TooLarge)
    }

    /// Seal chunk `i` (the caller says whether it is the last one).
    pub fn seal_chunk(
        &self,
        aead: &XChaCha20Poly1305,
        i: u64,
        last: bool,
        plain: &[u8],
    ) -> Result<Vec<u8>, CipherError> {
        let n = nonce(&self.prefix, Self::index(i)?, last);
        aead.encrypt(&n, Payload { msg: plain, aad: b"" }).map_err(|_| CipherError::TooLarge)
    }

    /// Open chunk `i`; `last` is decided by the record, never by the bytes.
    pub fn open_chunk(&self, aead: &XChaCha20Poly1305, i: u64, sealed: &[u8]) -> Result<Vec<u8>, CipherError> {
        let last = i + 1 == self.chunks();
        let n = nonce(&self.prefix, Self::index(i)?, last);
        aead.decrypt(&n, Payload { msg: sealed, aad: b"" }).map_err(|_| CipherError::Tampered(i))
    }

    /// Plaintext length of chunk `i`.
    fn plain_len(&self, i: u64) -> u64 {
        let start = i * u64::from(self.chunk);
        self.plain_size.saturating_sub(start).min(u64::from(self.chunk))
    }

    /// Encrypt a whole buffer (tests, small objects).
    pub fn encrypt_all(&self, plain: &[u8]) -> Result<Vec<u8>, CipherError> {
        if plain.len() as u64 != self.plain_size {
            return Err(if (plain.len() as u64) < self.plain_size {
                CipherError::Truncated
            } else {
                CipherError::TooLong
            });
        }
        let aead = self.aead();
        let n = self.chunks();
        let mut out = Vec::with_capacity(self.cipher_size() as usize);
        for i in 0..n {
            let start = (i * u64::from(self.chunk)) as usize;
            let end = start + self.plain_len(i) as usize;
            out.extend_from_slice(&self.seal_chunk(&aead, i, i + 1 == n, &plain[start..end])?);
        }
        Ok(out)
    }

    /// Decrypt a whole stored object (tests, small objects).
    pub fn decrypt_all(&self, sealed: &[u8]) -> Result<Vec<u8>, CipherError> {
        let expected = self.cipher_size();
        if (sealed.len() as u64) < expected {
            return Err(CipherError::Truncated);
        }
        if sealed.len() as u64 > expected {
            return Err(CipherError::TooLong);
        }
        let aead = self.aead();
        let mut out = Vec::with_capacity(self.plain_size as usize);
        let mut at = 0usize;
        for i in 0..self.chunks() {
            let len = (self.plain_len(i) + TAG_SIZE) as usize;
            out.extend_from_slice(&self.open_chunk(&aead, i, &sealed[at..at + len])?);
            at += len;
        }
        Ok(out)
    }

    /// The ciphertext byte range (inclusive) that holds the plaintext range
    /// `range`, with the chunk indices it spans.
    pub fn cipher_range(&self, range: ByteRange) -> (ByteRange, u64, u64) {
        let chunk = u64::from(self.chunk);
        let first = range.start / chunk;
        let last = range.end / chunk;
        let stride = chunk + TAG_SIZE;
        let start = first * stride;
        let end = ((last + 1) * stride).min(self.cipher_size()) - 1;
        (ByteRange { start, end }, first, last)
    }
}

// ── Streams ─────────────────────────────────────────────────────────────────

struct EncryptState {
    cipher: std::sync::Arc<ObjectCipher>,
    aead: XChaCha20Poly1305,
    inner: Option<ByteStream>,
    buf: BytesMut,
    index: u64,
    seen: u64,
    done: bool,
}

/// Encrypt a plaintext stream of exactly `cipher.plain_size` bytes into the
/// stored form. A chunk is sealed as "not last" only once more bytes are known
/// to follow, so the final flag is always right; a stream that turns out
/// shorter or longer than announced fails (the size is part of the record).
pub fn encrypt_stream(cipher: std::sync::Arc<ObjectCipher>, plain: ByteStream) -> ByteStream {
    let aead = cipher.aead();
    let state = EncryptState { cipher, aead, inner: Some(plain), buf: BytesMut::new(), index: 0, seen: 0, done: false };
    Box::pin(futures_util::stream::try_unfold(state, |mut st| async move {
        if st.done {
            return Ok(None);
        }
        let chunk = st.cipher.chunk as usize;
        loop {
            if st.buf.len() > chunk {
                let piece = st.buf.split_to(chunk);
                let sealed = st.cipher.seal_chunk(&st.aead, st.index, false, &piece).map_err(io)?;
                st.index += 1;
                return Ok(Some((Bytes::from(sealed), st)));
            }
            match st.inner.as_mut() {
                Some(inner) => match inner.next().await {
                    Some(Ok(bytes)) => {
                        st.seen += bytes.len() as u64;
                        if st.seen > st.cipher.plain_size {
                            return Err(io(CipherError::TooLong));
                        }
                        st.buf.extend_from_slice(&bytes);
                    }
                    Some(Err(e)) => return Err(e),
                    None => st.inner = None,
                },
                None => {
                    if st.seen != st.cipher.plain_size {
                        return Err(io(CipherError::Truncated));
                    }
                    let piece = std::mem::take(&mut st.buf);
                    let sealed = st.cipher.seal_chunk(&st.aead, st.index, true, &piece).map_err(io)?;
                    st.done = true;
                    return Ok(Some((Bytes::from(sealed), st)));
                }
            }
        }
    }))
}

struct DecryptState {
    cipher: std::sync::Arc<ObjectCipher>,
    aead: XChaCha20Poly1305,
    inner: ByteStream,
    buf: BytesMut,
    index: u64,
    last: u64,
    /// Plaintext bytes to drop from the front of the first chunk.
    skip: u64,
    /// Plaintext bytes still to hand out.
    remaining: u64,
}

/// Decrypt the stored bytes of `cipher_range(range)` (as read from storage)
/// into exactly the plaintext bytes of `range`. Every chunk is verified before
/// a byte of it is handed out.
pub fn decrypt_range_stream(cipher: std::sync::Arc<ObjectCipher>, range: ByteRange, sealed: ByteStream) -> ByteStream {
    let (_, first, last) = cipher.cipher_range(range);
    let chunk = u64::from(cipher.chunk);
    let aead = cipher.aead();
    let state = DecryptState {
        skip: range.start - first * chunk,
        remaining: range.end - range.start + 1,
        cipher,
        aead,
        inner: sealed,
        buf: BytesMut::new(),
        index: first,
        last,
    };
    Box::pin(futures_util::stream::try_unfold(state, |mut st| async move {
        if st.index > st.last || st.remaining == 0 {
            return Ok(None);
        }
        let need = (st.cipher.plain_len(st.index) + TAG_SIZE) as usize;
        while st.buf.len() < need {
            match st.inner.next().await {
                Some(Ok(bytes)) => st.buf.extend_from_slice(&bytes),
                Some(Err(e)) => return Err(e),
                None => return Err(io(CipherError::Truncated)),
            }
        }
        let sealed = st.buf.split_to(need);
        let plain = st.cipher.open_chunk(&st.aead, st.index, &sealed).map_err(io)?;
        let from = st.skip as usize;
        let take = ((plain.len() - from) as u64).min(st.remaining) as usize;
        st.skip = 0;
        st.remaining -= take as u64;
        st.index += 1;
        let out = Bytes::copy_from_slice(&plain[from..from + take]);
        Ok(Some((out, st)))
    }))
}

/// Decrypt a whole stored stream (the plaintext of every byte).
pub fn decrypt_stream(cipher: std::sync::Arc<ObjectCipher>, sealed: ByteStream) -> ByteStream {
    if cipher.plain_size == 0 {
        // One empty chunk: verified, nothing handed out.
        let aead = cipher.aead();
        return Box::pin(futures_util::stream::once(async move {
            let mut sealed = sealed;
            let mut buf = BytesMut::new();
            while let Some(part) = sealed.next().await {
                buf.extend_from_slice(&part?);
            }
            if buf.len() as u64 != TAG_SIZE {
                return Err(io(if (buf.len() as u64) < TAG_SIZE {
                    CipherError::Truncated
                } else {
                    CipherError::TooLong
                }));
            }
            cipher.open_chunk(&aead, 0, &buf).map_err(io)?;
            Ok(Bytes::new())
        }));
    }
    decrypt_range_stream(cipher.clone(), ByteRange { start: 0, end: cipher.plain_size - 1 }, sealed)
}

fn io(e: CipherError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, e)
}

/// Collect a stream into memory.
pub async fn collect(mut stream: Pin<Box<dyn Stream<Item = std::io::Result<Bytes>> + Send>>) -> std::io::Result<Bytes> {
    let mut out = BytesMut::new();
    while let Some(part) = stream.next().await {
        out.extend_from_slice(&part?);
    }
    Ok(out.freeze())
}

// ── The header in `uploads.encryption` ──────────────────────────────────────

/// `v1.<chunk>.<plain size>.<prefix>.<sealed key>`: what a row says about its
/// stored object. The sealed key opens only with the account's private key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CipherHeader {
    pub chunk: u32,
    pub plain_size: u64,
    pub prefix: [u8; PREFIX_SIZE],
    pub sealed_key: Vec<u8>,
}

impl CipherHeader {
    pub fn encode(&self) -> String {
        format!(
            "{HEADER_VERSION}.{}.{}.{}.{}",
            self.chunk,
            self.plain_size,
            URL_SAFE_NO_PAD.encode(self.prefix),
            URL_SAFE_NO_PAD.encode(&self.sealed_key)
        )
    }

    pub fn parse(text: &str) -> Result<CipherHeader, CipherError> {
        let parts: Vec<&str> = text.split('.').collect();
        let [version, chunk, plain, prefix, sealed] = parts.as_slice() else {
            return Err(CipherError::BadHeader);
        };
        if *version != HEADER_VERSION {
            return Err(CipherError::BadHeader);
        }
        let chunk: u32 = chunk.parse().map_err(|_| CipherError::BadHeader)?;
        // Only sizes this code writes: a record cannot make the reader
        // allocate a chunk of its choosing.
        if chunk != CHUNK_SIZE {
            return Err(CipherError::BadHeader);
        }
        let plain_size: u64 = plain.parse().map_err(|_| CipherError::BadHeader)?;
        let prefix: [u8; PREFIX_SIZE] =
            URL_SAFE_NO_PAD.decode(prefix).ok().and_then(|p| p.try_into().ok()).ok_or(CipherError::BadHeader)?;
        let sealed_key = URL_SAFE_NO_PAD.decode(sealed).map_err(|_| CipherError::BadHeader)?;
        if chunk_count(plain_size, chunk) > u64::from(u32::MAX) {
            return Err(CipherError::TooLarge);
        }
        Ok(CipherHeader { chunk, plain_size, prefix, sealed_key })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i * 31 + i / 7) as u8).collect()
    }

    fn chunked(bytes: &[u8], piece: usize) -> ByteStream {
        let parts: Vec<std::io::Result<Bytes>> =
            bytes.chunks(piece.max(1)).map(|c| Ok(Bytes::copy_from_slice(c))).collect();
        Box::pin(futures_util::stream::iter(parts))
    }

    #[test]
    fn sizes() {
        let c = CHUNK_SIZE;
        assert_eq!(chunk_count(0, c), 1);
        assert_eq!(chunk_count(1, c), 1);
        assert_eq!(chunk_count(65_536, c), 1);
        assert_eq!(chunk_count(65_537, c), 2);
        assert_eq!(cipher_size(0, c), 16);
        assert_eq!(cipher_size(65_536, c), 65_552);
        assert_eq!(cipher_size(65_537, c), 65_537 + 32);
    }

    #[tokio::test]
    async fn round_trips_at_every_awkward_size() {
        for n in [0usize, 1, 15, 16, 65_535, 65_536, 65_537, 131_072, 131_073, 200_000] {
            let plain = data(n);
            let cipher = Arc::new(ObjectCipher::generate(n as u64));
            // Streamed in odd pieces, so chunk borders never line up with them.
            let sealed = collect(encrypt_stream(cipher.clone(), chunked(&plain, 7_919))).await.unwrap();
            assert_eq!(sealed.len() as u64, cipher.cipher_size(), "{n}");
            // Same key and prefix: the streamed form is the buffered one.
            assert_eq!(sealed.to_vec(), cipher.encrypt_all(&plain).unwrap(), "{n}");
            assert_eq!(cipher.decrypt_all(&sealed).unwrap(), plain, "{n}");
            let back = collect(decrypt_stream(cipher.clone(), chunked(&sealed, 4_099))).await.unwrap();
            assert_eq!(back.to_vec(), plain, "{n}");
            // Nothing of the plaintext shows through.
            if n >= 64 {
                assert!(!sealed.windows(32).any(|w| w == &plain[..32]), "{n}");
            }
        }
    }

    #[tokio::test]
    async fn ranges_across_chunk_borders_decrypt_only_what_is_needed() {
        let n = 3 * CHUNK_SIZE as usize + 1234;
        let plain = data(n);
        let cipher = Arc::new(ObjectCipher::generate(n as u64));
        let sealed = cipher.encrypt_all(&plain).unwrap();
        let c = CHUNK_SIZE as u64;
        for (start, end) in [
            (0, 0),
            (0, 99),
            (c - 1, c),
            (c - 10, c + 10),
            (c, 2 * c - 1),
            (c + 5, 3 * c + 7),
            (3 * c, n as u64 - 1),
            (n as u64 - 1, n as u64 - 1),
            (0, n as u64 - 1),
        ] {
            let range = ByteRange { start, end };
            let (cr, first, last) = cipher.cipher_range(range);
            assert_eq!(first, start / c);
            assert_eq!(last, end / c);
            let slice = &sealed[cr.start as usize..=cr.end as usize];
            let out = collect(decrypt_range_stream(cipher.clone(), range, chunked(slice, 1_000))).await.unwrap();
            assert_eq!(out.to_vec(), plain[start as usize..=end as usize].to_vec(), "{start}-{end}");
        }
    }

    #[tokio::test]
    async fn tampering_is_detected() {
        let n = 2 * CHUNK_SIZE as usize + 10;
        let plain = data(n);
        let cipher = Arc::new(ObjectCipher::generate(n as u64));
        let sealed = cipher.encrypt_all(&plain).unwrap();

        // One flipped bit anywhere.
        for at in [0usize, 100, CHUNK_SIZE as usize + 16, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[at] ^= 1;
            assert!(matches!(cipher.decrypt_all(&bad), Err(CipherError::Tampered(_))), "{at}");
            assert!(collect(decrypt_stream(cipher.clone(), chunked(&bad, 5000))).await.is_err());
        }
        // Truncated or extended.
        assert_eq!(cipher.decrypt_all(&sealed[..sealed.len() - 1]), Err(CipherError::Truncated));
        let mut longer = sealed.clone();
        longer.push(0);
        assert_eq!(cipher.decrypt_all(&longer), Err(CipherError::TooLong));
        // Two chunks swapped.
        let stride = (CHUNK_SIZE as u64 + TAG_SIZE) as usize;
        let mut swapped = sealed.clone();
        let (a, b) = swapped.split_at_mut(stride);
        a.swap_with_slice(&mut b[..stride]);
        assert!(cipher.decrypt_all(&swapped).is_err());
        // The last full chunk passed off as the final one: a record claiming a
        // shorter file cannot cut a stored object short unnoticed.
        let short =
            ObjectCipher::from_parts(Zeroizing::new(*cipher.key()), cipher.prefix, CHUNK_SIZE, 2 * CHUNK_SIZE as u64);
        assert!(short.decrypt_all(&sealed[..2 * stride]).is_err());
        // Another key.
        let other = ObjectCipher::from_parts(Zeroizing::new([7u8; 32]), cipher.prefix, CHUNK_SIZE, n as u64);
        assert!(other.decrypt_all(&sealed).is_err());
        // An empty object is still authenticated.
        let empty = Arc::new(ObjectCipher::generate(0));
        let sealed = empty.encrypt_all(b"").unwrap();
        assert_eq!(sealed.len(), 16);
        assert!(collect(decrypt_stream(empty.clone(), chunked(&sealed, 3))).await.unwrap().is_empty());
        let mut bad = sealed.clone();
        bad[3] ^= 0x80;
        assert!(collect(decrypt_stream(empty.clone(), chunked(&bad, 3))).await.is_err());
    }

    #[tokio::test]
    async fn the_writer_refuses_a_stream_of_the_wrong_length() {
        let cipher = Arc::new(ObjectCipher::generate(10));
        assert!(collect(encrypt_stream(cipher.clone(), chunked(&data(9), 4))).await.is_err());
        assert!(collect(encrypt_stream(cipher.clone(), chunked(&data(11), 4))).await.is_err());
        assert!(collect(encrypt_stream(cipher, chunked(&data(10), 4))).await.is_ok());
    }

    #[test]
    fn header_round_trip_and_refusals() {
        let h =
            CipherHeader { chunk: CHUNK_SIZE, plain_size: 1234, prefix: [9; PREFIX_SIZE], sealed_key: vec![1, 2, 3] };
        let text = h.encode();
        assert!(text.starts_with("v1.65536.1234."));
        assert_eq!(CipherHeader::parse(&text).unwrap(), h);
        for bad in
            ["", "v2.65536.1.AAAA.AA", "v1.1024.1.CQkJCQkJCQkJCQkJCQkJCQkJCQ.AQID", "v1.65536.x.AA.AA", "v1.65536.1"]
        {
            assert!(CipherHeader::parse(bad).is_err(), "{bad}");
        }
        // A prefix of the wrong length.
        assert!(CipherHeader::parse("v1.65536.1.CQkJ.AQID").is_err());
    }
}
