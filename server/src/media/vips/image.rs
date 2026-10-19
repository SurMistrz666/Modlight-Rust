//! `Image` (one owned reference to a `VipsImage`), `Blob` (input bytes lent
//! to libvips without a copy), and typed wrappers for the operations the
//! sharp pipeline uses. Every wrapper is one [`Operation`] call; adding
//! another libvips operation means adding another five-line method here.

use std::collections::HashMap;
use std::ffi::{CStr, c_int, c_void};
use std::ptr::NonNull;
use std::sync::{LazyLock, Mutex};

use bytes::Bytes;

use super::operation::{Arg, Operation, cstring};
use super::{VipsError, ffi};

// ---------------------------------------------------------------------------
// Enums (values from vips/image.h; stable across libvips 8.x)
// ---------------------------------------------------------------------------

/// `VipsBandFormat`.
pub mod band_format {
    pub const UCHAR: i32 = 0;
    pub const CHAR: i32 = 1;
    pub const USHORT: i32 = 2;
    pub const SHORT: i32 = 3;
    pub const UINT: i32 = 4;
    pub const INT: i32 = 5;
    pub const FLOAT: i32 = 6;
    pub const COMPLEX: i32 = 7;
    pub const DOUBLE: i32 = 8;
    pub const DPCOMPLEX: i32 = 9;
}

/// `VipsInterpretation`.
pub mod interpretation {
    pub const MULTIBAND: i32 = 0;
    pub const B_W: i32 = 1;
    pub const CMYK: i32 = 15;
    pub const LABS: i32 = 21;
    pub const SRGB: i32 = 22;
    pub const RGB16: i32 = 25;
    pub const GREY16: i32 = 26;
}

/// `VipsAngle`, as the nicks the `rot` operation takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Angle {
    D0,
    D90,
    D180,
    D270,
}

impl Angle {
    fn nick(self) -> &'static str {
        match self {
            Angle::D0 => "d0",
            Angle::D90 => "d90",
            Angle::D180 => "d180",
            Angle::D270 => "d270",
        }
    }
}

/// `VipsDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Horizontal,
    Vertical,
}

// ---------------------------------------------------------------------------
// Image
// ---------------------------------------------------------------------------

/// One reference to a `VipsImage`. Cloning takes another reference.
///
/// `Send` because GObject reference counting is atomic and a built libvips
/// image is immutable: operations on it may run on any thread. Not `Sync`:
/// the one mutable part (metadata) is only ever touched through
/// [`Image::mutate`], on a private copy, by a single owner.
pub struct Image {
    ptr: NonNull<ffi::VipsImage>,
}

// SAFETY: see the type documentation.
unsafe impl Send for Image {}

impl Clone for Image {
    fn clone(&self) -> Self {
        // SAFETY: we hold a reference, so the object is alive.
        unsafe { ffi::g_object_ref(self.ptr.as_ptr().cast()) };
        Image { ptr: self.ptr }
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        // SAFETY: releases the reference this value owns.
        unsafe { ffi::g_object_unref(self.ptr.as_ptr().cast()) };
    }
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image({}x{}x{})", self.width(), self.height(), self.bands())
    }
}

impl Image {
    /// Takes over a reference the caller owns.
    pub(crate) fn from_owned(ptr: NonNull<ffi::VipsImage>) -> Image {
        Image { ptr }
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::VipsImage {
        self.ptr.as_ptr()
    }

    // --- header ---------------------------------------------------------------

    pub fn width(&self) -> i32 {
        // SAFETY: live image; plain accessor.
        unsafe { ffi::vips_image_get_width(self.as_ptr()) }
    }

    pub fn height(&self) -> i32 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_height(self.as_ptr()) }
    }

    pub fn bands(&self) -> i32 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_bands(self.as_ptr()) }
    }

    /// `VipsBandFormat` (see [`band_format`]).
    pub fn format(&self) -> i32 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_format(self.as_ptr()) }
    }

    /// `VipsInterpretation` (see [`interpretation`]).
    pub fn interpretation(&self) -> i32 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_interpretation(self.as_ptr()) }
    }

    /// Horizontal resolution in pixels per millimetre.
    pub fn xres(&self) -> f64 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_xres(self.as_ptr()) }
    }

    pub fn has_alpha(&self) -> bool {
        // SAFETY: as above.
        unsafe { ffi::vips_image_hasalpha(self.as_ptr()) != 0 }
    }

    /// `vips_image_get_page_height`: the `page-height` field when it divides
    /// the height evenly, the whole height otherwise.
    pub fn page_height(&self) -> i32 {
        // SAFETY: as above.
        unsafe { ffi::vips_image_get_page_height(self.as_ptr()) }
    }

    pub fn is_sequential(&self) -> bool {
        // SAFETY: as above.
        unsafe { ffi::vips_image_is_sequential(self.as_ptr()) != 0 }
    }

    /// Nick of the interpretation (`"srgb"`, `"b-w"`), as sharp reports `space`.
    pub fn interpretation_nick(&self) -> String {
        // SAFETY: vips_enum_nick returns a static string or NULL.
        unsafe { nick(ffi::vips_interpretation_get_type(), self.interpretation()) }
    }

    /// Nick of the band format (`"uchar"`), as sharp reports `depth`.
    pub fn format_nick(&self) -> String {
        // SAFETY: as above.
        unsafe { nick(ffi::vips_band_format_get_type(), self.format()) }
    }

    fn typeof_field(&self, name: &str) -> ffi::GType {
        let Ok(c) = cstring(name) else { return 0 };
        // SAFETY: live image, valid C string.
        unsafe { ffi::vips_image_get_typeof(self.as_ptr(), c.as_ptr()) }
    }

    pub fn has_field(&self, name: &str) -> bool {
        self.typeof_field(name) != 0
    }

    /// An int field, only when it is stored as an int (sharp's
    /// `get_typeof(...) == G_TYPE_INT` checks).
    pub fn get_int(&self, name: &str) -> Option<i32> {
        if self.typeof_field(name) != ffi::G_TYPE_INT {
            return None;
        }
        let c = cstring(name).ok()?;
        let mut out: c_int = 0;
        // SAFETY: field exists and is an int.
        let rc = unsafe { ffi::vips_image_get_int(self.as_ptr(), c.as_ptr(), &mut out) };
        (rc == 0).then_some(out)
    }

    /// An int field however it is stored (vips_image_get_int converts), as
    /// sharp's `ExifOrientation` reads `orientation`.
    pub fn get_int_any(&self, name: &str) -> Option<i32> {
        if !self.has_field(name) {
            return None;
        }
        let c = cstring(name).ok()?;
        let mut out: c_int = 0;
        // SAFETY: field exists; libvips converts or fails.
        let rc = unsafe { ffi::vips_image_get_int(self.as_ptr(), c.as_ptr(), &mut out) };
        (rc == 0).then_some(out)
    }

    /// A ref-string field (`heif-compression`).
    pub fn get_string(&self, name: &str) -> Option<String> {
        // SAFETY: plain type lookup.
        if self.typeof_field(name) != unsafe { ffi::vips_ref_string_get_type() } {
            return None;
        }
        let c = cstring(name).ok()?;
        let mut out: *const std::ffi::c_char = std::ptr::null();
        // SAFETY: field exists; the string is owned by the image and copied.
        unsafe {
            if ffi::vips_image_get_string(self.as_ptr(), c.as_ptr(), &mut out) != 0 || out.is_null() {
                return None;
            }
            Some(CStr::from_ptr(out).to_string_lossy().into_owned())
        }
    }

    /// An int-array field (`delay`).
    pub fn get_array_int(&self, name: &str) -> Option<Vec<i32>> {
        // SAFETY: plain type lookup.
        if self.typeof_field(name) != unsafe { ffi::vips_array_int_get_type() } {
            return None;
        }
        let c = cstring(name).ok()?;
        let mut out: *mut c_int = std::ptr::null_mut();
        let mut n: c_int = 0;
        // SAFETY: field exists; the array is owned by the image and copied.
        unsafe {
            if ffi::vips_image_get_array_int(self.as_ptr(), c.as_ptr(), &mut out, &mut n) != 0 || out.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(out, n.max(0) as usize).to_vec())
        }
    }

    /// A blob field (`exif-data`, `icc-profile-data`), copied out.
    pub fn get_blob(&self, name: &str) -> Option<Vec<u8>> {
        // SAFETY: plain type lookup.
        if self.typeof_field(name) != unsafe { ffi::vips_blob_get_type() } {
            return None;
        }
        let c = cstring(name).ok()?;
        let mut data: *const c_void = std::ptr::null();
        let mut len = 0usize;
        // SAFETY: field exists; the blob is owned by the image and copied.
        unsafe {
            if ffi::vips_image_get_blob(self.as_ptr(), c.as_ptr(), &mut data, &mut len) != 0 || data.is_null() {
                return None;
            }
            Some(std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec())
        }
    }

    /// Is this a blob field? (sharp's `HasProfile`.)
    pub fn has_blob(&self, name: &str) -> bool {
        // SAFETY: plain type lookup.
        self.typeof_field(name) == unsafe { ffi::vips_blob_get_type() }
    }

    // --- metadata writes, on a private copy only ---------------------------------

    /// Copy the image (uncached, so nobody else can be handed this object)
    /// and let `f` change its metadata. The pattern sharp spells
    /// `VImage copy = image.copy(); copy.set(...)`.
    pub fn mutate(&self, f: impl FnOnce(&mut MutableImage)) -> Result<Image, VipsError> {
        let copy = Operation::new("copy")?.set("in", self).call_uncached()?.image("out")?;
        let mut m = MutableImage { image: copy };
        f(&mut m);
        Ok(m.image)
    }

    // --- operations ---------------------------------------------------------------

    /// `vips_copy`, through the cache (a lazy reference, no pixels copied).
    pub fn copy(&self) -> Result<Image, VipsError> {
        Operation::new("copy")?.set("in", self).call()?.image("out")
    }

    /// `vips_copy` changing resolution (sharp's `SetDensity`).
    pub fn copy_with_resolution(&self, xres: f64, yres: f64) -> Result<Image, VipsError> {
        Operation::new("copy")?.set("in", self).set("xres", xres).set("yres", yres).call()?.image("out")
    }

    /// `vips_copy` changing the interpretation (raw input).
    pub fn copy_with_interpretation(&self, nick: &str) -> Result<Image, VipsError> {
        Operation::new("copy")?.set("in", self).set("interpretation", Arg::Nick(nick)).call()?.image("out")
    }

    /// `vips_image_copy_memory`: render the whole image into memory.
    pub fn copy_memory(&self) -> Result<Image, VipsError> {
        super::init()?;
        // SAFETY: returns a new reference or NULL with the error buffer set.
        let p = unsafe { ffi::vips_image_copy_memory(self.as_ptr()) };
        NonNull::new(p).map(Image::from_owned).ok_or_else(|| VipsError::from_buffer("vips_image_copy_memory"))
    }

    pub fn extract_area(&self, left: i32, top: i32, width: i32, height: i32) -> Result<Image, VipsError> {
        Operation::new("extract_area")?
            .set("input", self)
            .set("left", left)
            .set("top", top)
            .set("width", width)
            .set("height", height)
            .call()?
            .image("out")
    }

    /// `vips_extract_band(in, band)` (sharp's `image[0]`).
    pub fn extract_band(&self, band: i32, n: Option<i32>) -> Result<Image, VipsError> {
        let mut op = Operation::new("extract_band")?.set("in", self).set("band", band);
        if let Some(n) = n {
            op = op.set("n", n);
        }
        op.call()?.image("out")
    }

    /// `vips_resize(in, scale, vscale, kernel)`.
    pub fn resize(&self, scale: f64, vscale: f64, kernel: &str) -> Result<Image, VipsError> {
        Operation::new("resize")?
            .set("in", self)
            .set("scale", scale)
            .set("vscale", vscale)
            .set("kernel", Arg::Nick(kernel))
            .call()?
            .image("out")
    }

    pub fn rot(&self, angle: Angle) -> Result<Image, VipsError> {
        Operation::new("rot")?.set("in", self).set("angle", Arg::Nick(angle.nick())).call()?.image("out")
    }

    pub fn flip(&self, direction: Direction) -> Result<Image, VipsError> {
        let nick = match direction {
            Direction::Horizontal => "horizontal",
            Direction::Vertical => "vertical",
        };
        Operation::new("flip")?.set("in", self).set("direction", Arg::Nick(nick)).call()?.image("out")
    }

    /// `vips_smartcrop` with `interesting` = `"attention"` or `"entropy"`.
    /// `premultiplied` exists since libvips 8.13.
    pub fn smartcrop(
        &self,
        width: i32,
        height: i32,
        interesting: &str,
        premultiplied: bool,
    ) -> Result<Image, VipsError> {
        Operation::new("smartcrop")?
            .set("input", self)
            .set("width", width)
            .set("height", height)
            .set("interesting", Arg::Nick(interesting))
            .set_if_known("premultiplied", premultiplied)
            .call()?
            .image("out")
    }

    pub fn premultiply(&self) -> Result<Image, VipsError> {
        Operation::new("premultiply")?.set("in", self).call()?.image("out")
    }

    pub fn unpremultiply(&self) -> Result<Image, VipsError> {
        Operation::new("unpremultiply")?.set("in", self).call()?.image("out")
    }

    /// `vips_cast(in, format)` with a [`band_format`] value.
    pub fn cast(&self, format: i32) -> Result<Image, VipsError> {
        // SAFETY: static nick lookup.
        let nick = unsafe { nick(ffi::vips_band_format_get_type(), format) };
        Operation::new("cast")?.set("in", self).set("format", Arg::Nick(&nick)).call()?.image("out")
    }

    /// `vips_colourspace(in, space, source_space)` with [`interpretation`] values.
    pub fn colourspace(&self, space: i32, source_space: i32) -> Result<Image, VipsError> {
        // SAFETY: static nick lookups.
        let (to, from) = unsafe {
            (nick(ffi::vips_interpretation_get_type(), space), nick(ffi::vips_interpretation_get_type(), source_space))
        };
        Operation::new("colourspace")?
            .set("in", self)
            .set("space", Arg::Nick(&to))
            .set("source_space", Arg::Nick(&from))
            .call()?
            .image("out")
    }

    /// `vips_icc_transform` to a named output profile using the embedded one
    /// (sharp's sRGB/P3 conversion of tagged inputs).
    pub fn icc_transform_embedded(&self, output_profile: &str, depth: i32) -> Result<Image, VipsError> {
        Operation::new("icc_transform")?
            .set("in", self)
            .set("output_profile", Arg::Str(output_profile))
            .set("embedded", true)
            .set("depth", depth)
            .set("intent", Arg::Nick("perceptual"))
            .call()?
            .image("out")
    }

    /// `vips_icc_transform` from a named input profile (sharp's CMYK path).
    pub fn icc_transform_from(&self, output_profile: &str, input_profile: &str) -> Result<Image, VipsError> {
        Operation::new("icc_transform")?
            .set("in", self)
            .set("output_profile", Arg::Str(output_profile))
            .set("input_profile", Arg::Str(input_profile))
            .set("intent", Arg::Nick("perceptual"))
            .call()?
            .image("out")
    }

    /// `vips_bandjoin_const(in, [c])`.
    pub fn bandjoin_const(&self, c: &[f64]) -> Result<Image, VipsError> {
        Operation::new("bandjoin_const")?.set("in", self).set("c", c).call()?.image("out")
    }

    /// `vips_arrayjoin(images, across)`.
    pub fn arrayjoin(images: &[Image], across: i32) -> Result<Image, VipsError> {
        Operation::new("arrayjoin")?.set("in", Arg::Images(images)).set("across", across).call()?.image("out")
    }

    /// Raw pixels out (`vips_image_write_to_memory`). Evaluates the pipeline.
    pub fn write_to_memory(&self) -> Result<Vec<u8>, VipsError> {
        super::init()?;
        let mut size = 0usize;
        // SAFETY: returns g_malloc'd memory of `size` bytes or NULL with the
        // error buffer set; copied out and freed.
        unsafe {
            let p = ffi::vips_image_write_to_memory(self.as_ptr(), &mut size);
            if p.is_null() {
                return Err(VipsError::from_buffer("vips_image_write_to_memory"));
            }
            let out = std::slice::from_raw_parts(p.cast::<u8>(), size).to_vec();
            ffi::g_free(p);
            Ok(out)
        }
    }

    /// Uncompressed pixels in (sharp's `raw` input): a private copy of `data`.
    pub fn from_memory(data: &[u8], width: i32, height: i32, bands: i32, format: i32) -> Result<Image, VipsError> {
        super::init()?;
        // SAFETY: libvips copies `data`; the dimensions are checked by libvips
        // against the length.
        let p = unsafe {
            ffi::vips_image_new_from_memory_copy(data.as_ptr().cast(), data.len(), width, height, bands, format)
        };
        NonNull::new(p).map(Image::from_owned).ok_or_else(|| VipsError::from_buffer("vips_image_new_from_memory"))
    }

    // --- progress (sharp's `.timeout()`) -----------------------------------------------

    /// sharp's `SetTimeout`: listen to this image's `eval` signal and kill the
    /// evaluation once it has run for `seconds`. Libvips honours the kill
    /// only where it evaluates progressively (see transform-cost.ts on why a
    /// still AVIF encode can outlive it).
    pub fn set_timeout(&self, seconds: u32) {
        if seconds == 0 {
            return;
        }
        let state = Box::into_raw(Box::new(std::sync::atomic::AtomicI32::new(seconds as i32)));
        // SAFETY: the handler matches the `eval` signal signature
        // (VipsImage *, VipsProgress *, gpointer); `state` is freed by
        // `drop_timeout_state` when the handler goes away with the image.
        unsafe {
            let handler: unsafe extern "C" fn(*mut ffi::VipsImage, *mut ffi::VipsProgress, *mut c_void) = eval_timeout;
            ffi::g_signal_connect_data(
                self.as_ptr().cast(),
                c"eval".as_ptr(),
                std::mem::transmute::<
                    unsafe extern "C" fn(*mut ffi::VipsImage, *mut ffi::VipsProgress, *mut c_void),
                    ffi::GCallback,
                >(handler),
                state.cast(),
                Some(drop_timeout_state),
                0,
            );
            ffi::vips_image_set_progress(self.as_ptr(), ffi::GTRUE);
        }
    }

    // --- savers --------------------------------------------------------------------------

    /// Run a `*save_buffer` operation and return its `buffer`.
    pub fn save_buffer(&self, op: Operation) -> Result<Vec<u8>, VipsError> {
        op.set("in", self).call()?.bytes("buffer")
    }
}

/// `VipsProgressCallBack` from sharp's common.cc.
unsafe extern "C" fn eval_timeout(im: *mut ffi::VipsImage, progress: *mut ffi::VipsProgress, data: *mut c_void) {
    use std::sync::atomic::Ordering;
    if data.is_null() || progress.is_null() {
        return;
    }
    // SAFETY: `data` is the AtomicI32 boxed in `set_timeout`, alive until the
    // handler is destroyed; `progress` is the struct libvips passes.
    unsafe {
        let timeout = &*(data as *const std::sync::atomic::AtomicI32);
        let limit = timeout.load(Ordering::Relaxed);
        if limit > 0 && (*progress).run >= limit {
            ffi::vips_image_set_kill(im, ffi::GTRUE);
            ffi::vips_error(c"timeout".as_ptr(), c"%d%% complete".as_ptr(), (*progress).percent as c_int);
            timeout.store(0, Ordering::Relaxed);
        }
    }
}

unsafe extern "C" fn drop_timeout_state(data: *mut c_void, _closure: *mut c_void) {
    if !data.is_null() {
        // SAFETY: created by Box::into_raw in `set_timeout`, freed once.
        drop(unsafe { Box::from_raw(data as *mut std::sync::atomic::AtomicI32) });
    }
}

/// # Safety
/// `gtype` must be an enum type.
unsafe fn nick(gtype: ffi::GType, value: i32) -> String {
    // SAFETY: vips_enum_nick returns a static string (or NULL for an unknown value).
    unsafe {
        let p = ffi::vips_enum_nick(gtype, value);
        if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
    }
}

/// A freshly copied image whose metadata may be written. Only reachable
/// inside [`Image::mutate`].
pub struct MutableImage {
    image: Image,
}

impl MutableImage {
    pub fn set_int(&mut self, name: &str, value: i32) {
        if let Ok(c) = cstring(name) {
            // SAFETY: private, uncached copy; nobody else can observe it.
            unsafe { ffi::vips_image_set_int(self.image.as_ptr(), c.as_ptr(), value) };
        }
    }

    pub fn set_string(&mut self, name: &str, value: &str) {
        if let (Ok(c), Ok(v)) = (cstring(name), cstring(value)) {
            // SAFETY: as above; libvips copies the string.
            unsafe { ffi::vips_image_set_string(self.image.as_ptr(), c.as_ptr(), v.as_ptr()) };
        }
    }

    pub fn set_array_int(&mut self, name: &str, values: &[i32]) {
        if let Ok(c) = cstring(name) {
            // SAFETY: as above; libvips copies the array.
            unsafe {
                ffi::vips_image_set_array_int(self.image.as_ptr(), c.as_ptr(), values.as_ptr(), values.len() as c_int)
            };
        }
    }

    pub fn remove(&mut self, name: &str) {
        if let Ok(c) = cstring(name) {
            // SAFETY: as above.
            unsafe { ffi::vips_image_remove(self.image.as_ptr(), c.as_ptr()) };
        }
    }

    pub fn image(&self) -> &Image {
        &self.image
    }
}

// ---------------------------------------------------------------------------
// Blob: input bytes lent to libvips without a copy
// ---------------------------------------------------------------------------

/// Keeps a clone of every `Bytes` lent to libvips alive until libvips frees
/// the blob that points into it. Keyed by the data pointer; any clone sharing
/// that pointer keeps the same storage alive, so popping any one is correct.
static LENT: LazyLock<Mutex<HashMap<usize, Vec<Bytes>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "C" fn release_lent(data: *mut c_void, _area: *mut c_void) -> c_int {
    let mut lent = LENT.lock().unwrap_or_else(|e| e.into_inner());
    let key = data as usize;
    if let Some(list) = lent.get_mut(&key) {
        list.pop();
        if list.is_empty() {
            lent.remove(&key);
        }
    }
    0
}

/// A `VipsBlob` over a `Bytes`, without copying. libvips may keep the blob
/// (and so the bytes) alive after this value is dropped - inside a cached
/// loader, or a lazily evaluated image - and the bytes live exactly as long
/// as it does.
pub struct Blob {
    ptr: NonNull<ffi::VipsBlob>,
}

// SAFETY: VipsArea reference counting is thread-safe (it takes a lock).
unsafe impl Send for Blob {}

impl Blob {
    pub fn from_bytes(bytes: &Bytes) -> Result<Blob, VipsError> {
        if bytes.is_empty() {
            return Err(VipsError::new("Input Buffer is empty"));
        }
        super::init()?;
        let data = bytes.as_ptr();
        LENT.lock().unwrap_or_else(|e| e.into_inner()).entry(data as usize).or_default().push(bytes.clone());
        // SAFETY: `data` stays valid until `release_lent` runs, which libvips
        // calls exactly once when the last reference to the blob goes.
        let p = unsafe { ffi::vips_blob_new(Some(release_lent), data.cast(), bytes.len()) };
        NonNull::new(p).map(|ptr| Blob { ptr }).ok_or_else(|| VipsError::new("vips_blob_new failed"))
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::VipsBlob {
        self.ptr.as_ptr()
    }
}

impl Drop for Blob {
    fn drop(&mut self) {
        // SAFETY: releases the reference vips_blob_new gave us.
        unsafe { ffi::vips_area_unref(self.ptr.as_ptr()) };
    }
}
