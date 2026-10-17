#!/usr/bin/env bash
set -euo pipefail
workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$workspace/scripts/native-runtime.py" verify "${OOOSPLAT_ENGINE_DIR:-$workspace/engines/native-linux}"
