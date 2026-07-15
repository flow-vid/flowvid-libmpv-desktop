// SPDX-License-Identifier: ISC
//
// Minimal FFI bindings for the mpv client + render OpenGL API.
//
// These are FlowVid's OWN hand-written bindings, declared directly from mpv's public C
// headers (`libmpv/client.h`, `libmpv/render.h`, `libmpv/render_gl.h`), which upstream mpv
// licenses under ISC. They exist so the Linux desktop backend does not depend on the
// third-party `libmpv2` / `libmpv2-sys` crates (LGPL-2.1), which would otherwise be
// statically linked into the proprietary FlowVid binary. Only the small surface actually
// used by `desktop_linux.rs` is declared here.
//
// The library itself (libmpv.so) is LGPL and remains dynamically linked / user-replaceable;
// see src-tauri/lib/NOTICE-libmpv.txt.
//
// A bindings module intentionally declares the full API surface (constants + functions) even
// where the current backend doesn't call every item, so `dead_code` is allowed here.
#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_double, c_int, c_void};

// --- Opaque handles ---------------------------------------------------------
#[repr(C)]
pub struct mpv_handle {
    _private: [u8; 0],
}
#[repr(C)]
pub struct mpv_render_context {
    _private: [u8; 0],
}

// --- mpv_format (client.h) --------------------------------------------------
pub type mpv_format = c_int;
pub const MPV_FORMAT_NONE: mpv_format = 0;
pub const MPV_FORMAT_STRING: mpv_format = 1;
pub const MPV_FORMAT_FLAG: mpv_format = 3;
pub const MPV_FORMAT_INT64: mpv_format = 4;
pub const MPV_FORMAT_DOUBLE: mpv_format = 5;
pub const MPV_FORMAT_NODE: mpv_format = 6;

// --- mpv_event_id (client.h) ------------------------------------------------
pub type mpv_event_id = c_int;
pub const MPV_EVENT_NONE: mpv_event_id = 0;
pub const MPV_EVENT_SHUTDOWN: mpv_event_id = 1;
pub const MPV_EVENT_START_FILE: mpv_event_id = 6;
pub const MPV_EVENT_END_FILE: mpv_event_id = 7;
pub const MPV_EVENT_FILE_LOADED: mpv_event_id = 8;
pub const MPV_EVENT_IDLE: mpv_event_id = 11;
pub const MPV_EVENT_VIDEO_RECONFIG: mpv_event_id = 17;
pub const MPV_EVENT_AUDIO_RECONFIG: mpv_event_id = 18;
pub const MPV_EVENT_SEEK: mpv_event_id = 20;
pub const MPV_EVENT_PLAYBACK_RESTART: mpv_event_id = 21;
pub const MPV_EVENT_PROPERTY_CHANGE: mpv_event_id = 22;

// --- Event structs (client.h) ----------------------------------------------
#[repr(C)]
pub struct mpv_event {
    pub event_id: mpv_event_id,
    pub error: c_int,
    pub reply_userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_event_property {
    pub name: *const c_char,
    pub format: mpv_format,
    pub data: *mut c_void,
}

// --- Render API (render.h / render_gl.h) ------------------------------------
pub type mpv_render_param_type = c_int;
pub const MPV_RENDER_PARAM_INVALID: mpv_render_param_type = 0;
pub const MPV_RENDER_PARAM_API_TYPE: mpv_render_param_type = 1;
pub const MPV_RENDER_PARAM_OPENGL_INIT_PARAMS: mpv_render_param_type = 2;
pub const MPV_RENDER_PARAM_OPENGL_FBO: mpv_render_param_type = 3;
pub const MPV_RENDER_PARAM_FLIP_Y: mpv_render_param_type = 4;

#[repr(C)]
pub struct mpv_render_param {
    pub param_type: mpv_render_param_type,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_init_params {
    /// `void *(*get_proc_address)(void *ctx, const char *name)`
    pub get_proc_address:
        unsafe extern "C" fn(ctx: *mut c_void, name: *const c_char) -> *mut c_void,
    pub get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
pub struct mpv_opengl_fbo {
    pub fbo: c_int,
    pub w: c_int,
    pub h: c_int,
    pub internal_format: c_int,
}

/// Bit returned by `mpv_render_context_update` meaning a new frame is ready to draw.
pub const MPV_RENDER_UPDATE_FRAME: u64 = 1;

/// String value for the `MPV_RENDER_PARAM_API_TYPE` param (NUL-terminated).
pub const MPV_RENDER_API_TYPE_OPENGL: &[u8] = b"opengl\0";

// --- Functions (libmpv) -----------------------------------------------------
// libmpv exports a stable C ABI; link against the LGPL libmpv.so bundled in the AppImage.
#[link(name = "mpv")]
extern "C" {
    pub fn mpv_create() -> *mut mpv_handle;
    pub fn mpv_initialize(ctx: *mut mpv_handle) -> c_int;
    pub fn mpv_terminate_destroy(ctx: *mut mpv_handle);
    pub fn mpv_set_option_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    pub fn mpv_command(ctx: *mut mpv_handle, args: *mut *const c_char) -> c_int;
    pub fn mpv_set_property_string(
        ctx: *mut mpv_handle,
        name: *const c_char,
        data: *const c_char,
    ) -> c_int;
    pub fn mpv_get_property(
        ctx: *mut mpv_handle,
        name: *const c_char,
        format: mpv_format,
        data: *mut c_void,
    ) -> c_int;
    pub fn mpv_observe_property(
        ctx: *mut mpv_handle,
        reply_userdata: u64,
        name: *const c_char,
        format: mpv_format,
    ) -> c_int;
    pub fn mpv_free(data: *mut c_void);
    pub fn mpv_wait_event(ctx: *mut mpv_handle, timeout: c_double) -> *mut mpv_event;
    pub fn mpv_wakeup(ctx: *mut mpv_handle);
    pub fn mpv_error_string(error: c_int) -> *const c_char;

    pub fn mpv_render_context_create(
        res: *mut *mut mpv_render_context,
        mpv: *mut mpv_handle,
        params: *mut mpv_render_param,
    ) -> c_int;
    pub fn mpv_render_context_render(
        ctx: *mut mpv_render_context,
        params: *mut mpv_render_param,
    ) -> c_int;
    pub fn mpv_render_context_update(ctx: *mut mpv_render_context) -> u64;
    pub fn mpv_render_context_free(ctx: *mut mpv_render_context);
}
