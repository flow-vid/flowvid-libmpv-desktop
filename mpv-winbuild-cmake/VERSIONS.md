# FlowVid libmpv (Windows) — pinned versions

Reproducibility manifest for `FxPandaa/flowvid-mpv-winbuild-cmake` (fork of `shinchiro/mpv-winbuild-cmake`).
Used by `FxPandaa/flowvid-libmpv-windows` to build the **LGPL** `libmpv-2.dll` bundled in FlowVidPC.

## Pinned (behaviour-defining headline components)
| Component | Pin | Why |
|-----------|-----|-----|
| **mpv**    | `2339eb72767517fc5a113283939f59076946fbc1` | master ~v0.41.0. Proven-buildable with this recipe + current deps (== zhongfly's 2026-06-25 release). |
| **FFmpeg** | `b5ef72c16b697bb22b6ec979f58a1af0cc03b140` | master 8.x. **Release tags don't work**: the recipe floats its deps to latest, and an older ffmpeg release fails to compile against them (n8.0 → `libsvtav1` `enable_adaptive_quantization` API drift). So we pin a recent master SHA the recipe actually builds. |

## Intentionally rolling (do NOT naively pin)
The remaining deps (libplacebo, dav1d, libass, freetype, fribidi, harfbuzz, zlib, libpng, …) have **no
upstream release pin** in shinchiro's recipe — it is a rolling build, validated against latest-everything.
They are **tightly coupled to mpv** (e.g. mpv `v0.41.0` expects a recent libplacebo/dav1d). Pinning them to
older release tags risks breaking the build for no playback benefit. They therefore track the recipe.

This gives reproducible **playback behaviour** (the two components that define it are frozen) + TV parity,
without the fragility of fighting the recipe's rolling design. Revisit full transitive pinning only if
bit-for-bit reproducibility becomes a hard requirement (large, ongoing-maintenance task).

## LGPL
Built with `compile-lgpl-libmpv.patch` (from the CI fork): drops x264/x265 (encoders), libssh, libsrt,
libdvdnav, libdvdread, avisynth. All decoders + libass (ISC) + libplacebo remain → LGPLv2.1+, FFmpeg
statically linked LGPLv3.
