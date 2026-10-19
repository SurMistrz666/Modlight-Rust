//! Calling any libvips operation by name: the Rust twin of vips8's
//! `VImage::call` (which is what sharp's C++ uses for every step).
//!
//! An [`Operation`] is built by nickname (`"resize"`, `"webpsave_buffer"`),
//! given its arguments, built through the libvips operation cache, and then
//! asked for its outputs. Argument types are checked against the operation's
//! own `GParamSpec` before anything reaches GObject, so a wrong type is a
//! `VipsError` naming the argument, never undefined behaviour or a GLib
//! warning on stderr.

use std::ffi::{CStr, CString, c_int, c_void};
use std::ptr::{self, NonNull};

use super::{Blob, Image, VipsError, ffi};

/// One argument value. Enums and flags travel as their libvips nick
/// (`"lanczos3"`, `"sequential"`, `"none"`), which is what the vips CLI and
/// every binding accept and what reads naturally next to sharp's source.
pub enum Arg<'a> {
    Int(i32),
    Bool(bool),
    Double(f64),
    Str(&'a str),
    /// An enum or flags value by nick.
    Nick(&'a str),
    Image(&'a Image),
    Images(&'a [Image]),
    Blob(&'a Blob),
    Doubles(&'a [f64]),
    Ints(&'a [i32]),
}

impl From<i32> for Arg<'_> {
    fn from(v: i32) -> Self {
        Arg::Int(v)
    }
}

impl From<bool> for Arg<'_> {
    fn from(v: bool) -> Self {
        Arg::Bool(v)
    }
}

impl From<f64> for Arg<'_> {
    fn from(v: f64) -> Self {
        Arg::Double(v)
    }
}

impl<'a> From<&'a Image> for Arg<'a> {
    fn from(v: &'a Image) -> Self {
        Arg::Image(v)
    }
}

impl<'a> From<&'a Blob> for Arg<'a> {
    fn from(v: &'a Blob) -> Self {
        Arg::Blob(v)
    }
}

impl<'a> From<&'a [f64]> for Arg<'a> {
    fn from(v: &'a [f64]) -> Self {
        Arg::Doubles(v)
    }
}

/// An operation being prepared. Setters record the first error and turn
/// the rest of the chain into no-ops, so a call reads as one expression and
/// [`Operation::call`] reports what went wrong.
pub struct Operation {
    op: *mut ffi::VipsOperation,
    name: String,
    error: Option<VipsError>,
}

impl Operation {
    /// `vips_operation_new(nickname)`.
    pub fn new(nickname: &str) -> Result<Operation, VipsError> {
        super::init()?;
        let c = cstring(nickname)?;
        // SAFETY: libvips is initialised; the name is a valid C string.
        let op = unsafe { ffi::vips_operation_new(c.as_ptr()) };
        if op.is_null() {
            return Err(VipsError::from_buffer(&format!("VipsOperation: class \"{nickname}\" not found")));
        }
        Ok(Operation { op, name: nickname.to_string(), error: None })
    }

    /// Set a required or optional argument. An unknown argument name is an
    /// error: it is a typo, or a libvips too old for this code.
    pub fn set<'a>(mut self, name: &str, value: impl Into<Arg<'a>>) -> Operation {
        if self.error.is_none()
            && let Err(e) = self.try_set(name, value.into(), false)
        {
            self.error = Some(e);
        }
        self
    }

    /// Set an argument that only newer libvips releases have. Skipped
    /// silently when this libvips does not know it; use it only where the
    /// value passed equals what older releases did anyway (the call site says
    /// which release added it).
    pub fn set_if_known<'a>(mut self, name: &str, value: impl Into<Arg<'a>>) -> Operation {
        if self.error.is_none()
            && let Err(e) = self.try_set(name, value.into(), true)
        {
            self.error = Some(e);
        }
        self
    }

    /// Build through the operation cache (`vips_cache_operation_buildp`),
    /// exactly as vips8 and therefore sharp do.
    pub fn call(self) -> Result<Built, VipsError> {
        self.build(true)
    }

    /// Build WITHOUT the operation cache. Used for the one case where the
    /// output is about to be mutated (metadata on a private copy): a cached
    /// output may be handed to the next identical call, so it must never be
    /// written to.
    pub fn call_uncached(self) -> Result<Built, VipsError> {
        self.build(false)
    }

    fn build(mut self, cached: bool) -> Result<Built, VipsError> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        let mut op = std::mem::replace(&mut self.op, ptr::null_mut());
        // SAFETY: `op` is a live, unbuilt operation we own one reference to.
        // On failure the documented cleanup is unref_outputs + unref.
        unsafe {
            let failed = if cached {
                ffi::vips_cache_operation_buildp(&mut op) != 0
            } else {
                ffi::vips_object_build(op.cast()) != 0
            };
            if failed {
                let err = VipsError::from_buffer(&self.name);
                ffi::vips_object_unref_outputs(op.cast());
                ffi::g_object_unref(op.cast());
                return Err(err);
            }
        }
        Ok(Built { op, name: std::mem::take(&mut self.name) })
    }

    fn try_set(&mut self, name: &str, value: Arg<'_>, optional: bool) -> Result<(), VipsError> {
        let cname = cstring(name)?;
        let Some(value_type) = self.argument_type(&cname) else {
            if optional {
                return Ok(());
            }
            return Err(VipsError::new(format!("{}: no argument named {name}", self.name)));
        };
        let mismatch = || VipsError::new(format!("{}: wrong value type for {name}", self.name));
        // SAFETY: every branch initialises a zeroed GValue with the type the
        // operation declares for this argument (checked first), hands it to
        // GObject, and unsets it. Boxed and object values are copied or
        // reffed by g_value_set_*, so the caller keeps its own references.
        unsafe {
            let fundamental = ffi::g_type_fundamental(value_type);
            match value {
                Arg::Int(v) => {
                    if fundamental != ffi::G_TYPE_INT {
                        return Err(mismatch());
                    }
                    self.with_gvalue(&cname, ffi::G_TYPE_INT, |gv| ffi::g_value_set_int(gv, v));
                }
                Arg::Bool(v) => {
                    if fundamental != ffi::G_TYPE_BOOLEAN {
                        return Err(mismatch());
                    }
                    self.with_gvalue(&cname, ffi::G_TYPE_BOOLEAN, |gv| ffi::g_value_set_boolean(gv, v as c_int));
                }
                Arg::Double(v) => {
                    if fundamental != ffi::G_TYPE_DOUBLE {
                        return Err(mismatch());
                    }
                    self.with_gvalue(&cname, ffi::G_TYPE_DOUBLE, |gv| ffi::g_value_set_double(gv, v));
                }
                Arg::Str(v) => {
                    if fundamental != ffi::G_TYPE_STRING {
                        return Err(mismatch());
                    }
                    let s = cstring(v)?;
                    self.with_gvalue(&cname, ffi::G_TYPE_STRING, |gv| ffi::g_value_set_string(gv, s.as_ptr()));
                }
                Arg::Nick(v) => {
                    if fundamental != ffi::G_TYPE_ENUM && fundamental != ffi::G_TYPE_FLAGS {
                        return Err(mismatch());
                    }
                    let s = cstring(v)?;
                    if ffi::vips_object_set_argument_from_string(self.op.cast(), cname.as_ptr(), s.as_ptr()) != 0 {
                        return Err(VipsError::from_buffer(&self.name));
                    }
                }
                Arg::Image(img) => {
                    if value_type != ffi::vips_image_get_type() {
                        return Err(mismatch());
                    }
                    self.with_gvalue(&cname, value_type, |gv| ffi::g_value_set_object(gv, img.as_ptr().cast()));
                }
                Arg::Images(images) => {
                    if value_type != ffi::vips_array_image_get_type() {
                        return Err(mismatch());
                    }
                    let mut ptrs: Vec<*mut ffi::VipsImage> = images.iter().map(|i| i.as_ptr()).collect();
                    let n = c_int::try_from(ptrs.len()).map_err(|_| mismatch())?;
                    let area = ffi::vips_array_image_new(ptrs.as_mut_ptr(), n);
                    self.with_gvalue(&cname, value_type, |gv| ffi::g_value_set_boxed(gv, area as *const c_void));
                    ffi::vips_area_unref(area);
                }
                Arg::Blob(blob) => {
                    if value_type != ffi::vips_blob_get_type() {
                        return Err(mismatch());
                    }
                    self.with_gvalue(&cname, value_type, |gv| {
                        ffi::g_value_set_boxed(gv, blob.as_ptr() as *const c_void)
                    });
                }
                Arg::Doubles(values) => {
                    if value_type != ffi::vips_array_double_get_type() {
                        return Err(mismatch());
                    }
                    let n = c_int::try_from(values.len()).map_err(|_| mismatch())?;
                    let area = ffi::vips_array_double_new(values.as_ptr(), n);
                    self.with_gvalue(&cname, value_type, |gv| ffi::g_value_set_boxed(gv, area as *const c_void));
                    ffi::vips_area_unref(area);
                }
                Arg::Ints(values) => {
                    if value_type != ffi::vips_array_int_get_type() {
                        return Err(mismatch());
                    }
                    let n = c_int::try_from(values.len()).map_err(|_| mismatch())?;
                    let area = ffi::vips_array_int_new(values.as_ptr(), n);
                    self.with_gvalue(&cname, value_type, |gv| ffi::g_value_set_boxed(gv, area as *const c_void));
                    ffi::vips_area_unref(area);
                }
            }
        }
        Ok(())
    }

    /// The declared type of an argument, or `None` when this operation has no
    /// such property. Looked up on the class directly, so a miss leaves the
    /// (process-wide) libvips error buffer untouched.
    fn argument_type(&self, name: &CStr) -> Option<ffi::GType> {
        // SAFETY: a GObject instance starts with its class pointer
        // (`GTypeInstance`), and `g_object_class_find_property` returns a
        // borrowed pspec or NULL.
        unsafe {
            let class = (*(self.op as *const ffi::GTypeInstance)).g_class;
            let pspec = ffi::g_object_class_find_property(class, name.as_ptr());
            if pspec.is_null() { None } else { Some((*pspec).value_type) }
        }
    }

    /// # Safety
    /// `fill` must store a value of type `gtype` into the GValue it is given.
    unsafe fn with_gvalue(&mut self, name: &CStr, gtype: ffi::GType, fill: impl FnOnce(*mut ffi::GValue)) {
        let mut gv = ffi::GValue::zeroed();
        // SAFETY: zeroed GValue, initialised, filled, handed over, unset.
        unsafe {
            ffi::g_value_init(&mut gv, gtype);
            fill(&mut gv);
            ffi::g_object_set_property(self.op.cast(), name.as_ptr(), &gv);
            ffi::g_value_unset(&mut gv);
        }
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.op.is_null() {
            // SAFETY: an operation that was never built; we own one ref.
            unsafe {
                ffi::vips_object_unref_outputs(self.op.cast());
                ffi::g_object_unref(self.op.cast());
            }
        }
    }
}

/// A built operation, holding its outputs until dropped.
pub struct Built {
    op: *mut ffi::VipsOperation,
    name: String,
}

impl Built {
    /// An output image (`"out"` for nearly every operation). The returned
    /// image holds its own reference.
    pub fn image(&self, name: &str) -> Result<Image, VipsError> {
        let cname = cstring(name)?;
        // SAFETY: GValue of the image type filled by GObject; the object it
        // holds is reffed for the returned `Image` before the GValue is unset.
        unsafe {
            let mut gv = ffi::GValue::zeroed();
            ffi::g_value_init(&mut gv, ffi::vips_image_get_type());
            ffi::g_object_get_property(self.op.cast(), cname.as_ptr(), &mut gv);
            let obj = ffi::g_value_get_object(&gv);
            let image = NonNull::new(obj.cast::<ffi::VipsImage>()).map(|p| {
                ffi::g_object_ref(p.as_ptr().cast());
                Image::from_owned(p)
            });
            ffi::g_value_unset(&mut gv);
            image.ok_or_else(|| VipsError::new(format!("{}: no output {name}", self.name)))
        }
    }

    /// An output blob (`"buffer"` for the `*save_buffer` operations), copied
    /// into Rust-owned memory.
    pub fn bytes(&self, name: &str) -> Result<Vec<u8>, VipsError> {
        let cname = cstring(name)?;
        // SAFETY: GValue of the blob type; the data pointer is valid while the
        // boxed value is held, and is copied out before the GValue is unset.
        unsafe {
            let mut gv = ffi::GValue::zeroed();
            ffi::g_value_init(&mut gv, ffi::vips_blob_get_type());
            ffi::g_object_get_property(self.op.cast(), cname.as_ptr(), &mut gv);
            let blob = ffi::g_value_get_boxed(&gv).cast::<ffi::VipsBlob>();
            let out = if blob.is_null() {
                None
            } else {
                let mut len = 0usize;
                let data = ffi::vips_blob_get(blob, &mut len);
                Some(if data.is_null() || len == 0 {
                    Vec::new()
                } else {
                    std::slice::from_raw_parts(data.cast::<u8>(), len).to_vec()
                })
            };
            ffi::g_value_unset(&mut gv);
            out.ok_or_else(|| VipsError::new(format!("{}: no output {name}", self.name)))
        }
    }

    /// An optional integer output (`attention_x` of smartcrop).
    pub fn int(&self, name: &str) -> Result<i32, VipsError> {
        let cname = cstring(name)?;
        // SAFETY: plain int GValue.
        unsafe {
            let mut gv = ffi::GValue::zeroed();
            ffi::g_value_init(&mut gv, ffi::G_TYPE_INT);
            ffi::g_object_get_property(self.op.cast(), cname.as_ptr(), &mut gv);
            let v = ffi::g_value_get_int(&gv);
            ffi::g_value_unset(&mut gv);
            Ok(v)
        }
    }
}

impl Drop for Built {
    fn drop(&mut self) {
        // SAFETY: the documented release order for a built operation: drop
        // the refs it holds on its outputs, then our ref on it (the cache may
        // keep its own).
        unsafe {
            ffi::vips_object_unref_outputs(self.op.cast());
            ffi::g_object_unref(self.op.cast());
        }
    }
}

pub(crate) fn cstring(s: &str) -> Result<CString, VipsError> {
    CString::new(s).map_err(|_| VipsError::new(format!("argument contains a NUL byte: {s:?}")))
}
