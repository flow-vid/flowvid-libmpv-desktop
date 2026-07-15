# FlowVid modifications to tauri-plugin-libmpv (MPL-2.0)

This directory is a **modified** copy of [`tauri-plugin-libmpv`](https://github.com/nini22P/tauri-plugin-libmpv)
by nini22P, used under the **Mozilla Public License 2.0** (see `LICENSE`).

FlowVid ships this modified plugin inside the (closed-source) FlowVid Desktop app.
Per MPL-2.0 §3, the source of the MPL-covered files is made available here. MPL is a
file-level ("weak") copyleft: only these files are covered; it does not affect FlowVid's
own proprietary application code.

## Changes made by FlowVid

- **Added `src/desktop_linux.rs`** — a Linux backend that embeds mpv via the libmpv **render
  API** (an mpv-drawn `GtkGLArea` under the transparent WebKitGTK webview, via `GtkOverlay`)
  instead of the upstream `--wid` embed, which does not composite correctly under a transparent
  webview and is unsupported on Wayland. (This file is MPL-2.0, derived from the upstream
  `desktop.rs`.)
- **Added `src/mpv_sys.rs`** — FlowVid's own **ISC-licensed** minimal FFI bindings for the mpv
  client + render OpenGL API, declared directly from mpv's ISC C headers. Used so the LGPL-2.1
  `libmpv2` / `libmpv2-sys` crates are not linked. This single file is ISC (see its SPDX header),
  not MPL.
- **`src/lib.rs`** — `#[cfg]`-split so Windows/macOS keep the upstream `--wid` path unchanged and
  Linux uses the render backend. The public command/event surface (`init`, `destroy`, `command`,
  `set_property`, `get_property`, `set_video_margin_ratio`, and the `mpv-event-<label>` event) is
  identical, so the JS package is unchanged.
- **`Cargo.toml`** — added the Linux GTK/glib/epoxy dependencies; removed `libmpv2` /
  `libmpv2-sys`.
- **`build.rs`** — link `libmpv` on Linux (previously handled by `libmpv2-sys`).

All other files are unchanged from upstream. The full MPL-2.0 text is in `LICENSE`.

## How this is published

The FlowVid Desktop "Open-Source Licenses" screen (Settings → About) links here:
`https://github.com/FxPandaa/flowvid-libmpv-desktop/tree/main/tauri-plugin-libmpv`. That public
repo (the desktop LGPL-libmpv factory) carries this plugin's source as a subfolder, kept in sync
whenever the plugin changes — so no separate repository is needed.
