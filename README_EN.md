# Splat Server

Headless OOOSplat modification for **native Ubuntu 22.04 x86_64**, without Docker. Video ingestion, `fast` / `balanced` / `high` generation, automatic optimization, detached jobs and terminal monitoring are retained. Desktop UI, viewer/editor, MCP and telemetry are removed.

See [README.md](README.md) for prerequisites. Native build and synthetic-video end-to-end testing passed on Ubuntu 22.04.5 / RTX 5090 / CUDA Toolkit 13.0 on 2026-10-18: fast mode with optimization, 36/36 registered frames, 8,000 Vulkan training steps, and a valid final.ply. CUDA 12.8/13.2 builds and balanced/high full training have not been separately exercised. See [test record](docs/ubuntu-smoke-test.md).

```bash
# CUDA 12.8 / 13.0 / 13.2 Toolkit accepted for cuda; CPU COLMAP is also available.
bash scripts/setup-engines-linux.sh cuda  # alternatively: cpu
bash scripts/verify-engines-linux.sh
./splat health
JOB=$(./splat submit /path/to/video.mp4 --quality balanced)
./splat watch "$JOB"
./splat cancel "$JOB"
```

Both modes require a working Vulkan GPU for Brush training. The builder compiles pinned COLMAP, vcpkg dependencies, OOOBrush `brush-cli`, and the Rust core against the host ABI. No Ubuntu 24.04 runtime binaries are installed. Python uses only its standard library; Node 22.13+ supports engine checks. Rust is pinned to 1.95.0.

Native builds are installed as immutable directories under `engines/.native-release-*/`, selected by `engines/native-linux`. Jobs pin the resolved runtime path. Inputs, results and logs persist under `jobs/`. Exiting watch does not cancel a job. Interrupted jobs are not automatically resumed.

The historical `src-tauri/` path now holds a plain Rust crate with no Tauri dependency. Apache-2.0; see LICENSE, NOTICE and `licenses/`.
