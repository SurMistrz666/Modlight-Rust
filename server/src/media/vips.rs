//! libvips, the library sharp wraps, called directly through FFI.
//!
//! # Why a binding of our own
//!
//! Upstream decodes, resizes and encodes every picture with sharp 0.34.5,
//! which is a thin C++ layer over libvips (8.17.3 in the production image).
//! Calling libvips with the same operations and the same arguments gives the
//! same pixels, so the port binds libvips itself instead of re-implementing
//! image processing in Rust. Hand-written `extern "C"` declarations
//! (`vips/ffi.rs`), no bindgen and no extra crates; `build.rs` links
//! whatever `pkg-config --libs vips` names (Homebrew on the Mac, Debian's
//! `libvips-dev` in the image). libvips 8.15 or newer is required (`keep`,
//! the metadata argument sharp passes to every encoder).
//!
//! # Layers
//!
//! | File | What it is |
//! |---|---|
//! | `vips/ffi.rs` | raw declarations: GLib/GObject/libvips functions, `GValue`, `VipsProgress` |
//! | `vips/operation.rs` | [`Operation`]: call any libvips operation by nickname, the twin of vips8's `VImage::call` |
//! | `vips/image.rs` | [`Image`] (one owned `VipsImage` reference), [`Blob`], typed wrappers (`resize`, `rot`, `extract_area`, ...), the timeout |
//! | `vips/sharp.rs` | [`Sharp`]: sharp's pipeline (`pipeline.cc`) and `metadata()` for the options the server uses |
//!
//! Server code uses [`Sharp`] the way upstream uses `sharp(...)`:
//!
//! ```ignore
//! let webp = Sharp::new(input)
//!     .limit_input_pixels(Some(64_000_000))
//!     .animated(false)
//!     .timeout(10)
//!     .rotate()
//!     .resize(Resize::to(Some(2560), Some(2560)).fit(Fit::Inside).without_enlargement(true))
//!     .webp(WebpOptions { quality: 82, ..Default::default() })
//!     .to_buffer()          // async: runs on tokio's blocking pool
//!     .await?;
//! ```
//!
//! # Rules (safety and behaviour)
//!
//! 1. **Initialise once.** [`init`] runs `vips_init` exactly once (a
//!    `OnceLock`) and applies `lib/media/sharp-runtime.ts`: libvips threads
//!    = `runtime::limits::effective_cpu_count()`, operation cache 32 MB / 10
//!    files / 50 operations. Every entry point calls it; nothing else may
//!    call `vips_init`.
//! 2. **Everything is blocking.** A libvips call can take seconds. Async code
//!    calls the `async` methods ([`Sharp::to_buffer`], [`Sharp::metadata`]) or
//!    wraps its own work in [`run_blocking`], which runs it on tokio's
//!    blocking pool and then releases libvips' per-thread buffers
//!    (`vips_thread_shutdown`, as sharp does after every job). Never call
//!    libvips from an async task directly.
//! 3. **Ownership.** [`Image`] owns one GObject reference (clone = ref, drop
//!    = unref) and is `Send` but not `Sync`. Images are immutable; metadata
//!    is written only through [`Image::mutate`], which works on a private,
//!    uncached copy (a cached copy could be handed to the next identical
//!    call, so it must never be modified).
//! 4. **Input bytes are copied into tracked memory.** [`Blob::from_bytes`]
//!    hands libvips its own copy, allocated with `vips_tracked_malloc`. The
//!    operation cache keeps every cached operation's inputs alive, and each
//!    pipeline leaves cached operations behind (some made inside libvips);
//!    tracked memory is what the cache's 32 MB ceiling counts, so the copies
//!    it pins are evicted past that ceiling instead of accumulating.
//! 5. **Errors.** A failing call turns libvips' error buffer into a
//!    [`VipsError`] and clears it (`vips_error_buffer_copy`). The buffer is
//!    process-wide in libvips, so under concurrent failures one message may
//!    carry another job's text; nothing branches on the wording. libvips
//!    warnings (the GLib "VIPS" domain) go to `tracing` at debug level, as
//!    sharp routes them to its debuglog.
//! 6. **Loaders.** Only the loaders the production libvips (sharp-libvips
//!    1.2.4) has may run: JPEG, PNG, WebP, GIF, TIFF, HEIF/AVIF, SVG
//!    ([`sharp::BUFFER_LOADERS`]). [`init`] blocks every other
//!    `VipsForeignLoad` class (ImageMagick, PDF, JPEG XL, Ultra HDR, FITS,
//!    ...) that a Homebrew or Debian libvips may carry, so a file production
//!    refuses ("unsupported image format") is refused here too, and the
//!    larger decoder attack surface of those builds is unreachable. Their
//!    libheif can also decode HEVC (`.heic`) pixels, which sharp-libvips
//!    cannot: it reads the header and fails on the pixels, and so does this
//!    binding while [`sharp::HEVC_DECODE`] is off. Loads are always
//!    sequential, as in sharp 0.34.5 (its `sequentialRead: false` never
//!    reaches the C++ side, which looks for an `access` key).
//! 7. **Timeouts.** [`Image::set_timeout`] is sharp's `.timeout({seconds})`:
//!    an `eval` signal handler that kills the evaluation once it has run that
//!    long. libvips only checks between progress updates, so an encoder that
//!    never reports progress (a single-frame AVIF) runs to the end anyway;
//!    the transform limiter's deadline (`media::transform_queue`) is what
//!    bounds the response.
//! 8. **Types are checked.** [`Operation`] looks each argument up on the
//!    operation's class and checks its declared type before handing over a
//!    `GValue`; an unknown name or a wrong type is an error, not undefined
//!    behaviour.
//!
//! # Adding an operation
//!
//! Nothing new is declared in `ffi.rs` for an ordinary operation. Find its
//! nickname and argument names with `vips <nickname>` (or the libvips docs),
//! then add a method to `Image` in `vips/image.rs`:
//!
//! ```ignore
//! /// `vips_flatten(in, background)`.
//! pub fn flatten(&self, background: &[f64]) -> Result<Image, VipsError> {
//!     Operation::new("flatten")?
//!         .set("in", self)                 // images: &Image
//!         .set("background", background)   // arrays: &[f64]  (Arg::Ints for ints)
//!         .call()?                         // through the operation cache, like vips8
//!         .image("out")                    // outputs: .image(), .bytes(), .int()
//! }
//! ```
//!
//! Scalars are `i32`, `f64`, `bool` (`Arg::Str` for strings), enums and
//! flags are `Arg::Nick("lanczos3")`. An argument that only newer libvips
//! releases have goes through `.set_if_known(..)`, with a comment naming the
//! release, and only when the value passed equals what older releases did.
//! An encoder is an `Operation` for `*save_buffer` handed to
//! [`Image::save_buffer`]. When porting a sharp feature, copy the order and
//! the arguments from sharp's `src/pipeline.cc` into `vips/sharp.rs`; the
//! reference sources are in `_scratch/srv-library/ref-src/sharp@0.34.5`.
//!
//! # Parity, measured
//!
//! Checked on 2026-10-19 against the reference container (sharp 0.34.5 on
//! libvips 8.17.3, linux-arm64) with a corpus of 20 files: JPEG (EXIF-rotated,
//! ICC-tagged, greyscale, CMYK, a 2400 px one on the shrink-on-load path),
//! PNG with alpha, WebP, GIF and WebP animations, TIFF, AVIF, an AVIF with the
//! `avis` brand, HEIC, 1x1 and 3x2 pictures. `imageMetaFromBuffer` (size,
//! frames, blurhash), `makeThumbnail` and `encodeAvatar` gave byte-identical
//! results through this binding on Homebrew's libvips 8.18, except CMYK input:
//! a small colour shift from the ICC engine build (mean 3/255 per channel, two
//! blurhash digits). On Debian 13's libvips 8.16.1 (the same tests all pass)
//! CMYK matches too, but two of the 20 blurhashes differ in one digit (the
//! 32 px reduce of an older libvips) and one WebP differs by a few bytes
//! (older libwebp): an image built on libvips 8.17 or newer is the closest
//! match. The blurhash encoder matches the npm package on 580 reference
//! vectors (`tests/fixtures/blurhash_vectors.json`), with macOS's and glibc's
//! libm alike.
//!
//! # Not ported yet
//!
//! Used by upstream modules outside this port and left for the media agent:
//! `flatten` (convert.ts, the PDF path), `join: { animated }` (motion.ts),
//! `composite` and `text` input (card-render.ts; SVG input itself is here).
//! Used nowhere upstream: `fit: contain`, `rotate(angle)`, `flip`, `flop`,
//! `trim`, `extend`, colour operations, TIFF/JP2/JXL output, file input and
//! output.

mod ffi;
pub mod image;
pub mod operation;
pub mod sharp;

use std::ffi::{CStr, c_char, c_int};
use std::sync::OnceLock;

pub use image::{Blob, Image};
pub use operation::{Arg, Operation};
pub use sharp::{
    FailOn, Fit, GifOptions, HeifOptions, JpegOptions, Kernel, Metadata, OutputInfo, PngOptions, Position, Region,
    Resize, Sharp, WebpOptions,
};

/// What went wrong inside libvips (or before reaching it, for invalid
/// parameters, with sharp's wording).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VipsError {
    message: String,
}

impl VipsError {
    pub fn new(message: impl Into<String>) -> VipsError {
        VipsError { message: message.into() }
    }

    /// The libvips error buffer, taken and cleared; `context` when it is
    /// empty.
    pub(crate) fn from_buffer(context: &str) -> VipsError {
        // SAFETY: returns a g_malloc'd copy (and clears the buffer).
        let text = unsafe {
            let p = ffi::vips_error_buffer_copy();
            if p.is_null() {
                String::new()
            } else {
                let s = CStr::from_ptr(p).to_string_lossy().into_owned();
                ffi::g_free(p.cast());
                s
            }
        };
        let text = text.trim_end();
        if text.is_empty() { VipsError::new(format!("{context}: unknown error")) } else { VipsError::new(text) }
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    /// Did sharp's `.timeout()` stop this job?
    pub fn is_timeout(&self) -> bool {
        self.message.contains("timeout")
    }
}

impl std::fmt::Display for VipsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VipsError {}

/// The libvips loader classes production has (see rule 6). Every other
/// `VipsForeignLoad` subclass is blocked at [`init`].
const ALLOWED_LOADER_CLASSES: [&CStr; 8] = [
    c"VipsForeignLoadJpeg",
    c"VipsForeignLoadPng",
    c"VipsForeignLoadWebp",
    c"VipsForeignLoadTiff",
    c"VipsForeignLoadNsgif",
    // The giflib loader of libvips releases before 8.11.
    c"VipsForeignLoadGif",
    c"VipsForeignLoadHeif",
    c"VipsForeignLoadSvg",
];

/// Oldest libvips this binding accepts: 8.15 brought `keep`.
const MIN_VERSION: (i32, i32) = (8, 15);

static INIT: OnceLock<Result<(), String>> = OnceLock::new();

/// Initialise libvips once (`VIPS_INIT`) and apply the process-wide settings
/// of `lib/media/sharp-runtime.ts` plus the loader allowlist. Cheap after the
/// first call.
pub fn init() -> Result<(), VipsError> {
    INIT.get_or_init(|| {
        // SAFETY: runs once, before any other libvips call in this process.
        unsafe { init_once() }
    })
    .clone()
    .map_err(VipsError::new)
}

unsafe fn init_once() -> Result<(), String> {
    use crate::media::sharp_runtime;
    // SAFETY: plain libvips/GLib calls with static C strings.
    unsafe {
        if ffi::vips_init(c"modlight".as_ptr()) != 0 {
            return Err(VipsError::from_buffer("vips_init").message);
        }
        let version = (ffi::vips_version(0), ffi::vips_version(1));
        if version < MIN_VERSION {
            return Err(format!(
                "libvips {}.{} is too old: {}.{} or newer is required",
                version.0, version.1, MIN_VERSION.0, MIN_VERSION.1
            ));
        }
        ffi::g_log_set_handler(c"VIPS".as_ptr(), ffi::G_LOG_LEVEL_WARNING, log_warning, std::ptr::null_mut());

        ffi::vips_operation_block_set(c"VipsForeignLoad".as_ptr(), ffi::GTRUE);
        for class in ALLOWED_LOADER_CLASSES {
            ffi::vips_operation_block_set(class.as_ptr(), ffi::GFALSE);
        }

        // lib/media/sharp-runtime.ts: sharp.concurrency(SHARP_CONCURRENCY)
        // and sharp.cache({ memory: 32, files: 10, items: 50 }).
        ffi::vips_concurrency_set(*sharp_runtime::SHARP_CONCURRENCY as c_int);
        ffi::vips_cache_set_max_mem(sharp_runtime::CACHE_MEMORY_MB * 1_048_576);
        ffi::vips_cache_set_max_files(sharp_runtime::CACHE_FILES);
        ffi::vips_cache_set_max(sharp_runtime::CACHE_ITEMS);
    }
    Ok(())
}

unsafe extern "C" fn log_warning(_domain: *const c_char, _level: c_int, message: *const c_char, _data: ffi::gpointer) {
    if message.is_null() {
        return;
    }
    // SAFETY: GLib passes a NUL-terminated message.
    let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    tracing::debug!(target: "vips", "{text}");
}

/// The libvips version string (`8.18.7`), for the startup log.
pub fn version() -> Result<String, VipsError> {
    init()?;
    // SAFETY: static string.
    Ok(unsafe { CStr::from_ptr(ffi::vips_version_string()) }.to_string_lossy().into_owned())
}

/// Memory libvips is tracking (`sharp.cache().memory.current`, in bytes),
/// and the operations its cache holds (`sharp.cache().items.current`).
pub fn cache_usage() -> Result<(usize, i32), VipsError> {
    init()?;
    // SAFETY: plain getters.
    Ok(unsafe { (ffi::vips_tracked_get_mem(), ffi::vips_cache_get_size()) })
}

/// Threads libvips uses per operation (`sharp.concurrency()`).
pub fn concurrency() -> Result<i32, VipsError> {
    init()?;
    // SAFETY: plain getter.
    Ok(unsafe { ffi::vips_concurrency_get() })
}

/// Run libvips work on tokio's blocking pool, then release the thread's
/// libvips buffers (sharp's `vips_thread_shutdown` after every job).
pub async fn run_blocking<T, F>(f: F) -> Result<T, VipsError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, VipsError> + Send + 'static,
{
    let joined = tokio::task::spawn_blocking(move || {
        let out = f();
        // SAFETY: frees this thread's libvips-private data; libvips
        // re-creates it on the next call from this thread.
        unsafe { ffi::vips_thread_shutdown() };
        out
    })
    .await;
    match joined {
        Ok(out) => out,
        Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
        Err(_) => Err(VipsError::new("libvips job cancelled")),
    }
}
