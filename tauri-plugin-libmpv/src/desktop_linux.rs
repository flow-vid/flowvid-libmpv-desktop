// SPDX-License-Identifier: MPL-2.0
//
// Linux desktop backend for tauri-plugin-libmpv.
//
// Upstream (desktop.rs) embeds mpv via `--wid`, which does not composite correctly under a
// transparent WebKitGTK webview and is unsupported on Wayland. This module instead uses the
// libmpv RENDER API: mpv draws into a `GtkGLArea` that sits *under* the transparent webview via
// a `GtkOverlay` (the Celluloid approach, proven in FlowVidPC/linux-mpv-spike). The public
// surface (`init`, `Mpv<R>`, its 6 methods, the `mpv-event-<label>` event) is identical to
// desktop.rs so commands.rs / lib.rs / the JS package are unchanged.
//
// The mpv core is thread-safe: the GTK/GL render runs on the main thread, while commands
// (spawn_blocking) and a dedicated event-loop thread drive the same `mpv_handle` via FFI.
//
// FFI is FlowVid's own ISC-licensed bindings (`crate::mpv_sys`) declared from mpv's ISC C
// headers — NOT the LGPL-2.1 `libmpv2`/`libmpv2-sys` crates — so nothing LGPL is statically
// linked into the proprietary binary. libmpv.so itself stays LGPL + dynamically linked.

use log::{error, info, warn};
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{plugin::PluginApi, AppHandle, Emitter, Manager, Runtime};

use gtk::prelude::*;

use crate::models::*;
use crate::mpv_sys as sys;
use crate::Result;

// ---------------------------------------------------------------------------
// Plugin entry
// ---------------------------------------------------------------------------

pub fn init<R: Runtime, C: serde::de::DeserializeOwned>(
    app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> crate::Result<Mpv<R>> {
    info!("Plugin registered (linux render-API backend).");
    Ok(Mpv {
        app: app.clone(),
        instances: Mutex::new(HashMap::new()),
    })
}

// ---------------------------------------------------------------------------
// A raw mpv_handle that we deliberately share across threads. libmpv guarantees
// its client API is thread-safe, so this is sound as long as the handle outlives
// every user (the owning `Mpv` is kept in the instance until `destroy`).
// ---------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct RawMpv(*mut sys::mpv_handle);
unsafe impl Send for RawMpv {}
unsafe impl Sync for RawMpv {}

/// One embedded player, keyed by window label.
pub struct LinuxInstance {
    /// The owned mpv core. Terminated in `destroy` (kept valid until then).
    raw: RawMpv,
    /// Cleared on destroy so the render callback stops touching a dead core.
    alive: Arc<AtomicBool>,
    /// Signals the event-loop thread to exit.
    shutdown: Arc<AtomicBool>,
    event_thread: Option<JoinHandle<()>>,
}

// `Mpv` holds a raw pointer so is not Send by default; sharing is sound (see RawMpv).
unsafe impl Send for LinuxInstance {}
unsafe impl Sync for LinuxInstance {}

pub struct Mpv<R: Runtime> {
    app: AppHandle<R>,
    pub instances: Mutex<HashMap<String, LinuxInstance>>,
}

// ---------------------------------------------------------------------------
// GL / render helpers (ported from linux-mpv-spike)
// ---------------------------------------------------------------------------

/// mpv's OpenGL init callback: resolve a GL symbol by name. Matches the C signature
/// `void *(*)(void *ctx, const char *name)`.
unsafe extern "C" fn gl_get_proc_address(
    _ctx: *mut c_void,
    name: *const std::os::raw::c_char,
) -> *mut c_void {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    match CStr::from_ptr(name).to_str() {
        Ok(s) => epoxy::get_proc_addr(s) as *mut c_void,
        Err(_) => std::ptr::null_mut(),
    }
}

fn current_fbo() -> i32 {
    let mut fbo: i32 = 0;
    unsafe { epoxy::GetIntegerv(epoxy::FRAMEBUFFER_BINDING, &mut fbo) };
    fbo
}

/// Load epoxy's GL symbols exactly once for the whole process.
fn ensure_epoxy_loaded() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let lib = libloading::Library::new("libepoxy.so.0")
            .or_else(|_| libloading::Library::new("libepoxy.so"));
        if let Ok(lib) = lib {
            epoxy::load_with(|name| {
                lib.get::<*const c_void>(name.as_bytes())
                    .map(|s| *s.into_raw() as *const _)
                    .unwrap_or(std::ptr::null())
            });
            std::mem::forget(lib);
        } else {
            error!("Failed to load libepoxy — mpv render will not work.");
        }
    });
}

/// Owns an `mpv_render_context`. Created on the GTK main thread; kept alive for the window's
/// lifetime (leaked via `mem::forget` in setup_render, so `Drop` never actually runs — the
/// core is torn down wholesale in `destroy`).
struct RenderCtx(*mut sys::mpv_render_context);

impl RenderCtx {
    /// Create an OpenGL render context bound to `mpv`.
    fn new(mpv: *mut sys::mpv_handle) -> std::result::Result<Self, i32> {
        // The API-type string must stay valid across the create call.
        let mut init = sys::mpv_opengl_init_params {
            get_proc_address: gl_get_proc_address,
            get_proc_address_ctx: std::ptr::null_mut(),
        };
        let mut params = [
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_API_TYPE,
                data: sys::MPV_RENDER_API_TYPE_OPENGL.as_ptr() as *mut c_void,
            },
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                data: &mut init as *mut _ as *mut c_void,
            },
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        let mut ctx: *mut sys::mpv_render_context = std::ptr::null_mut();
        let ret =
            unsafe { sys::mpv_render_context_create(&mut ctx, mpv, params.as_mut_ptr()) };
        if ret < 0 || ctx.is_null() {
            return Err(ret);
        }
        Ok(RenderCtx(ctx))
    }

    /// Render one frame into the given FBO. `flip_y` accounts for GL's bottom-left origin.
    fn render(&self, fbo: i32, w: i32, h: i32, flip_y: bool) {
        let mut fbo_p = sys::mpv_opengl_fbo {
            fbo,
            w,
            h,
            internal_format: 0,
        };
        let mut flip: std::os::raw::c_int = if flip_y { 1 } else { 0 };
        let mut params = [
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_OPENGL_FBO,
                data: &mut fbo_p as *mut _ as *mut c_void,
            },
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_FLIP_Y,
                data: &mut flip as *mut _ as *mut c_void,
            },
            sys::mpv_render_param {
                param_type: sys::MPV_RENDER_PARAM_INVALID,
                data: std::ptr::null_mut(),
            },
        ];
        unsafe {
            sys::mpv_render_context_render(self.0, params.as_mut_ptr());
        }
    }

    /// Poll for a pending update; returns the render-update flags bitset.
    fn update(&self) -> u64 {
        unsafe { sys::mpv_render_context_update(self.0) }
    }
}

// ---------------------------------------------------------------------------
// mpv raw control helpers
// ---------------------------------------------------------------------------

/// Properties mpv updates continuously (roughly once per frame) whose consumers only need a
/// few updates per second. These get rate-limited in the event thread; all other properties
/// pass through immediately.
fn is_high_frequency(name: &str) -> bool {
    matches!(
        name,
        "time-pos"
            | "playback-time"
            | "percent-pos"
            | "time-remaining"
            | "video-bitrate"
            | "audio-bitrate"
            | "estimated-vf-fps"
            | "avsync"
    )
}

/// Read an mpv property as a string on a raw handle (best-effort; "?" on error). Diagnostics only.
fn raw_get_string(handle: *mut sys::mpv_handle, name: &str) -> String {
    let cname = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return "?".into(),
    };
    unsafe {
        let mut out: *mut std::os::raw::c_char = std::ptr::null_mut();
        let ret = sys::mpv_get_property(
            handle,
            cname.as_ptr(),
            sys::MPV_FORMAT_STRING,
            &mut out as *mut _ as *mut c_void,
        );
        if ret < 0 || out.is_null() {
            return "?".into();
        }
        let s = CStr::from_ptr(out).to_string_lossy().into_owned();
        sys::mpv_free(out as *mut c_void);
        s
    }
}

fn mpv_format_from_str(fmt: &str) -> sys::mpv_format {
    match fmt {
        "string" => sys::MPV_FORMAT_STRING,
        "flag" => sys::MPV_FORMAT_FLAG,
        "int64" => sys::MPV_FORMAT_INT64,
        "double" => sys::MPV_FORMAT_DOUBLE,
        "node" => sys::MPV_FORMAT_NODE,
        _ => sys::MPV_FORMAT_STRING,
    }
}

/// Convert a serde value into the plain string mpv accepts for `mpv_set_property_string`
/// / command arguments. mpv parses these (e.g. "yes"/"no" for flags, numbers as text).
fn value_to_mpv_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Bool(b) => if *b { "yes".into() } else { "no".into() },
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

impl<R: Runtime> Mpv<R> {
    pub fn init(&self, mpv_config: MpvConfig, window_label: &str) -> Result<String> {
        {
            let instances = self.lock_instances();
            if instances.contains_key(window_label) {
                info!("mpv instance for '{}' already exists; skipping.", window_label);
                return Ok(window_label.to_string());
            }
        }

        ensure_epoxy_loaded();

        // mpv REQUIRES the C numeric locale; GTK sets the user locale (also set in lib.rs setup,
        // but re-assert here in case init runs before that on some runtimes).
        unsafe {
            let c = CString::new("C").unwrap();
            libc::setlocale(libc::LC_NUMERIC, c.as_ptr());
        }

        // Create + configure the mpv core. `vo=libmpv` and the other init-only options come from
        // the JS layer's initial_options (embeddedMpvService already sets vo=libmpv on Linux).
        let mpv_ptr = unsafe { sys::mpv_create() };
        if mpv_ptr.is_null() {
            return Err(crate::Error::FFI("mpv_create returned null".into()));
        }
        for (key, value) in &mpv_config.initial_options {
            // Set as an mpv OPTION (pre-initialize); mpv parses per the option's own type.
            if let (Ok(ckey), Ok(cval)) = (
                CString::new(key.as_str()),
                CString::new(value_to_mpv_string(value)),
            ) {
                let r =
                    unsafe { sys::mpv_set_option_string(mpv_ptr, ckey.as_ptr(), cval.as_ptr()) };
                if r < 0 {
                    warn!("mpv init option '{}' rejected: {}", key, mpv_err_str(r));
                }
            }
        }
        let r = unsafe { sys::mpv_initialize(mpv_ptr) };
        if r < 0 {
            unsafe { sys::mpv_terminate_destroy(mpv_ptr) };
            return Err(crate::Error::FFI(format!(
                "mpv_initialize failed: {}",
                mpv_err_str(r)
            )));
        }

        let raw = RawMpv(mpv_ptr);

        // Observe the requested properties on the raw handle (thread-safe).
        for (name, fmt) in &mpv_config.observed_properties {
            if let Ok(cname) = CString::new(name.as_str()) {
                unsafe {
                    sys::mpv_observe_property(raw.0, 0, cname.as_ptr(), mpv_format_from_str(fmt));
                }
            }
        }

        let alive = Arc::new(AtomicBool::new(true));
        let shutdown = Arc::new(AtomicBool::new(false));

        // Event-loop thread: pump mpv events → emit `mpv-event-<label>` to the frontend.
        let event_thread = self.spawn_event_thread(raw, window_label.to_string(), shutdown.clone());

        // Opt-in live diagnostics: FLOWVID_MPV_STATS=1 logs mpv's own frame stats every second
        // to stderr so we can SEE whether stutter is frame-drops (render can't keep up),
        // A/V-sync drift, or cache underrun — without guessing.
        if std::env::var("FLOWVID_MPV_STATS").is_ok() {
            let raw_addr = raw.0 as usize;
            let shutdown_stats = shutdown.clone();
            std::thread::spawn(move || {
                let handle = raw_addr as *mut sys::mpv_handle;
                let props = [
                    "time-pos",
                    "estimated-vf-fps",
                    "container-fps",
                    "frame-drop-count",       // frames the VO dropped (render too slow)
                    "decoder-frame-drop-count", // frames the decoder dropped
                    "vo-delayed-frame-count", // frames shown late
                    "avsync",                 // audio/video desync (s)
                    "cache-buffering-state",  // 100 = full
                    "demuxer-cache-duration", // seconds buffered ahead
                ];
                while !shutdown_stats.load(Ordering::Relaxed) {
                    let mut line = String::from("[mpv-stats]");
                    for p in props {
                        line.push(' ');
                        line.push_str(p);
                        line.push('=');
                        line.push_str(&raw_get_string(handle, p));
                    }
                    eprintln!("{line}");
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }
            });
        }

        // GTK + render setup MUST run on the main thread.
        self.setup_render(raw, alive.clone(), window_label.to_string())?;

        let instance = LinuxInstance {
            raw,
            alive,
            shutdown,
            event_thread: Some(event_thread),
        };
        self.lock_instances().insert(window_label.to_string(), instance);

        info!("Render mode initialized for '{}'.", window_label);
        Ok(window_label.to_string())
    }

    fn spawn_event_thread(
        &self,
        raw: RawMpv,
        window_label: String,
        shutdown: Arc<AtomicBool>,
    ) -> JoinHandle<()> {
        let app = self.app.clone();
        let event_name = format!("mpv-event-{}", window_label);
        // Cross the thread boundary as an integer address — a raw `*mut mpv_handle` is !Send,
        // but the mpv client API is thread-safe so re-materializing it here is sound.
        let raw_addr = raw.0 as usize;
        std::thread::spawn(move || {
            let handle = raw_addr as *mut sys::mpv_handle;
            // Coalesce high-frequency position properties. mpv fires these ~every frame
            // (~30/s); forwarding each one makes the React player UI re-render + the
            // transparent webview repaint that often, which composited over the video pins
            // the WebKitWebProcess. A seek bar only needs a handful of updates per second.
            let mut last_emit: HashMap<String, std::time::Instant> = HashMap::new();
            const THROTTLE: std::time::Duration = std::time::Duration::from_millis(200);
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }
                let ev = unsafe { sys::mpv_wait_event(handle, 0.1) };
                if ev.is_null() {
                    continue;
                }
                let event_id = unsafe { (*ev).event_id };
                if event_id == sys::MPV_EVENT_NONE {
                    continue;
                }
                if event_id == sys::MPV_EVENT_SHUTDOWN {
                    break;
                }
                if let Some(json) = unsafe { event_to_json(ev) } {
                    // Rate-limit only the continuously-changing position properties; everything
                    // else (pause, tracks, file-loaded, end-file, …) is forwarded immediately.
                    if json.get("event").and_then(|e| e.as_str()) == Some("property-change") {
                        if let Some(name) = json.get("name").and_then(|n| n.as_str()) {
                            if is_high_frequency(name) {
                                let now = std::time::Instant::now();
                                let too_soon = last_emit
                                    .get(name)
                                    .map(|t| now.duration_since(*t) < THROTTLE)
                                    .unwrap_or(false);
                                if too_soon {
                                    continue;
                                }
                                last_emit.insert(name.to_string(), now);
                            }
                        }
                    }
                    if let Err(e) = app.emit_to(&window_label, &event_name, &json) {
                        error!("emit mpv event failed: {}", e);
                    }
                }
            }
            info!("mpv event thread for '{}' exited.", window_label);
        })
    }

    /// Build the GtkOverlay { GtkGLArea (mpv) + transparent webview } and wire the render context.
    fn setup_render(&self, raw: RawMpv, alive: Arc<AtomicBool>, window_label: String) -> Result<()> {
        let window = self
            .app
            .get_webview_window(&window_label)
            .ok_or_else(|| crate::Error::WindowNotFound(window_label.clone()))?;

        // Run the GTK work on the main thread and block until it completes so init returns
        // only once the render context exists.
        let raw_addr = raw.0 as usize;
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<(), String>>();
        self.app
            .run_on_main_thread(move || {
                let res = (|| -> std::result::Result<(), String> {
                    let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
                    let vbox = window.default_vbox().map_err(|e| e.to_string())?;

                    if let Some(screen) = WidgetExt::screen(&gtk_window) {
                        if let Some(rgba) = screen.rgba_visual() {
                            gtk_window.set_visual(Some(&rgba));
                        }
                    }
                    gtk_window.set_app_paintable(true);

                    let webview_widget =
                        vbox.children().into_iter().next().ok_or("empty vbox")?;
                    vbox.remove(&webview_widget);

                    let overlay = gtk::Overlay::new();
                    let gl_area = gtk::GLArea::new();
                    gl_area.set_hexpand(true);
                    gl_area.set_vexpand(true);
                    gl_area.set_has_depth_buffer(false);
                    gl_area.set_has_stencil_buffer(false);

                    let render_cell: std::rc::Rc<std::cell::RefCell<Option<RenderCtx>>> =
                        std::rc::Rc::new(std::cell::RefCell::new(None));

                    {
                        let render_cell = render_cell.clone();
                        gl_area.connect_realize(move |area| {
                            area.make_current();
                            if render_cell.borrow().is_some() {
                                return;
                            }
                            let handle = raw_addr as *mut sys::mpv_handle;
                            match RenderCtx::new(handle) {
                                Ok(ctx) => {
                                    *render_cell.borrow_mut() = Some(ctx);
                                    info!("mpv render context created.");
                                }
                                Err(code) => {
                                    error!("mpv_render_context_create failed: {}", mpv_err_str(code))
                                }
                            }
                        });
                    }

                    {
                        let render_cell = render_cell.clone();
                        let alive = alive.clone();
                        gl_area.connect_render(move |area, _ctx| {
                            if alive.load(Ordering::Relaxed) {
                                if let Some(rctx) = render_cell.borrow().as_ref() {
                                    let scale = area.scale_factor();
                                    let w = area.allocated_width() * scale;
                                    let h = area.allocated_height() * scale;
                                    let fbo = current_fbo();
                                    rctx.render(fbo, w.max(1), h.max(1), true);
                                    return glib::Propagation::Stop;
                                }
                            }
                            unsafe {
                                epoxy::ClearColor(0.0, 0.0, 0.0, 1.0);
                                epoxy::Clear(epoxy::COLOR_BUFFER_BIT);
                            }
                            glib::Propagation::Stop
                        });
                    }

                    // Redraw ONLY when mpv reports a new frame, polled on the GTK main thread.
                    // Rendering every tick even while paused pins the CPU/GPU (very visible on
                    // software GL) — mpv_render_context_update() lets us skip idle re-renders.
                    // (mpv's own render-update callback can't be used to queue_render directly:
                    // it fires on mpv's render thread and requires a Send closure, but GTK
                    // widgets are !Send.)
                    {
                        let render_cell = render_cell.clone();
                        gl_area.add_tick_callback(move |area, _clock| {
                            if let Some(rctx) = render_cell.borrow().as_ref() {
                                let flags = rctx.update();
                                if (flags & sys::MPV_RENDER_UPDATE_FRAME) != 0 {
                                    area.queue_render();
                                }
                            }
                            glib::ControlFlow::Continue
                        });
                    }

                    // Compose: mpv (GLArea) at the bottom, the transparent webview on top.
                    overlay.add(&gl_area);
                    overlay.add_overlay(&webview_widget);
                    overlay.set_overlay_pass_through(&webview_widget, false);

                    // Make the overlay the window's DIRECT child (replacing the vbox) rather than
                    // nesting it inside the vbox. tauri-runtime-wry's undecorated-resize handler
                    // runs `webview.parent().parent().downcast::<Window>().unwrap()`, hardcoding
                    // the tree as webview → GtkBox → Window. Nesting the webview one level deeper
                    // (webview → overlay → vbox) makes the grandparent a GtkBox, so that unwrap
                    // panics (non-unwinding → abort) on the very first click — which is what
                    // crashed the app on pause. With the overlay as the window's child,
                    // webview → overlay → Window holds and the downcast succeeds.
                    gtk_window.remove(&vbox);
                    gtk_window.add(&overlay);
                    overlay.show_all();
                    gtk_window.show_all();

                    // Keep the render context alive for the window's lifetime.
                    std::mem::forget(render_cell);
                    Ok(())
                })();
                let _ = tx.send(res);
            })
            .map_err(|e| crate::Error::FFI(format!("run_on_main_thread failed: {e}")))?;

        match rx.recv() {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(crate::Error::FFI(format!("render setup failed: {e}"))),
            Err(e) => Err(crate::Error::FFI(format!("render setup channel error: {e}"))),
        }
    }

    pub fn destroy(&self, window_label: &str) -> Result<()> {
        let mut instance = match self.lock_instances().remove(window_label) {
            Some(i) => i,
            None => return Ok(()),
        };
        instance.alive.store(false, Ordering::Relaxed);
        instance.shutdown.store(true, Ordering::Relaxed);
        // Nudge the event loop so it wakes and sees `shutdown`.
        unsafe { sys::mpv_wakeup(instance.raw.0) };
        if let Some(t) = instance.event_thread.take() {
            let _ = t.join();
        }
        // Terminate the mpv core (was previously the drop of the owned libmpv2 `Mpv`).
        unsafe { sys::mpv_terminate_destroy(instance.raw.0) };
        info!("mpv instance for '{}' destroyed.", window_label);
        Ok(())
    }

    pub fn command(
        &self,
        name: &str,
        args: &Vec<serde_json::Value>,
        window_label: &str,
    ) -> Result<()> {
        let raw = self.raw_for(window_label)?;
        // Build a NULL-terminated argv: [name, arg0, arg1, ..., NULL].
        let mut owned: Vec<CString> = Vec::with_capacity(args.len() + 1);
        owned.push(CString::new(name).map_err(|e| crate::Error::FFI(e.to_string()))?);
        for a in args {
            owned.push(
                CString::new(value_to_mpv_string(a))
                    .map_err(|e| crate::Error::FFI(e.to_string()))?,
            );
        }
        let mut argv: Vec<*const std::os::raw::c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        argv.push(std::ptr::null());
        let ret = unsafe { sys::mpv_command(raw.0, argv.as_mut_ptr()) };
        check_mpv(ret, || crate::Error::Command {
            window_label: window_label.to_string(),
            message: mpv_err_str(ret),
        })
    }

    pub fn set_property(
        &self,
        name: &str,
        value: &serde_json::Value,
        window_label: &str,
    ) -> Result<()> {
        let raw = self.raw_for(window_label)?;
        let cname = CString::new(name).map_err(|e| crate::Error::FFI(e.to_string()))?;
        let cval = CString::new(value_to_mpv_string(value))
            .map_err(|e| crate::Error::FFI(e.to_string()))?;
        let ret = unsafe { sys::mpv_set_property_string(raw.0, cname.as_ptr(), cval.as_ptr()) };
        check_mpv(ret, || crate::Error::SetProperty {
            window_label: window_label.to_string(),
            message: mpv_err_str(ret),
        })
    }

    pub fn get_property(
        &self,
        name: String,
        format: String,
        window_label: &str,
    ) -> Result<serde_json::Value> {
        let raw = self.raw_for(window_label)?;
        let cname = CString::new(name.as_str()).map_err(|e| crate::Error::FFI(e.to_string()))?;
        unsafe {
            match format.as_str() {
                "flag" => {
                    let mut out: std::os::raw::c_int = 0;
                    let ret = sys::mpv_get_property(
                        raw.0,
                        cname.as_ptr(),
                        sys::MPV_FORMAT_FLAG,
                        &mut out as *mut _ as *mut c_void,
                    );
                    get_checked(ret, window_label, || serde_json::json!(out != 0))
                }
                "int64" => {
                    let mut out: i64 = 0;
                    let ret = sys::mpv_get_property(
                        raw.0,
                        cname.as_ptr(),
                        sys::MPV_FORMAT_INT64,
                        &mut out as *mut _ as *mut c_void,
                    );
                    get_checked(ret, window_label, || serde_json::json!(out))
                }
                "double" => {
                    let mut out: f64 = 0.0;
                    let ret = sys::mpv_get_property(
                        raw.0,
                        cname.as_ptr(),
                        sys::MPV_FORMAT_DOUBLE,
                        &mut out as *mut _ as *mut c_void,
                    );
                    get_checked(ret, window_label, || serde_json::json!(out))
                }
                _ => {
                    // Default to string.
                    let mut out: *mut std::os::raw::c_char = std::ptr::null_mut();
                    let ret = sys::mpv_get_property(
                        raw.0,
                        cname.as_ptr(),
                        sys::MPV_FORMAT_STRING,
                        &mut out as *mut _ as *mut c_void,
                    );
                    if ret < 0 || out.is_null() {
                        return Err(crate::Error::GetProperty {
                            window_label: window_label.to_string(),
                            message: mpv_err_str(ret),
                        });
                    }
                    let s = CStr::from_ptr(out).to_string_lossy().into_owned();
                    sys::mpv_free(out as *mut c_void);
                    Ok(serde_json::json!(s))
                }
            }
        }
    }

    pub fn set_video_margin_ratio(
        &self,
        ratio: VideoMarginRatio,
        window_label: &str,
    ) -> Result<()> {
        let margins = [
            ("video-margin-ratio-left", ratio.left),
            ("video-margin-ratio-right", ratio.right),
            ("video-margin-ratio-top", ratio.top),
            ("video-margin-ratio-bottom", ratio.bottom),
        ];
        for (prop, val) in margins {
            if let Some(v) = val {
                self.set_property(prop, &serde_json::json!(v), window_label)?;
            }
        }
        Ok(())
    }

    // -- helpers --

    fn lock_instances(&self) -> std::sync::MutexGuard<'_, HashMap<String, LinuxInstance>> {
        match self.instances.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn raw_for(&self, window_label: &str) -> Result<RawMpv> {
        self.lock_instances()
            .get(window_label)
            .map(|i| i.raw)
            .ok_or_else(|| {
                crate::Error::InstanceNotFound(format!(
                    "mpv instance for window label '{}' not found",
                    window_label
                ))
            })
    }
}

// ---------------------------------------------------------------------------
// mpv error + event helpers
// ---------------------------------------------------------------------------

fn mpv_err_str(code: std::os::raw::c_int) -> String {
    unsafe {
        let p = sys::mpv_error_string(code);
        if p.is_null() {
            format!("mpv error {}", code)
        } else {
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }
}

fn check_mpv<F: FnOnce() -> crate::Error>(ret: std::os::raw::c_int, err: F) -> Result<()> {
    if ret < 0 {
        Err(err())
    } else {
        Ok(())
    }
}

fn get_checked<F: FnOnce() -> serde_json::Value>(
    ret: std::os::raw::c_int,
    window_label: &str,
    ok: F,
) -> Result<serde_json::Value> {
    if ret < 0 {
        Err(crate::Error::GetProperty {
            window_label: window_label.to_string(),
            message: mpv_err_str(ret),
        })
    } else {
        Ok(ok())
    }
}

/// Serialize an mpv event into the JSON shape the JS package expects
/// (`{ event: "<kind>", ... }`). Only the events FlowVid consumes are given rich fields;
/// others map to a bare `{ event }`.
unsafe fn event_to_json(ev: *const sys::mpv_event) -> Option<serde_json::Value> {
    let id = (*ev).event_id;
    if id == sys::MPV_EVENT_PROPERTY_CHANGE {
        let prop = (*ev).data as *const sys::mpv_event_property;
        if prop.is_null() {
            return None;
        }
        let name = CStr::from_ptr((*prop).name).to_string_lossy().into_owned();
        let data = property_data_to_json((*prop).format, (*prop).data);
        return Some(serde_json::json!({
            "event": "property-change",
            "name": name,
            "data": data,
            "id": (*ev).reply_userdata,
        }));
    }

    let kind = match id {
        x if x == sys::MPV_EVENT_FILE_LOADED => "file-loaded",
        x if x == sys::MPV_EVENT_START_FILE => "start-file",
        x if x == sys::MPV_EVENT_END_FILE => "end-file",
        x if x == sys::MPV_EVENT_IDLE => "idle",
        x if x == sys::MPV_EVENT_PLAYBACK_RESTART => "playback-restart",
        x if x == sys::MPV_EVENT_SEEK => "seek",
        x if x == sys::MPV_EVENT_VIDEO_RECONFIG => "video-reconfig",
        x if x == sys::MPV_EVENT_AUDIO_RECONFIG => "audio-reconfig",
        _ => return None,
    };
    Some(serde_json::json!({ "event": kind }))
}

unsafe fn property_data_to_json(
    format: sys::mpv_format,
    data: *mut c_void,
) -> serde_json::Value {
    if data.is_null() {
        return serde_json::Value::Null;
    }
    if format == sys::MPV_FORMAT_FLAG {
        let v = *(data as *const std::os::raw::c_int);
        serde_json::json!(v != 0)
    } else if format == sys::MPV_FORMAT_INT64 {
        serde_json::json!(*(data as *const i64))
    } else if format == sys::MPV_FORMAT_DOUBLE {
        serde_json::json!(*(data as *const f64))
    } else if format == sys::MPV_FORMAT_STRING {
        let s = *(data as *const *const std::os::raw::c_char);
        if s.is_null() {
            serde_json::Value::Null
        } else {
            serde_json::json!(CStr::from_ptr(s).to_string_lossy().into_owned())
        }
    } else {
        serde_json::Value::Null
    }
}
