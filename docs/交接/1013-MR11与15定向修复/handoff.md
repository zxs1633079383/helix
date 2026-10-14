# MR11与15定向修复交接
更新时间：2026-10-13，Asia/Shanghai
## 目标与授权
仅修MR11 P1当前成员查询、MR15 P2自有WS夹具，并做本地定向验证。其他MR不改，不push、不触发流水线。另只读核对PC导航Token。
规模目标5000人/群、每群5RPS、多群并行、1秒可达；本轮不证明容量达标。
## 当前身份
- P1：/tmp/helix-review-fix11，codex/review-mr11-p1；base 5c8b32792e930bc74dbb7ad976b1745bc1c650a8；修复357dfb0ace83c6df23bc79e9a573dd775651c35a。
- P2：/tmp/helix-review-fix15，codex/review-mr15-p2；base b4729e4c3cc7b8d4004747e42a6189bd5bdcb719；修复80b4a5649ce2228fd7a2b6378fca0c90b03c9b43，已本地提交。
- 原主仓共享脏文件保留；两worktree隔离，其他MR分支不修改。
## 已完成
- P1 ScopedScan增加可选等值filter；SQLite与Web在limit前过滤当前viewer，保留10000行上限和成员跨公司语义。
- 公开Tick→Host SQLite三群15000成员红→绿；Host存储26、P6查询14、Web过滤2及channel helper1项通过。
- 五crate check、core测试、WASM build、core零runtime依赖、选定rustfmt/diff check通过；全仓fmt既有漂移未修。
- 7个代码/测试文件diff指纹3c2d8eaf3ac34299df708983b44e689bc2dc33635468df0c99faf38fb42b578d。
- 证据：P1 .perf/review-fix/（亦保留review-fix11/），命令/红绿输出随文件保存。
- PC客户端dev-message HEAD 21a66006966275115fc45cb9b8396be53df8f736，相关源码未改；locate适配器8项、UI locate40项通过。具体字段/路径见PC导航核对.md。
- P2 localhost WS握手与实际HTTP temporaryId/server identity关联echo；Rust与C++均等待唯一持久sent。
- P2 CTest3/3通过（正向/握手失败/缺echo），Rust公开FFI定向1项通过并复验3次；选定格式/语法/diff检查通过。
- P2四文件diff指纹540bf6a7bc29fed3f2ee27d55f764fa0449754df05a2cbaa0171d01ba90468e9；证据P2 .perf/review-fix/p2-local-evidence.md，构建使用DEVELOPER_DIR=/Library/Developer/CommandLineTools。
- P2测试中accepted socket WouldBlock竞态已修，重新验证通过；初始红测及运行输出保留在任务执行记录，具体过程见本地证据。
## 当前阶段与边界
P1/P2实现、本地验证、本地提交完成。没有push或流水线；远端MR仍为原提交。
真实Go、安装版PC/Mobile、UI宿主和容量均未验收。
## 下一步
授权范围内无剩余实现。后续若更新远端MR，需单独授权push；串联MR的祖先与目标关系须在发布时重新核对，本轮未调整。
