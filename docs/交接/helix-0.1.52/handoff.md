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
- 发布后的 `cargo test -p helix-im --test read_relay_test` 37/37 与 `cargo test -p helix-im --lib outbound::registry` 9/9 通过，日志为 `read-relay-tests.log` 与 `registry-tests.log`。
- 本轮命令、registry 与推送响应在本仓 `artifacts/helix-merge-publish-20261011/`。
- 当前未证明真实 Tauri 到已部署 Go 的撤回链路，也未构建桌面安装包。

## 客户端接入

客户端四个精确依赖和 lock 已升级为 0.1.52，四个 registry checksum 与本轮上传 `.crate` 的 SHA-256 相同；其他 lock 依赖及此前待提交源码均保持不变。独立提交 `3e1e748f92` 只包含 `src-tauri/Cargo.toml` 与 `src-tauri/Cargo.lock`，两文件各版本/校验和替换共12增12删。未生成桌面安装包。

- `cargo test --lib features::helix_im::commands::tests` 首次并行运行17/18通过；`engagement_mutation_key_maps_to_stable_mrc_track_id` 读到了另一测试设置的 `mrc-shared-suite-g01`，之后又读到被清空的状态。两个测试共用进程全局 `E2E_SCENARIO_CORRELATION`，相关函数及测试与升级前客户端 HEAD 逐字节一致，未调用 Helix crate。
- 首次完整串行重跑同18项（`-- --test-threads=1`）18/18通过；没有删断言、跳过用例或修改此前源码。这次验证基于原客户端工作区，包含原有待提交撤回实现，不等于真实 Tauri/Go 全链路验收。
- 客户端远端新增3个前端提交，Rust manifest/lock、commands及相关测试均未变；代码过旧不能解释该并行失败。用户随后明确批准合并最新代码、复测并推送客户端本地七个提交。
- 客户端证据在 `artifacts/helix-0.1.52-client-20261011/`，含 `dependency-integrity.json`、`test-verification.json`、两份native测试日志。

## 客户端合并交付

- 在独立工作树 `/Users/mac17/.codex/worktrees/helix-0-1-52-client-sync/cses-client` 无冲突合并 `3e1e748f92` 与远端 `e796a2053e`，合并提交 `55d1a62fcbbd46dfb0eae28874dd70fb1b309f98` 已正常推送 `origin/dev-message`；原工作区此前15份待提交/新增/删除文件的内容指纹保持不变，原目录的分支未强制移动。
- 最终合并后的干净源码：Native定向18/18串行通过；类型、消息包及其依赖构建、App dev构建、包边界、task兼容检查通过。工作树最初缺少本机生成的配置、macOS资源和前端bundle，已按最终源码生成App bundle并复制、校验已有资源后重跑，不作为业务或桌面验收证据。
- 前端18个定向套件407项中406通过，唯一失败为 `enableTopPrefetch` 的旧输入清单遗漏；该输入和未变的断言都存在于共同祖先 `575243435e`。40文件lint有3处旧错误，相同问题源码在共同祖先中已存在。失败证据保留，没有删断言或放宽规则。
- `check-ccc` 完整命令运行通过，但自动把UI升级到4.33.0；其manifest/lock副作用已恢复，通过frozen lock安装将实际UI恢复4.32.2。最终清单、锁文件、实际安装对齐，包边界与task兼容重跑通过，managed依赖使用现有只读模式核查，不把“最新版本”提示当作提版授权。
- 已核对 `@cses/message` 与 `@ccc/cses-common` 的dev/pre/prod发布基线；本任务不改变npm包版本、不上传模块包或桌面安装包。
- 最终验证、基线失败证明、源码指纹、资源校验和及推送回执在工作树 `artifacts/helix-0.1.52-client-sync-20261011/`。真实Tauri/已部署Go链路与Windows实机未验证。
