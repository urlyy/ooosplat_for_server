# Splat Server（OOOSplat 无界面修改版）

上游更新采用[选择性跟进](docs/upstream-sync.md)：保留服务器所需的重建、训练和 Linux 修复，逐项记录已移植、跳过及候选更新。

在 **Ubuntu 22.04 x86_64 原生运行，不使用 Docker**。视频 → 抽帧 → COLMAP 重建 → Brush 训练 → `final.ply`。保留 `fast` / `balanced` / `high` 三档和默认开启的自动优化；任务后台执行，可通过 `watch` 查看详情。

已移除 React/Tauri GUI、模型预览编辑、HTML/视频导出、MCP 和遥测。`src-tauri/` 只是历史目录名，现为普通 Rust crate，不依赖 Tauri、GTK 或 WebKit。Python 只使用标准库；Node 用于引擎构建检查，不需要安装前端 npm 依赖。

**已于 2026-10-18 在 Ubuntu 22.04.5 / RTX 5090 / CUDA Toolkit 13.0 完成原生编译和合成视频端到端测试，并于 2026-10-25 在同系统/GPU、CUDA Toolkit 12.9.86 上再次通过。** 快速档开启自动优化，36/36 帧注册成功，完成 8000 步 Vulkan GPU 训练并输出 `final.ply`。CUDA 12.8 / 13.2 接受构建，但尚未分别实机验证；balanced/high 已检查计划入口，未执行完整训练。详见 [实测记录](docs/ubuntu-smoke-test.md)。

## 0.7.1 核心更新（2026-10-28）

已跟进上游至 `fbea259`，保留 Ubuntu 22.04 原生、无界面运行和 CUDA 12.9 支持。新增全景视频/图片计算流程、失败分类与诊断、新版生成策略及精细档点数预算；后台任务仍按单 GPU 串行执行。

```bash
# 普通图片目录也可以提交后台任务
./splat submit /absolute/path/images --quality balanced
# 完整、单目、2:1 ERP 全景视频；素材格式要求见下方全景文档
./splat submit /absolute/path/panorama.mp4 --video-projection equirectangular
# 自定义初始视频抽帧比例；默认开启自动优化和自动失败重试
./splat submit /absolute/path/video.mp4 --frame-sampling-ratio 0.2 --no-auto-retry
# 对应的估算也使用同样抽帧比例
./splat estimate /absolute/path/video.mp4 --frame-sampling-ratio 0.2
```

这些选项也支持前台 `generate`；`plan`/`extract` 支持抽帧比例。`--no-auto-retry` 控制新版优化流程中的失败重试，`--no-optimize` 保留旧流程；抽帧比例不能和 `--no-optimize` 混用。显式全景投影参数用于视频，图片目录由图片元数据与尺寸检测。`estimate` 仍是普通视频的近似估算，不能当作全景六面训练或重建后精细档点数预算的精确耗时。

新版优化流程的精细档使用原生分辨率及重建后计算的 Splat 上限，不再仅凭显存自动选择较低 High 配方；资源占用可能高于旧版。规则见[生成策略](docs/GENERATION_POLICY.md)和[精细档预算](docs/FINE_SPLAT_BUDGET.md)。[全景视频](docs/PANORAMA_VIDEO_V1.md)、[全景图片](docs/PANORAMA_IMAGES_V1.md)与[补拍核心](docs/RESHOOT_V2.md)沿用上游计算实现；补拍核心没有新增服务器 CLI 入口。

GPS 元数据读取及几何逻辑已纳入依赖，但[原生 GPS 辅助](docs/IMAGE_GPS.md)仍按上游限制只对 Windows x64 开启，Linux 自动保留视觉流程。部分 PNG 焦距兼容处理也仅在识别出受影响的确切引擎版本时生效。本次只做本机自动化验证，未重新进行 Ubuntu/GPU 端到端验收；前述 5090 测试记录对应旧版配方，不代表新增全景或新版 high 档已验收。`docs/` 中导入的上游文档含桌面使用及其作者实测说明，不代表本服务器提供对应 UI 或导出入口。

## 1. 准备 Ubuntu 22.04

通常使用普通用户构建，只有系统包安装需要 sudo。AutoDL 等只提供 root 的实例可显式设置 `export OOOSPLAT_ALLOW_ROOT_BUILD=1`。构建需要访问 GitHub、crates.io、PyPI 和 vcpkg 上游依赖；首次编译较久，源码、缓存和中间产物会占用较多磁盘。默认使用 4 个编译进程，可用 `OOOSPLAT_BUILD_JOBS` 调整。

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential gfortran git curl ca-certificates pkg-config \
  python3 python3-venv python3-pip zip unzip tar xz-utils \
  autoconf automake libtool bison flex gperf nasm patchelf \
  libgl1-mesa-dev libglu1-mesa-dev libglew-dev libssl-dev libudev-dev \
  libx11-dev libxrandr-dev libxi-dev \
  ffmpeg libvulkan1 vulkan-tools procps
```

还需准备：

- **rustup**：按 [Rust 安装说明](https://www.rust-lang.org/tools/install) 安装。构建脚本会安装并使用 Rust **1.95.0**。
- **Node.js 22.13+**：从 [Node.js 官方下载页](https://nodejs.org/en/download) 安装；Ubuntu 22.04 默认仓库中的旧 Node 版本不适用。
- **GPU 驱动与 Vulkan**：Brush 是无窗口 CLI，但训练仍需可用的 Vulkan GPU；先运行 `vulkaninfo --summary` 检查设备。没有桌面会话也可以运行，无需 Xvfb。软件 Vulkan/CPU-only 训练未验收。
- **CUDA 模式额外要求**：原生构建接受 CUDA Toolkit **12.8 / 12.9 / 13.0 / 13.2**，需要 `nvcc`，不是只有 NVIDIA 驱动。默认路径 `/usr/local/cuda`，可设置 `CUDA_PATH`。默认只为本机 GPU 架构编译（`native`），可用 `OOOSPLAT_CUDA_ARCHITECTURES` 指定其他架构。运行时自动选择 NVIDIA CUDA 的门槛仍是驱动 **580.00+**、Compute Capability **7.5+**。脚本不自动安装或改动驱动/Toolkit。

为避免大核数宿主机上 OpenBLAS 并发池溢出，COLMAP 子进程限制 OpenBLAS 为 1 线程、OpenMP 最多 8 线程，并显式设置闭环检索线程数为 8；闭环匹配仍保持开启。

系统 OpenGL/X11 开发库是 COLMAP 依赖构建所需，不会启用 COLMAP GUI 或启动显示服务。

## 2. 原生构建

在项目根目录选择一种 COLMAP 后端：

```bash
# NVIDIA CUDA：保留 CUDA SIFT 和 Caspar/Ceres 能力
bash scripts/setup-engines-linux.sh cuda

# 或者：CPU COLMAP，不要求 CUDA Toolkit；Brush 仍通过 GPU/Vulkan 训练
bash scripts/setup-engines-linux.sh cpu
```

等价入口是 `npm run setup:native -- cuda` / `npm run setup:native -- cpu`。脚本只接受 Ubuntu 22.04 x86_64，并会：

1. 在项目内的 Python venv 安装 CMake 4.3.2 / Ninja 1.13.2，不覆盖系统版本。
2. 按仓库锁定的 Git commit 获取 COLMAP、vcpkg 和 OOOBrush。
3. 在本机编译 COLMAP 和其依赖，关闭 GUI/OpenGL 渲染/MVS 等无关功能。
4. 编译 OOOBrush 的 `brush-cli`，不编译 viewer；再编译 Rust 核心。
5. 下载、校验离线闭环词汇树。该文件是与平台无关的数据，来自 [COLMAP 官方记录的资源](https://colmap.github.io/legacy/3.13/pycolmap/pycolmap.html)，不使用 24.04 预编译引擎。
6. 检查 COLMAP PNG/JPEG/蒙版处理、动态库加载和 CLI 参数，记录本地构建清单，最后切换运行时链接。

运行时输出到 `engines/.native-release-*/`，`engines/native-linux` 指向当前版本；下载/编译缓存位于 `.cache/native-linux/`。旧版本与失败构建保留，便于排错。没有运行任务引用后，才可手动清理旧版本。源码树被修改或 commit 不符时会拒绝复用缓存。

CPU 模式的 COLMAP 阶段通常更慢，不启用 CUDA/Caspar；三档与自动优化流程仍可用。无法识别兼容 NVIDIA GPU 时，精细档显存规划会沿用保守配置。

```bash
# 验证本地构建清单、依赖、版本和必需 CLI 参数
bash scripts/verify-engines-linux.sh
./splat health
./splat probe /absolute/path/orbit.mp4
```

`NATIVE-BUILD.json` 记录本地构建文件哈希与源码身份，区别于上游 Release 的二进制哈希；不代表经过官方签名或 GPU 验收。`health` 成功也不等于端到端训练成功。

## 运行前比较预计耗时

先用同一段视频比较三档的大概总耗时范围，再选择生成模式：

```bash
./splat estimate /absolute/path/orbit.mp4
# 只查看指定模式，或取得机器可读的毫秒数
./splat estimate /absolute/path/orbit.mp4 --quality high
./splat estimate /absolute/path/orbit.mp4 --json
# 如果准备关闭自动优化，估算时也使用相同选项
./splat estimate /absolute/path/orbit.mp4 --no-optimize
```

命令会显示 fast / balanced / high 的预计分钟数范围、参考值、计划帧数、置信度和历史样本数。默认按自动优化开启估算，读取视频元数据并复用原产品的估算算法、GPU 显存规划和当前用户的成功任务历史，不抽帧、不提交任务。首次使用没有可比历史时仍会给出范围，但置信度低。新完成的任务会登记到当前用户的项目目录索引以供校准；旧版诊断入口生成但未登记的任务不会自动参与。实际画面内容、优化后的帧数、GPU 性能与负载会影响耗时；范围不包含排队时间和首次引擎编译时间，不是完成时限保证。

选定模式后再执行下方的 `submit`；如估算用了 `--no-optimize`，提交时也加上它。

## 3. 后台生成与 watch

```bash
# 默认开启自动优化，提交后立即返回任务 ID
JOB=$(./splat submit /absolute/path/orbit.mp4 --quality balanced)
echo "$JOB"

# 查看实时进度；Ctrl+C 只退出查看，后台继续运行
./splat watch "$JOB" --interval 2 --lines 12

# 也可以用系统 watch
watch -n 2 "./splat status $JOB --lines 8"

./splat status "$JOB" --json
./splat list
./splat cancel "$JOB"
```

三种模式及开关：

```bash
./splat submit /path/video.mp4 --quality fast
./splat submit /path/video.mp4 --quality balanced
./splat submit /path/video.mp4 --quality high
./splat submit /path/video.mp4 --quality high --no-optimize
```

自动优化复用原有素材/显存规划、抽帧与分辨率选择、视频闭环匹配、按需桥接补帧、精细档显存不足时的一次降档重试。配置在 `config/pipeline-optimization.json`。

`watch` 显示阶段、总进度、阶段进度、帧/训练计数、耗时、加速信息、运行资源详情和最近日志。部分阶段百分比不精确，JSON 保留 `indeterminate`。任务结束时内置 `watch` 自动退出。

同一任务目录通过文件锁串行运行 GPU 工作，不保证严格 FIFO。退出终端或 SSH 不影响后台任务；主机重启或进程异常结束后显示 `interrupted`，不自动续跑。运行中的取消会等待 Rust 清理引擎进程组。

默认任务目录为仓库的 `jobs/`。可以用 `./splat --jobs /absolute/jobs submit ...` 自定义；同一 GPU 请共用一个支持 `flock` 的本地任务目录。每个任务固定使用提交时的运行时目录，升级引擎不会让排队任务切换到另一版本。

## 4. 输出

```text
jobs/<TASK_ID>/
  request.json       输入、模式、优化开关和固定引擎路径
  status.json        生命周期、退出码、最终结果
  events.jsonl       完整进度事件
  pipeline.log       阶段消息与错误
  worker.log         后台监督进程日志
  result.json        成功时的结构化结果
  projects/<项目>/
    final.ply
    project.json
    state.json
    source/          源素材副本
    work/            抽帧、COLMAP 和 Brush 中间文件
    logs/            各引擎详细日志
```

`status` 结合持久化记录和进程锁识别异常退出，查看实时状态请使用命令，而不只读取旧的 `status.json`。模型和中间文件不会自动删除。

## 开发与检查

```bash
python3 -m unittest discover -s tests -v
npm run test:engine-scripts
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml --check
bash -n splat scripts/build-native-linux.sh scripts/setup-engines-linux.sh scripts/verify-engines-linux.sh
```

前台诊断入口：

```bash
./splat plan /path/video.mp4 --quality balanced
./splat generate /path/video.mp4 --projects-root ./projects --quality high --events ./events.jsonl
```

`plan` 展示静态预设的抽帧预估，实际优化决策以生成事件和 `state.json` 为准。前台 `generate` 不经过后台串行锁，日常运行用 `submit`。后台入口支持视频文件及 JPG/PNG 图片目录。

`./splat` 默认使用本机编译的 `engines/native-linux`；可通过 `OOOSPLAT_ENGINE_DIR` 或提交参数 `--engine-dir` 覆盖。FFmpeg/FFprobe 从 PATH 或 `OOOSPLAT_FFMPEG` / `OOOSPLAT_FFPROBE` 获取。不要用 Ubuntu 默认仓库中的旧 COLMAP 或其他版本的 Brush CLI 替换锁定引擎。

## 许可

本项目是基于 OOOSplat 的修改版，不是上游官方桌面发行版。沿用 [Apache-2.0](LICENSE)，保留 [NOTICE](NOTICE)、[品牌政策](TRADEMARK_POLICY.md)、[输出说明](GENERATED_OUTPUTS.md) 和 `licenses/`。原有上游引擎锁与通用下载/校验工具保留用于溯源，默认安装入口已经改为本机构建。
