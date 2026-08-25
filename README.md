# FlowVid desktop libmpv builds

Public build and release source for the LGPL libmpv binaries used by FlowVid Desktop on Windows and
Linux. The Windows build consumes the pinned
[`flowvid-mpv-winbuild-cmake`](https://github.com/flow-vid/flowvid-mpv-winbuild-cmake) recipe. The
Linux build records its source and dependency pins in the release manifest.

## Published artifacts

- Windows: `libmpv-2.dll` archives created by the manual `mpv.yml` workflow.
- Linux: immutable release
  [`linux-lgpl-mpv-v0.41.0-ffmpeg-n8.1.2-r2`](../../releases/tag/linux-lgpl-mpv-v0.41.0-ffmpeg-n8.1.2-r2).
- Tauri integration: the MPL-2.0 wrapper under [`tauri-plugin-libmpv`](tauri-plugin-libmpv).

FlowVid Desktop verifies the expected SHA-256 before packaging a downloaded native library. Release
workflows create populated drafts and publish immutable releases only after artifact validation.
Third-party GitHub Actions are pinned to full commit SHAs.

## Windows build

The Windows toolchain is intentionally manual because it is expensive and should only run for a
reviewed source update:

1. Run `llvm.yml`.
2. Run `toolchain.yml`.
3. Run `mpv.yml` with `lgpl=true`, `compiler=clang` and `build_target=64bit`.
4. Enable the release input only after reviewing the build inputs and generated artifact.

The recipe revision is pinned inside the workflow. Its headline mpv and FFmpeg revisions are
documented in the recipe repository's `VERSIONS.md`.

## Licensing

The media build uses mpv with `-Dgpl=false` and FFmpeg without GPL or nonfree components. See
[`LICENSE-NOTES.md`](LICENSE-NOTES.md) for the exact exclusions and redistribution obligations.
The Tauri wrapper is MPL-2.0 and keeps its own source and notices in this repository.

This repository is derived from [`zhongfly/mpv-winbuild`](https://github.com/zhongfly/mpv-winbuild)
and preserves the relevant upstream history.
