# 上游选择性跟进

上游：https://github.com/ooolabdev/ooosplat ，分支 `main`。
本项目保留 Ubuntu 22.04 原生、无界面的视频重建与 Gaussian Splatting 流程。

## 筛选规则

- 优先移植：视频探测/抽帧、COLMAP 重建、Brush 训练、自动优化、质量档位、耗时估算、Linux 引擎兼容性、相关安全和稳定性修复。
- 保护本地改造：后台任务/取消/锁、原生引擎构建与清单、CUDA 选择、COLMAP 线程上限、命令行 estimate、无桌面依赖的 Rust crate。
- 默认跳过：React/Tauri UI、预览编辑、桌面导出、MCP、遥测、Windows/macOS 打包和安装器。混合提交必须逐段拆分，不能仅凭文件名排除。
- 引擎版本、依赖锁、配置格式、质量默认值和训练算法变化需审查兼容性；不直接覆盖 manifest、Cargo 文件或整个目录。
- 潜在有用但超出当前功能的更新登记为候选。例如 SPZ/SOG 导出可在有需求时另做 headless CLI 适配。

## 操作流程

1. `git status --short` 检查工作区，保留尚未提交的改动。
2. `git fetch --no-tags upstream main`；网络失败时报告失败，不能以缓存结果宣称上游无更新。
3. 从 `config/upstream-tracking.json` 的 `reviewed_through` 开始比较。先确认它仍是 `upstream/main` 的祖先；历史重写时重新核对，不能直接推进检查点。
4. `git log --reverse <reviewed_through>..upstream/main`，逐提交看完整 diff 和依赖。检查点表示已审查，不代表所有功能都已启用。
5. 将服务器改造 rebase 到已审查的上游提交之上，保留上游原始提交历史。解决冲突时按上述规则保留服务器功能、排除不需要的上游功能，并核对 rebase 前后的最终文件树。上游提交进入历史，不代表其桌面功能必须留在服务器最终代码中。
6. 执行 README 的 Python、引擎脚本、Rust test/clippy/fmt 检查。影响引擎或训练时，额外做 Ubuntu/GPU 验收；本机单测不能替代。
7. 更新逐项记录后才推进 `reviewed_through`。候选项保留，后续不因检查点推进而丢失。未经验证的移植不能标记为已完成。

按需跟进：用户明确不需要每日检查，已关闭自动任务。仅在用户提出需求时拉取、审查和移植更新。

## 2026-10-18 首次审查

最初分叉点：`9cd6b5ab1364f2b24ddaafc61192e3735a4601c9`。
首次 rebase 基线：`410859b91850a7b3591878809f3bf11febe3fb64`。上游三个新增提交均在历史中；服务器改造在其后应用，最终只保留所需代码。
已审查至：`410859b91850a7b3591878809f3bf11febe3fb64`。

| 上游提交 | 决定 | 原因与处理 |
| --- | --- | --- |
| `0cf8cdc6b46340da0aeca8faa5d365b5b38332f3` | 移植 | imports.rs 和 pipeline/runner.rs 的相机参数按 8 字节分组读取，修复 Clippy；保留非法 BLOB 回归测试。适配服务器版精简后的 imports.rs。 |
| `09d6ebcee0fdc797524d1336ca22f95e105aff8d` | 跳过 | Windows FFmpeg 下载/打包及相关许可更新；本分支使用 Ubuntu 系统 FFmpeg，未引入该二进制。 |
| `410859b91850a7b3591878809f3bf11febe3fb64` | 候选，未移植 | PLY/压缩 PLY/SPZ/SOG 导出涉及桌面命令、浏览器 worker 与预览变换。当前服务输出 final.ply；需要独立 CLI 设计后再引入。 |

本次本机验证：Rust 217 通过、3 忽略；Python 12 通过；引擎脚本 44 通过、1 跳过；Clippy（warnings as errors）、rustfmt 和 git diff --check 通过。未重新执行 Ubuntu/GPU 端到端验收。

首次选择性移植已改为真正 rebase：COLMAP 修复使用上游原始提交，重复的本地修复提交被自动丢弃；Windows 打包和桌面导出功能随服务器提取提交排除。已核对 rebase 后代码与先前验证并推送的服务器代码完全一致，只有本跟进说明及基线元数据另行更新。重写前备份保存在本地 `codex/backup-before-upstream-rebase` 分支。


## 2026-10-28：跟进上游 0.7.1

已审查并 rebase 至 `fbea25999948fcbf2b874fccebd8b13c02f8e198`。保留远程新增的 CUDA 12.9 支持及 5090 验收记录（原提交 `d049c31`），重写前本地备份：`codex/backup-before-upstream-20261028`。

| 上游提交 | 最终服务器代码处理 |
| --- | --- |
| `1f3343a`、`13a9953`、`f516f03` | 接入全景视频/图片检测、球面重建和六面训练计算；不恢复桌面 UI。后台提交接受图片目录和视频投影参数。 |
| `a77b4b5` | 核心及服务器包版本更新为 0.7.x，最终采用 0.7.1。 |
| `e2af865` | 接收核心固定分组读取修复；服务器仍用 Rust 1.95.0，并适配该版本 Clippy。 |
| `6f8427a` | SPZ 确认、桌面导出和预览更新不开放；服务器使用核心所需的 PLY 校验/坐标变换辅助，不增加导出 CLI。 |
| `1ba53af` | 桌面预览按需渲染与漫游交互跳过。 |
| `c00a09d` | 接入 GPU OOM、COLMAP 原生崩溃及子进程终止分类，保留真实失败证据。 |
| `2cebd01` | 接入 warm-start 补拍计算、验证及流式 PLY 变换；无新增补拍 CLI。 |
| `2db1f6f` | 接入 FFprobe 原始错误诊断、引擎健康详情，保留 Ubuntu 系统 FFmpeg 兼容测试。 |
| `fe80eb3` | 保留共享引擎安装脚本的目录替换重试及测试，不恢复 Windows 打包工作流。 |
| `6519351` | 接入本地 GPS 元数据与几何计算依赖、PNG 焦距兼容处理；保留上游平台/版本保护，Linux 不启用尚未验收的原生 GPS 辅助。 |
| `fbea259` | 接入生成策略 v2、精细档有效像素/轨迹点数预算和相关计算更新；桌面交互跳过。 |

适配：将补拍缓存分析 DTO 从桌面 commands 移到 pipeline；保留简化 tasks/imports、无网络遥测的库入口、系统引擎发现、OpenBLAS=1/OMP最多8/闭环检索8线程、后台锁/取消/运行时固定与 estimate。新增依赖仅为核心所需的 EXIF/XML/压缩和 SQLite backup，不恢复 Tauri/GTK/WebKit/前端依赖。

新任务默认接入上游生成策略；`--no-optimize` 保留旧策略路径。CLI/后台增加抽帧比例和关闭自动重试选项，旧任务请求 JSON 继续兼容。High 新策略可能增加资源需求，estimate 尚不能精确预测重建后的 Splat 预算或全景六面训练。GPS/全景/新版配方不冒用旧 5090 小视频验收结果。

本次本机验证：Rust 353 通过、19 项资源/人工测试默认忽略；Python 15 通过（包含后台参数转发、图片目录提交及非法抽帧比例拒绝）；引擎脚本 51 通过、2 跳过；Rust 1.95.0 全目标 Clippy（`-D warnings`）、rustfmt、Shell 语法、git diff --check、release 构建及 CLI 版本/帮助检查通过。Ubuntu 系统 FFprobe 旋转视频测试保留于 CI，本机 macOS 未执行；未连接 GPU 服务器，没有新增 Ubuntu/GPU 训练验收结论。
