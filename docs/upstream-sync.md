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
当前 rebase 基线：`410859b91850a7b3591878809f3bf11febe3fb64`。上游三个新增提交均在历史中；服务器改造在其后应用，最终只保留所需代码。
已审查至：`410859b91850a7b3591878809f3bf11febe3fb64`。

| 上游提交 | 决定 | 原因与处理 |
| --- | --- | --- |
| `0cf8cdc6b46340da0aeca8faa5d365b5b38332f3` | 移植 | imports.rs 和 pipeline/runner.rs 的相机参数按 8 字节分组读取，修复 Clippy；保留非法 BLOB 回归测试。适配服务器版精简后的 imports.rs。 |
| `09d6ebcee0fdc797524d1336ca22f95e105aff8d` | 跳过 | Windows FFmpeg 下载/打包及相关许可更新；本分支使用 Ubuntu 系统 FFmpeg，未引入该二进制。 |
| `410859b91850a7b3591878809f3bf11febe3fb64` | 候选，未移植 | PLY/压缩 PLY/SPZ/SOG 导出涉及桌面命令、浏览器 worker 与预览变换。当前服务输出 final.ply；需要独立 CLI 设计后再引入。 |

本次本机验证：Rust 217 通过、3 忽略；Python 12 通过；引擎脚本 44 通过、1 跳过；Clippy（warnings as errors）、rustfmt 和 git diff --check 通过。未重新执行 Ubuntu/GPU 端到端验收。

首次选择性移植已改为真正 rebase：COLMAP 修复使用上游原始提交，重复的本地修复提交被自动丢弃；Windows 打包和桌面导出功能随服务器提取提交排除。已核对 rebase 后代码与先前验证并推送的服务器代码完全一致，只有本跟进说明及基线元数据另行更新。重写前备份保存在本地 `codex/backup-before-upstream-rebase` 分支。
