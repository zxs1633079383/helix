# 发送与回应表情最近记录

- 授权：2026-10-06 用户要求按最近使用时间排序、两边分开保存、使用 Helix；不发布，不改版本。
- 复用本地确定性 `im_recent_history`，新增 kind `emoji-send` / `emoji-reaction` 与 target type `emoji`。最新使用移到头部，重复去重，最多32项；不累计次数、不重建没有时间依据的旧共享频次。
- 运行时 scope 仍由环境/公司/账号生成，写回 read 的 scope 并再验证；读改写串行、PersistOk 后返回结果，失败/重复回包/队列释放沿用现有协议，不新增网络或数据库表。
- 生命周期/成功与失败终态合同见 `crates/helix-im/src/recent_history.rs` 首部。HTTP/WS/远程写 N/A；Host 负责等待超时和取消。
- 本仓验证：公开 Module 定向4项通过，Host 真 SQLite2项通过（表情历史+既有搜索/转发），`cargo check -p helix-im` 通过，受影响 Rust 文件 rustfmt/diff 检查通过。
- 扩展全包测试的 lib456项及此前多个 integration target 通过；`message_v3_artifact_schema` 5项因写死的历史客户端路径 `/System/Volumes/Data/workspace/angular/visual/cses-client/specs/...` 不存在而失败，未修改断言或扩大修复。后来新增的第4项定向用例已另跑通过；不宣称全包通过。
- 客户端正式 pin 0.1.39，源码0.1.41：真实桌面候选使用 `/tmp/cses-emoji-candidate` 临时 manifest接本地源码，不修改客户端正式 Cargo.toml/Cargo.lock；数据库为原库只读快照 `/tmp/cses-emoji-candidate/emoji-live.db`。
- 真实 UI/记录验证状态及脚本见客户端 `e2e-desktop/specs/emoji-history-shared-ui.regression.md` 和 `artifacts/reaction-menu-wdio/`，与本仓 SQLite 验证分开报告。
- 无版本变更、无发布、无 push。正式交付仍需授权的新版本及客户端依赖升级。

- 独立候选实机发送表情 grinning、grinning、smile 的原生输入成功，最近顺序 smile、grinning；回应 +1 使用真实 Host Port 记录，未更改远端回应。候选进程重启后两类均读回，草稿为空。组合 UI 自动化偶发超时与原生点不中，不能称为稳定或远端业务全链路验收。
