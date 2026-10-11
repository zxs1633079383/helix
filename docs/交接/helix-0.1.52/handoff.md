# Helix 0.1.52 发布与客户端依赖接入（2026-10-11）

## 授权与范围

用户明确要求合并 Helix、push、发布新版并让客户端升级依赖；随后明确选择 patch 0.1.52。目标为四个 crates.io 公共包，不包含桌面安装包、npm 包或服务端部署。

- 分支 `reactor/MessageV3-helix`，upstream `origin/reactor/MessageV3-helix`；保留分支和既有版本 Tag。
- 合并提交 `b4398d59bf2857c76bd071665662ba2f75a0d11d` 的两个父提交为本地撤回修复记录 `ac5994ea` 与远端文档 `8c193292`。合并仅改变 Markdown；当前源码、manifest、lock 保留。
- 已正常推送分支及 `helix-v-0.1.51`；该 Tag 仍指向原发布提交 `d61ff593`。
- 发布计划通过：0.1.51 四包均在 registry 可解析，0.1.52 四包均未占用；使用 `scripts/publish_crates.py --version 0.1.52 --publish`，精确版本被占用时停止，不自动更换目标。
- 四包：`cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native`；FFI/Web/CLI/perf 只参与 workspace 版本事务和编译，不上传。

## 当前状态

RELEASED：四公共包 0.1.52 均上传 crates.io 并可解析。发布脚本完成八 crate check 和四包 dry-run；九份 manifest/lock 已提交为 `7a953edac69cad7fbcfd00416629cb960a4c475f`，注释 Tag `helix-v-0.1.52` 指向该提交。四包归档的源码、README 与原始 manifest 逐字节对应发布提交，制品 SHA 已保存。分支及新 Tag 的最终远端核验在本轮推送回执记录。

客户端只更新四个精确依赖及对应 Cargo.lock；保护工作区已有撤回实现、未跟踪的动态库实验和其他任务文件。客户端源码接入、定向验证、真实桌面验收与桌面制品分别记录。

## 验证与证据

- 合并：Markdown-only、无冲突、六处当前版本入口和新增本地链接断言、git diff --cached --check 通过。Rust 源码和依赖未变，复用已有撤回定向证据。
- 发布脚本执行八 crate check、逐包 dry-run、依赖顺序上传及 registry 可解析核验；四包均通过，日志见 `release-0.1.52.log`。
- 本轮命令、registry 与推送响应在本仓 `artifacts/helix-merge-publish-20261011/`。
- 当前未证明真实 Tauri 到已部署 Go 的撤回链路，也未构建桌面安装包。

## 客户端接入

客户端四个精确依赖和 lock 已升级为 0.1.52，四个 registry checksum 与本轮上传 `.crate` 的 SHA-256 相同；其他 lock 依赖及此前待提交源码均保持不变。客户端定向 native 验证正在执行，最终结果和独立依赖提交另行记录。未生成桌面安装包。
