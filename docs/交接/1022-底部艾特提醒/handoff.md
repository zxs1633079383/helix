# 底部艾特提醒交接
更新时间：2026-10-22，Asia/Shanghai

## 目标与边界
- 修复实时 mentions 遗漏；旧 @ 清除后新 @ 能重新出现；会议/任务点击移除不变。
- 当前新增授权：发布Helix新patch，客户端dev-message/dev依赖同步，提交并push；不构建安装包。发布及双分支push完成；不构建安装包。
- 单群5000人、每群5消息RPS、多群并行、1秒可达为目标，本轮未做容量验收。

## 当前代码身份
- Helix `/System/Volumes/Data/workspace/angular/visual/helix`，reactor/MessageV3-helix。
- 验证基线 `6dc8324121ff787179cd2a38060438175537311c` 加本批 diff；提交从本文件所属 commit 查询。
- post.rs SHA256 `fa5715708b208aec1787ec42af58c645c9998509e209eb988c02be952a8f7523`。
- empty_json_delivery.rs SHA256 `d5a075a9647de5425a409b1decfd1dcf5385f54e1bd675741dd8feeba5e50bf8`。
- 客户端 `/System/Volumes/Data/workspace/angular/visual/cses-client`，dev-message，`a87f1d91a1557a59829fa947e2823c952a06b08e`；回归提交3771a8d169。
- 验证期间客户端基线从86f4c3fd推进至822e13ca，仅其他任务的投票修改；通知源码无变化，证据可复用。
- 原有未跟踪资产保留；本任务只暂存精确白名单。

## 实现与证据
- authority_projection 直接输出 fields.mentions，空数组显式保留；其他生产行为不变。
- 新增真实 Tick → 原子 SQLite → PortReply → received 回归；存储失败/重复回声沿既有套件验证。
- 客户端 App 回归执行生产方法体，宿主端口隔离；fixture由上述Rust测试实际Emit生成，非手写业务事件。
- fixture SHA256 `6be3c5704e51c83611427ec483dbf63a7ea31dce864f7004190dde9e5dea2c74`。
- 证据根目录 `artifacts/1022-mention-delivery/`（本地保留）。

| 验证 | 命令/结果 | 日志 |
|---|---|---|
| 红测 | cargo test -p helix-driver-host --test empty_json_delivery mentions_survive_persist_and_received_for_footer_notifications；SQLite正确而received为Null，失败 | red.log |
| 绿测 | cargo test -p helix-driver-host --test empty_json_delivery；8通过 | host-green.log |
| IM | cargo test -p helix-im --lib；501通过 | im-lib.log |
| 多端 | cargo check -p cses-helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-web；通过 | check.log |
| core | cargo test -p cses-helix-core；29通过、1既有doc忽略 | core.log |
| Web | cargo build --target wasm32-unknown-unknown -p helix-driver-web；通过 | wasm.log |
| 客户端 | Node22.23.1直接执行Jest，footer-mention-delivery + message-notice-todo；7通过 | client-green.log |
| 可证伪性 | 仅移除第二条新事件mentions，旧清除后新提醒测试失败；恢复fixture后7项通过 | client-mutation-red.log |
| 边界/热路径 | check-package-boundaries.mjs、audit.sh post.rs、diff --check通过；core tree无runtime依赖 | boundaries.log/hot-path.log/core-tree.log |

## 复现与后续
- fixture再生成：设置 HELIX_MENTION_FIXTURE_PATH 为客户端 app/tauri/__tests__/fixtures/helix-mentions-received.json 绝对路径，运行上述Host单项测试。
- 客户端执行：node node_modules/jest/bin/jest.js --runInBand --runTestsByPath app/tauri/__tests__/footer-mention-delivery.test.ts packages/message/src/lib/services/__tests__/message-notice-todo.test.ts。
- 热路径新增工作为序列化现有mentions，O(@人数)，不展开全群、不增SQL、不改持久屏障。
- 扩大回归 client.log：16通过1失败；既有footer-quick-chat-panel.test.ts:95精确匹配旧模板，模板已含data-tauri-drag-region，两文件与基线一致；未修改无关断言。
- 真实Go/WS、安装版Tauri/UI均NOT_VERIFIED；两客户端分支已pin 0.1.85，未构建或交付新安装包。
- 真机后续步骤：他人发送旧@→主消息页打开该会话清除底部项→离开后发送新@→检查底部重新出现并能定位；同时检查会议/任务点击单条消失、关闭浮窗不直接删@。
- 仅内存SQLite与测试fixture，无真实业务资源；故障注入fixture已恢复。

## 发布结果
- 四包0.1.85发布crates.io并可解析；版本提交 `6bc5ca0a3a5941544b4c38ce7bafb2b2fe4918db`。
- 注释Tag `helix-v-0.1.85` 已push，远端peeled SHA等于版本提交；源码分支已push。
- 客户端dev-message远端 `a87f1d91a1557a59829fa947e2823c952a06b08e`。
- 客户端dev远端 `5c675fd22fc166339c8bd3c2541e577011d72a19`；合入本轮4文件，保留dev独有历史。
- 发布脚本候选八crate编译、四包dry-run/上传通过；下载制品checksum=registry索引=客户端lock，发布包post.rs与修复源码一致。
- 客户端lock仅四包版本/checksum变更，cargo tree --locked解析通过；dev合并后同组7测试通过。
- 发布证据仍在上述目录：release-plan.log/release.log/registry-checksums.json/client-lock.log/client-resolution.log/dev-client-green.log。
- 临时dev工作树 `/tmp/cses-client-mention-release-dev-20261022` 在远端核对后移除；保留分支，不影响主工作区。
- 构建安装包、真实UI/Go链、远端CI结果均未验收；先前无关footer精确字符串断言失败保持单列。
