use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, RunEvent, Runtime, WindowEvent,
};

pub use models::*;

// Windows / macOS: upstream `--wid` embed via the C libmpv-wrapper.
#[cfg(all(desktop, not(target_os = "linux")))]
mod desktop;
// Linux: render-API embed (GtkGLArea under the transparent webview). Same `Mpv<R>` surface,
// selected via #[path] so the rest of the crate (commands.rs, lib.rs) is platform-agnostic.
#[cfg(all(desktop, target_os = "linux"))]
#[path = "desktop_linux.rs"]
mod desktop;
// FlowVid's own ISC-licensed mpv FFI bindings — used only by the Linux render backend.
// Replaces the LGPL-2.1 libmpv2 / libmpv2-sys crates so no LGPL code is statically linked
// into the proprietary binary (libmpv.so itself stays LGPL + dynamically linked).
#[cfg(all(desktop, target_os = "linux"))]
mod mpv_sys;
#[cfg(mobile)]
mod mobile;
// The dlopen'd C wrapper is only used by the `--wid` path (Windows/macOS).
#[cfg(all(desktop, not(target_os = "linux")))]
mod wrapper;

mod commands;
mod error;
mod models;
// `utils::get_wid` is only used by the `--wid` path (Windows/macOS/mobile); the Linux
// render backend doesn't need a native window id.
#[cfg(not(target_os = "linux"))]
mod utils;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::Mpv;
#[cfg(mobile)]
use mobile::Mpv;

pub trait MpvExt<R: Runtime> {
    fn mpv(&self) -> &Mpv<R>;
}

impl<R: Runtime, T: Manager<R>> crate::MpvExt<R> for T {
    fn mpv(&self) -> &Mpv<R> {
        self.state::<Mpv<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("libmpv")
        .invoke_handler(tauri::generate_handler![
            commands::init,
            commands::destroy,
            commands::command,
            commands::set_property,
            commands::get_property,
            commands::set_video_margin_ratio,
        ])
        .setup(|app, api| {
            unsafe {
                let locale = std::ffi::CString::new("C").unwrap();
                libc::setlocale(libc::LC_NUMERIC, locale.as_ptr());
            }

            #[cfg(mobile)]
            let mpv = mobile::init(app, api)?;
            #[cfg(desktop)]
            let mpv = desktop::init(app, api)?;
            app.manage(mpv);
            Ok(())
        })
        .on_event(|app_handle, run_event| {
            if let RunEvent::WindowEvent {
                label,
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } = run_event
            {
                let mpv_state = app_handle.state::<Mpv<R>>();

                let instance_exists = {
                    let instances_lock = match mpv_state.instances.lock() {
                        Ok(guard) => guard,
                        Err(poisoned) => {
                            log::warn!("Mutex for mpv instances was poisoned. Recovering.");
                            poisoned.into_inner()
                        }
                    };
                    instances_lock.contains_key(label)
                };

                if instance_exists {
                    api.prevent_close();

                    let app_handle_clone = app_handle.clone();
                    let window_label = label.to_string();

                    tauri::async_runtime::spawn(async move {
                        log::info!(
                            "Close requested for '{}', destroying mpv instance first...",
                            &window_label
                        );

                        if let Err(e) = app_handle_clone.mpv().destroy(&window_label) {
                            log::error!(
                                "Failed to destroy mpv for '{}': {}. Still closing.",
                                &window_label,
                                e
                            );
                        }

                        if let Some(window) = app_handle_clone.get_webview_window(&window_label) {
                            if let Err(e) = window.close() {
                                log::error!("Failed to close window '{}': {}", &window_label, e);
                            }
                        }
                    });
                }
            }
        })
        .build()
}
