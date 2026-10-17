#!/usr/bin/env bash
# Build the pinned engines against the host Ubuntu 22.04 ABI.
set -Eeuo pipefail
trap 'echo "Native build failed at line $LINENO; build cache retained for diagnosis." >&2' ERR
workspace="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
backend="${1:-cuda}"
[[ "$backend" == cuda || "$backend" == cpu ]] || { echo 'Usage: build-native-linux.sh [cuda|cpu]' >&2; exit 2; }
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || { echo 'Ubuntu 22.04 x86_64 required' >&2; exit 1; }
# shellcheck source=/dev/null
source /etc/os-release
[[ "$ID" == ubuntu && "$VERSION_ID" == 22.04 ]] || { echo 'Build on Ubuntu 22.04 itself, not a newer distribution.' >&2; exit 1; }
[[ "$EUID" != 0 || "${OOOSPLAT_ALLOW_ROOT_BUILD:-0}" == 1 ]] || { echo 'Root-only cloud instance: explicitly set OOOSPLAT_ALLOW_ROOT_BUILD=1.' >&2; exit 1; }
for command_name in python3 git curl tar cc c++ gfortran make pkg-config node cargo rustup patchelf flock; do
  command -v "$command_name" >/dev/null || { echo "Missing prerequisite: $command_name (see README.md)" >&2; exit 1; }
done
node -e 'const [a,b]=process.versions.node.split(".").map(Number); if(a<22 || (a===22 && b<13)) throw Error("Node.js 22.13+ required")'
cache="$workspace/.cache/native-linux"
mkdir -p "$cache"
exec 9>"$cache/build.lock"
flock -n 9 || { echo 'Another native build is running.' >&2; exit 1; }
read_lock() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$workspace/engines/colmap-build.json" "$1"; }
python3 -m venv "$cache/tools"
"$cache/tools/bin/pip" install --disable-pip-version-check "cmake==$(read_lock cmakeVersion)" "ninja==1.13.2"
export PATH="$cache/tools/bin:$PATH"
export VCPKG_MAX_CONCURRENCY="${OOOSPLAT_BUILD_JOBS:-4}"
export VCPKG_DISABLE_METRICS=1
rustup toolchain install 1.95.0 --profile minimal

# Use detached, immutable Git commits and refuse reuse of modified source trees.
checkout() {
  local url="$1" commit="$2" dest="$3"
  if [[ ! -d "$dest/.git" ]]; then
    git init "$dest"
    git -C "$dest" fetch --depth 1 "$url" "$commit"
    git -C "$dest" checkout --detach FETCH_HEAD
  fi
  [[ "$(git -C "$dest" rev-parse HEAD)" == "$commit" ]] || { echo "Wrong source commit: $dest" >&2; exit 1; }
  [[ -z "$(git -C "$dest" status --porcelain --untracked-files=no)" ]] || { echo "Modified source: $dest" >&2; exit 1; }
}
colmap_commit="$(read_lock commit)"
brush_commit="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["brush"]["commit"])' "$workspace/engines/manifest.linux.json")"
checkout https://github.com/colmap/colmap.git "$colmap_commit" "$cache/colmap-$colmap_commit"
checkout https://github.com/microsoft/vcpkg.git "$(read_lock vcpkgCommit)" "$cache/vcpkg"
checkout https://github.com/ooolabdev/OOOBrush.git "$brush_commit" "$cache/brush-$brush_commit"
source_dir="$cache/colmap-$colmap_commit"
bash "$cache/vcpkg/bootstrap-vcpkg.sh" -disableMetrics
build="$cache/colmap-build-$backend"
# Build into a fresh immutable release. Activation is an atomic symlink switch.
release="$(mktemp -d "$workspace/engines/.native-release-XXXXXX")"
colmap="$release/linux/colmap"
mkdir -p "$colmap/lib" "$release/linux/brush"
options=()
while IFS= read -r feature; do options+=("-D${feature}_ENABLED=OFF"); done < <(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["disabledFeatures"]))' "$workspace/engines/colmap-build.json")
cuda_root="${CUDA_PATH:-/usr/local/cuda}"
if [[ "$backend" == cuda ]]; then
  export CUDA_PATH="$cuda_root"
  export PATH="$cuda_root/bin:$PATH"
  python3 "$workspace/scripts/native-cuda.py" "$cuda_root" > "$cache/cuda.json"
  architectures="${OOOSPLAT_CUDA_ARCHITECTURES:-native}"
  options+=(-DCUDA_ENABLED=ON -DCASPAR_ENABLED=ON -DCASPAR_USE_DOUBLE=OFF
    "-DCMAKE_CUDA_ARCHITECTURES:STRING=$architectures" "-DCMAKE_CUDA_COMPILER:FILEPATH=$cuda_root/bin/nvcc" "-DCUDAToolkit_ROOT:PATH=$cuda_root")
else
  options+=(-DCUDA_ENABLED=OFF -DCASPAR_ENABLED=OFF)
fi
cmake -S "$source_dir" -B "$build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$colmap" \
  -DGIT_COMMIT_ID="$colmap_commit" -DGIT_COMMIT_DATE=Unknown \
  -DCMAKE_TOOLCHAIN_FILE="$cache/vcpkg/scripts/buildsystems/vcpkg.cmake" \
  -DVCPKG_TARGET_TRIPLET=x64-linux-dynamic -DBUILD_SHARED_LIBS=OFF \
  -DCMAKE_INSTALL_RPATH='$ORIGIN/../lib' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON "${options[@]}"
if [[ "$backend" == cuda ]]; then
  python3 "$workspace/scripts/native-cuda.py" "$cuda_root" --build "$build" > "$release/CUDA-BUILD.json"
fi
cmake --build "$build" --parallel "$VCPKG_MAX_CONCURRENCY"
cmake --install "$build"
installed="$build/vcpkg_installed/x64-linux-dynamic"
while IFS= read -r library; do
  relative="${library#"$installed/lib/"}"
  mkdir -p "$(dirname "$colmap/lib/$relative")"
  cp -L "$library" "$colmap/lib/$relative"
done < <(find -L "$installed/lib" -type f -name '*.so*')
node "$workspace/scripts/collect-linux-runtime.mjs" "$colmap" "$installed" "$cuda_root"
while IFS= read -r library; do
  case "$(basename "$library")" in libcudart.so*|libcurand.so*) continue ;; esac
  relative_lib="$(realpath --relative-to="$(dirname "$library")" "$colmap/lib")"
  patchelf --set-rpath "\$ORIGIN:\$ORIGIN/$relative_lib" "$library"
done < <(find "$colmap/lib" -type f -name '*.so*')
patchelf --set-rpath '$ORIGIN/../lib' "$colmap/bin/colmap"
# Keep dependency license material with the local build; do not package a GUI.
mkdir -p "$colmap/licenses"
cp "$source_dir/COPYING.txt" "$colmap/licenses/COLMAP-LICENSE.txt"
cp -a "$installed/share" "$colmap/licenses/vcpkg"
while IFS= read -r license_file; do
  relative="${license_file#"$source_dir/src/thirdparty/"}"
  mkdir -p "$(dirname "$colmap/licenses/source-thirdparty/$relative")"
  cp "$license_file" "$colmap/licenses/source-thirdparty/$relative"
done < <(find "$source_dir/src/thirdparty" -type f \( -iname '*license*' -o -iname '*copying*' \))
if [[ -d "$build/_deps" ]]; then
  while IFS= read -r license_file; do
    relative="${license_file#"$build/_deps/"}"
    mkdir -p "$(dirname "$colmap/licenses/fetched/$relative")"
    cp "$license_file" "$colmap/licenses/fetched/$relative"
  done < <(find "$build/_deps" -type f \( -iname '*license*' -o -iname '*copying*' \))
fi
# Data only: download and hash the locked offline vocabulary, not a runtime binary.
python3 "$workspace/scripts/native-runtime.py" vocabulary "$release"
node "$workspace/scripts/smoke-colmap-image-io.mjs" "$colmap/bin/colmap"

brush_source="$cache/brush-$brush_commit"
CARGO_TARGET_DIR="$cache/brush-target" cargo +1.95.0 build --locked --release \
  --manifest-path "$brush_source/Cargo.toml" -p brush-cli --bin brush-cli --jobs "$VCPKG_MAX_CONCURRENCY"
install -m 755 "$cache/brush-target/release/brush-cli" "$release/linux/brush/brush_app"
cp "$brush_source/LICENSE" "$release/linux/brush/LICENSE"
cp "$brush_source/Cargo.lock" "$release/linux/brush/UPSTREAM-Cargo.lock"
cp "$workspace/engines/manifest.linux.json" "$release/manifest.linux.json"
cargo +1.95.0 build --release --locked --manifest-path "$workspace/src-tauri/Cargo.toml" --bin splatstudio --jobs "$VCPKG_MAX_CONCURRENCY"
python3 "$workspace/scripts/native-runtime.py" record "$release" --backend "$backend"
python3 "$workspace/scripts/native-runtime.py" verify "$release"
# Avoid switching the default runtime during an active job in the default store.
# Submitted jobs resolve their runtime symlink and remain pinned to that release.
mkdir -p "$workspace/jobs"
exec 8>"$workspace/jobs/gpu.lock"
flock -n 8 || { echo "A job is using the current runtime; stop jobs and rerun. Built runtime retained: $release" >&2; exit 1; }
python3 "$workspace/scripts/native-runtime.py" activate "$release"
echo "Native build installed. Run: $workspace/splat health"
