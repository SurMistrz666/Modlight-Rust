//! Port of `lib/media/avif-brand.ts`: make an AVIF image SEQUENCE readable,
//! in memory only.
//!
//! libvips picks its loader from the MAJOR brand of the `ftyp` box and nothing
//! else, so a file whose major brand is `avis` is refused outright - even though
//! `avif` is right there in its compatible-brands list and libheif can decode the
//! cover image perfectly well. Four bytes in a COPY of the buffer work around the
//! reader; the stored file is never touched, because there is nothing wrong with
//! it.
//!
//! Pure and dependency-free so every libvips caller can share it: the conversion
//! path had this workaround and the thumbnail and metadata paths did not, which
//! is why one production file (alicja/tibax8.avif) had no preview, no dimensions
//! and no blurhash while converting perfectly well.

use bytes::Bytes;

/// A copy of `buf` with the major brand `avis` replaced by `avif`, or `None`
/// when `buf` is not an `ftyp`-headed file with that major brand.
pub fn with_still_avif_brand(buf: &[u8]) -> Option<Bytes> {
    if buf.len() < 16 {
        return None;
    }
    if &buf[4..8] != b"ftyp" {
        return None;
    }
    if &buf[8..12] != b"avis" {
        return None;
    }
    let mut copy = buf.to_vec();
    copy[8..12].copy_from_slice(b"avif");
    Some(Bytes::from(copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(brand: &[u8; 4]) -> Vec<u8> {
        let mut v = vec![0, 0, 0, 0x1c];
        v.extend_from_slice(b"ftyp");
        v.extend_from_slice(brand);
        v.extend_from_slice(&[0, 0, 0, 0]);
        v.extend_from_slice(b"avifmif1miaf");
        v
    }

    #[test]
    fn swaps_only_the_major_brand_of_a_copy() {
        let original = boxed(b"avis");
        let patched = with_still_avif_brand(&original).expect("patched");
        assert_eq!(&patched[8..12], b"avif");
        assert_eq!(&patched[..8], &original[..8]);
        assert_eq!(&patched[12..], &original[12..]);
        // The input is untouched.
        assert_eq!(&original[8..12], b"avis");
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(with_still_avif_brand(&boxed(b"avif")), None);
        assert_eq!(with_still_avif_brand(&boxed(b"heic")), None);
        assert_eq!(with_still_avif_brand(&[0u8; 4]), None);
        assert_eq!(with_still_avif_brand(&[0u8; 15]), None);
        let mut not_ftyp = boxed(b"avis");
        not_ftyp[4..8].copy_from_slice(b"moov");
        assert_eq!(with_still_avif_brand(&not_ftyp), None);
    }
}
