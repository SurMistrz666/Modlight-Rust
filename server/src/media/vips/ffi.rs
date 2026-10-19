//! Raw declarations of the parts of GLib, GObject and libvips the binding
//! uses. Hand-written on purpose (no bindgen): every item here is either a
//! function with a plain C signature, or one of the three public structs whose
//! layout GLib and libvips have kept stable for over a decade (`GValue`,
//! `VipsProgress`, and the opaque handles that are only ever used through
//! pointers).
//!
//! Nothing outside `media::vips` may use this module; the safe wrappers in
//! `operation.rs` and `image.rs` are the only callers.

#![allow(non_camel_case_types, dead_code)]

use std::ffi::{c_char, c_double, c_int, c_uint, c_ulong, c_void};

/// `GType` is a `gsize`.
pub type GType = usize;
pub type gboolean = c_int;
pub type gpointer = *mut c_void;

pub const GFALSE: gboolean = 0;
pub const GTRUE: gboolean = 1;

// Fundamental types: `G_TYPE_MAKE_FUNDAMENTAL(n)` is `n << 2`.
pub const G_TYPE_BOOLEAN: GType = 5 << 2;
pub const G_TYPE_INT: GType = 6 << 2;
pub const G_TYPE_ENUM: GType = 12 << 2;
pub const G_TYPE_FLAGS: GType = 13 << 2;
pub const G_TYPE_DOUBLE: GType = 15 << 2;
pub const G_TYPE_STRING: GType = 16 << 2;
pub const G_TYPE_BOXED: GType = 18 << 2;
pub const G_TYPE_OBJECT: GType = 20 << 2;

/// `GLogLevelFlags`: G_LOG_LEVEL_WARNING.
pub const G_LOG_LEVEL_WARNING: c_int = 1 << 4;

/// The public `GValue` layout: the type, then two 8-byte data words.
/// Always zero-initialised before `g_value_init`, always `g_value_unset` after.
#[repr(C)]
pub struct GValue {
    pub g_type: GType,
    pub data: [u64; 2],
}

impl GValue {
    pub const fn zeroed() -> GValue {
        GValue { g_type: 0, data: [0, 0] }
    }
}

/// `VipsProgress` (vips/image.h), handed to the `eval` signal.
#[repr(C)]
pub struct VipsProgress {
    pub im: *mut VipsImage,
    pub run: c_int,
    pub eta: c_int,
    pub tpels: i64,
    pub npels: i64,
    pub percent: c_int,
    pub start: *mut c_void,
}

// Opaque handles, only ever behind pointers.
#[repr(C)]
pub struct VipsImage {
    _private: [u8; 0],
}
#[repr(C)]
pub struct VipsOperation {
    _private: [u8; 0],
}
#[repr(C)]
pub struct VipsObject {
    _private: [u8; 0],
}
#[repr(C)]
pub struct VipsArea {
    _private: [u8; 0],
}
/// The public head of `GParamSpec` (gobject/gparam.h). Only `value_type` is
/// read, to check an argument's type before a `GValue` is handed over.
#[repr(C)]
pub struct GParamSpec {
    pub g_type_instance: *mut c_void,
    pub name: *const c_char,
    pub flags: c_int,
    pub value_type: GType,
    pub owner_type: GType,
}

/// `GTypeInstance`: every GObject starts with a pointer to its class.
#[repr(C)]
pub struct GTypeInstance {
    pub g_class: *mut c_void,
}

pub type VipsBlob = VipsArea;
pub type VipsArrayDouble = VipsArea;
pub type VipsArrayInt = VipsArea;
pub type VipsArrayImage = VipsArea;

/// `int (*VipsCallbackFn)(void *a, void *b)`.
pub type VipsCallbackFn = unsafe extern "C" fn(a: *mut c_void, b: *mut c_void) -> c_int;
/// `void (*GLogFunc)(const gchar *domain, GLogLevelFlags level, const gchar *message, gpointer user_data)`.
pub type GLogFunc = unsafe extern "C" fn(domain: *const c_char, level: c_int, message: *const c_char, data: gpointer);
/// `void (*GCallback)(void)`, cast to the real handler signature by GObject.
pub type GCallback = unsafe extern "C" fn();
/// `void (*GClosureNotify)(gpointer data, GClosure *closure)`.
pub type GClosureNotify = unsafe extern "C" fn(data: gpointer, closure: *mut c_void);

unsafe extern "C" {
    // --- GLib -------------------------------------------------------------
    pub fn g_free(mem: gpointer);
    pub fn g_log_set_handler(
        log_domain: *const c_char,
        log_levels: c_int,
        log_func: GLogFunc,
        user_data: gpointer,
    ) -> c_uint;

    // --- GObject ----------------------------------------------------------
    pub fn g_type_fundamental(type_id: GType) -> GType;
    pub fn g_object_class_find_property(oclass: *mut c_void, property_name: *const c_char) -> *mut GParamSpec;
    pub fn g_object_ref(object: gpointer) -> gpointer;
    pub fn g_object_unref(object: gpointer);
    pub fn g_object_set_property(object: gpointer, property_name: *const c_char, value: *const GValue);
    pub fn g_object_get_property(object: gpointer, property_name: *const c_char, value: *mut GValue);
    pub fn g_value_init(value: *mut GValue, g_type: GType) -> *mut GValue;
    pub fn g_value_unset(value: *mut GValue);
    pub fn g_value_set_int(value: *mut GValue, v: c_int);
    pub fn g_value_get_int(value: *const GValue) -> c_int;
    pub fn g_value_set_boolean(value: *mut GValue, v: gboolean);
    pub fn g_value_set_double(value: *mut GValue, v: c_double);
    pub fn g_value_set_string(value: *mut GValue, v: *const c_char);
    pub fn g_value_set_object(value: *mut GValue, v: gpointer);
    pub fn g_value_get_object(value: *const GValue) -> gpointer;
    pub fn g_value_set_boxed(value: *mut GValue, v: *const c_void);
    pub fn g_value_get_boxed(value: *const GValue) -> gpointer;
    pub fn g_signal_connect_data(
        instance: gpointer,
        detailed_signal: *const c_char,
        c_handler: GCallback,
        data: gpointer,
        destroy_data: Option<GClosureNotify>,
        connect_flags: c_int,
    ) -> c_ulong;

    // --- libvips: lifecycle and global settings -----------------------------
    pub fn vips_init(argv0: *const c_char) -> c_int;
    pub fn vips_version(flag: c_int) -> c_int;
    pub fn vips_version_string() -> *const c_char;
    pub fn vips_thread_shutdown();
    pub fn vips_concurrency_set(concurrency: c_int);
    pub fn vips_concurrency_get() -> c_int;
    pub fn vips_cache_set_max(max: c_int);
    pub fn vips_cache_set_max_mem(max_mem: usize);
    pub fn vips_cache_set_max_files(max_files: c_int);
    pub fn vips_operation_block_set(name: *const c_char, state: gboolean);
    pub fn vips_tracked_malloc(size: usize) -> *mut c_void;
    pub fn vips_tracked_free(s: *mut c_void);
    pub fn vips_tracked_get_mem() -> usize;
    pub fn vips_cache_get_size() -> c_int;

    // --- libvips: errors ------------------------------------------------------
    pub fn vips_error_buffer_copy() -> *mut c_char;
    pub fn vips_error_clear();
    pub fn vips_error(domain: *const c_char, fmt: *const c_char, ...);

    // --- libvips: operations --------------------------------------------------
    pub fn vips_operation_new(name: *const c_char) -> *mut VipsOperation;
    pub fn vips_cache_operation_buildp(operation: *mut *mut VipsOperation) -> c_int;
    pub fn vips_object_build(object: *mut VipsObject) -> c_int;
    pub fn vips_object_unref_outputs(object: *mut VipsObject);
    pub fn vips_object_set_argument_from_string(
        object: *mut VipsObject,
        name: *const c_char,
        value: *const c_char,
    ) -> c_int;
    pub fn vips_object_get_argument(
        object: *mut VipsObject,
        name: *const c_char,
        pspec: *mut *mut GParamSpec,
        argument_class: *mut *mut c_void,
        argument_instance: *mut *mut c_void,
    ) -> c_int;
    pub fn vips_foreign_is_a_buffer(loader: *const c_char, data: *const c_void, size: usize) -> gboolean;

    // --- libvips: boxed types ---------------------------------------------------
    pub fn vips_image_get_type() -> GType;
    pub fn vips_blob_get_type() -> GType;
    pub fn vips_array_double_get_type() -> GType;
    pub fn vips_array_int_get_type() -> GType;
    pub fn vips_array_image_get_type() -> GType;
    pub fn vips_ref_string_get_type() -> GType;
    pub fn vips_interpretation_get_type() -> GType;
    pub fn vips_band_format_get_type() -> GType;
    pub fn vips_blob_new(free_fn: Option<VipsCallbackFn>, data: *const c_void, length: usize) -> *mut VipsBlob;
    pub fn vips_blob_get(blob: *mut VipsBlob, length: *mut usize) -> *const c_void;
    pub fn vips_area_unref(area: *mut VipsArea);
    pub fn vips_array_double_new(array: *const c_double, n: c_int) -> *mut VipsArrayDouble;
    pub fn vips_array_int_new(array: *const c_int, n: c_int) -> *mut VipsArrayInt;
    pub fn vips_array_image_new(array: *mut *mut VipsImage, n: c_int) -> *mut VipsArrayImage;
    pub fn vips_enum_nick(enm: GType, value: c_int) -> *const c_char;

    // --- libvips: images ----------------------------------------------------------
    pub fn vips_image_get_width(image: *const VipsImage) -> c_int;
    pub fn vips_image_get_height(image: *const VipsImage) -> c_int;
    pub fn vips_image_get_bands(image: *const VipsImage) -> c_int;
    pub fn vips_image_get_format(image: *const VipsImage) -> c_int;
    pub fn vips_image_get_interpretation(image: *const VipsImage) -> c_int;
    pub fn vips_image_get_xres(image: *const VipsImage) -> c_double;
    pub fn vips_image_get_page_height(image: *mut VipsImage) -> c_int;
    pub fn vips_image_hasalpha(image: *mut VipsImage) -> gboolean;
    pub fn vips_image_is_sequential(image: *mut VipsImage) -> gboolean;
    pub fn vips_image_get_typeof(image: *const VipsImage, name: *const c_char) -> GType;
    pub fn vips_image_get_int(image: *const VipsImage, name: *const c_char, out: *mut c_int) -> c_int;
    pub fn vips_image_get_string(image: *const VipsImage, name: *const c_char, out: *mut *const c_char) -> c_int;
    pub fn vips_image_get_array_int(
        image: *mut VipsImage,
        name: *const c_char,
        out: *mut *mut c_int,
        n: *mut c_int,
    ) -> c_int;
    pub fn vips_image_get_blob(
        image: *const VipsImage,
        name: *const c_char,
        data: *mut *const c_void,
        length: *mut usize,
    ) -> c_int;
    pub fn vips_image_set_int(image: *mut VipsImage, name: *const c_char, i: c_int);
    pub fn vips_image_set_string(image: *mut VipsImage, name: *const c_char, s: *const c_char);
    pub fn vips_image_set_array_int(image: *mut VipsImage, name: *const c_char, array: *const c_int, n: c_int);
    pub fn vips_image_remove(image: *mut VipsImage, name: *const c_char) -> gboolean;
    pub fn vips_image_set_progress(image: *mut VipsImage, progress: gboolean);
    pub fn vips_image_set_kill(image: *mut VipsImage, kill: gboolean);
    pub fn vips_image_copy_memory(image: *mut VipsImage) -> *mut VipsImage;
    pub fn vips_image_new_from_memory_copy(
        data: *const c_void,
        size: usize,
        width: c_int,
        height: c_int,
        bands: c_int,
        format: c_int,
    ) -> *mut VipsImage;
    pub fn vips_image_write_to_memory(image: *mut VipsImage, size: *mut usize) -> *mut c_void;
    pub fn vips_interpretation_max_alpha(interpretation: c_int) -> c_double;
}
