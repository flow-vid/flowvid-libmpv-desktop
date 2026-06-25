# FlowVid libmpv (Windows) — license notes

This fork builds **libmpv-2.dll for Windows under LGPLv2.1+** for use in the closed-source FlowVid PC app.

- **Build is LGPL**: `compile-lgpl-libmpv.patch` adds `-Dgpl=false` to the mpv meson setup and removes all
  GPL-only FFmpeg components: encoders **x264, x265, davs2**; protocols **libssh, libsrt, libdvdnav,
  libdvdread**; filters **librubberband, libzvbi**; **avisynth**. FFmpeg is statically linked under LGPLv3.
- **No decoder is removed** — H.264/HEVC/AV1/VP9 are FFmpeg-native (LGPL), libass is ISC, libplacebo
  (HDR/scaling/deband) stays. Playback + FlowVidPC's high-quality mpv profile are unaffected.
- **Compliance**: shipped as a dynamic `libmpv-2.dll` the user can replace; corresponding source =
  this repo (CI) + `FxPandaa/flowvid-mpv-winbuild-cmake` (recipe, version-pinned — see its `VERSIONS.md`).
- **Pins**: mpv `v0.41.0`, FFmpeg `n8.0` (recipe pinned at the wired `ref:` in the workflows).

Upstreams: CI automation forked from `zhongfly/mpv-winbuild`; recipe forked from
`shinchiro/mpv-winbuild-cmake`. mpv/FFmpeg are official upstream sources, compiled by us.
