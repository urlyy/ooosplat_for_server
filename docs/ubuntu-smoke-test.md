# Ubuntu 原生运行实测

日期：2026-10-18（北京时间）。不使用 Docker，无桌面会话。

## 环境与结果

- Ubuntu 22.04.5 x86_64，RTX 5090（32 GB），驱动 595.71.05。
- 实际 nvcc：CUDA Toolkit 13.0.88。nvidia-smi 显示的 13.2 是驱动能力，不是已安装 Toolkit。
- 本机源码编译 COLMAP 4.2.1 与 OOOBrush CLI 1.0.0；COLMAP GPU 执行探测、动态库与图像读写检查通过。
- Rust 核心 220 项测试通过；另行执行 Ubuntu 系统 FFmpeg/FFprobe 旋转视频测试通过。Python 12 项测试通过。
- 三档 plan 入口通过：6 秒 / 180 帧视频的静态计划为 fast 54、balanced 90、high 180 帧。
- 实际后台任务：fast，默认自动优化开启，优化后使用 36 帧，36/36 注册成功，13,496 个稀疏点。
- Brush Vulkan 后端使用 RTX 5090，完成 8,000 步训练。
- 最终文件：21,816,747 字节，92,437 个 Splat；流水线 PLY 校验通过，无告警。
- 端到端耗时 115,648 ms（不含首次编译）；watch 显示 completed / 100%。
- 验收任务 ID：`3e29bd73-8a4e-4a4a-93b2-2d9e827e496b`。

## 测试中修复

1. Ubuntu 22.04 FFprobe 4.4 不识别旧调用中的 stream_side_data 查询节，改用 show_streams/show_format，并保留旋转元数据读取。
2. 原生构建识别实际 nvcc，接受 12.8、13.0、13.2，默认为当前 GPU 架构构建；不修改上游发布锁。
3. 修正 PyPI Ninja 版本，允许 root-only 云实例显式选择 root 构建。
4. 高核数宿主机的默认闭环检索线程数触发 OpenBLAS 内存错误；显式限制 COLMAP 检索与数学库并发后，同一视频完整通过。未关闭闭环匹配或自动优化。

## 复现

测试视频生成器额外需要 Ubuntu 的 python3-numpy、python3-opencv；这些不是后台服务运行依赖。

```bash
sudo apt-get install -y python3-numpy python3-opencv
/usr/bin/python3 scripts/create-smoke-video.py /tmp/splat-orbit.mp4
JOB=$(./splat submit /tmp/splat-orbit.mp4 --quality fast)
./splat watch "$JOB"
```

这是合成纹理场景的执行验收，不代表真实拍摄视频的重建质量。CUDA 12.8/13.2、balanced/high 完整训练未在本次分别执行。


## 2026-10-25：CUDA Toolkit 12.9 实测

Ubuntu 22.04.5 x86_64、RTX 5090（32 GB）、NVIDIA 驱动 590.44.01，实际 nvcc 为 **12.9.86**。原生构建的版本检查加入 12.9 后，未修改 COLMAP、Caspar 或 Brush 源码，即完成锁定版本的源码编译、引擎清单/ABI/CLI 验证和 `./splat health`。构建记录 `CUDA-BUILD.json` 确认编译器为 `/usr/local/cuda-12.9/bin/nvcc`，架构为 `native`；没有使用备用的 CUDA 13.0 编译。

使用上述合成视频生成器，fast 模式、默认自动优化开启的后台任务通过完整流水线：

- COLMAP 使用 NVIDIA CUDA / Caspar GPU，36/36 帧注册成功，13,492 个稀疏点。
- Brush 在 RTX 5090 的 Vulkan 后端完成 8,000 步训练。
- 输出 `final.ply`：21,221,083 字节、89,913 个 Splat，校验通过，无告警。
- 流水线耗时 81,630 ms（不含首次编译），任务状态 `completed`，退出码 0。
- 验收任务 ID：`74f2857b-54f4-42ff-a2dd-b0e4f1855b5f`。

构建时使用 Rust 1.95.0、Node.js 22.23.4、CMake 4.3.2、Ninja 1.13.2。数据盘曾触发账户配额限制，将 `.cache/native-linux` 迁移至系统盘并在原路径保留符号链接后完成构建；项目、运行时和任务输出仍在数据盘。共享盘的 `df` 空闲空间不代表账户可用配额。

本次验证的是 CUDA 12.9.86 的原生编译与 fast 合成视频端到端执行；CUDA 12.8/13.2、balanced/high 完整训练仍未分别实测。
