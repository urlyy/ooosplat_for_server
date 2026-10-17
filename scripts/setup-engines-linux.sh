#!/usr/bin/env bash
set -euo pipefail
workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec bash "$workspace/scripts/build-native-linux.sh" "${1:-cuda}"
