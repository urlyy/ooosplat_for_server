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
