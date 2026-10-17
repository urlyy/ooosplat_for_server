# Development

This fork provides a headless pipeline. Read [README.md](README.md) for deployment and testing.

- `src-tauri/src/`: Rust pipeline, engine adapters, automatic optimization and project files. No desktop runtime.
- `scripts/splat.py`: detached jobs, file locking, cancellation, status/watch.
- `tests/test_jobs.py`: supervision integration tests using a fake core executable.
- `config/pipeline-optimization.json`: shared quality/optimization policy.
- `engines/`: pinned engine provenance and runtime manifests.
- `scripts/build-native-linux.sh`, `scripts/native-runtime.py`, `splat`: native Ubuntu 22.04 engine builds, verification and launcher.

Use Rust 1.95.0, Python 3.10+, Node 22.13+ for engine tooling. Run the validation commands in README before submitting changes. Do not commit source videos, models, downloaded engines, job directories or build outputs. Keep upstream licensing notices. Real GPU acceptance must identify the OS, GPU/driver, engine versions, input, quality and optimization mode; mocked tests alone do not establish hardware compatibility.
