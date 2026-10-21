# 频道共享属性防回退

- 目标：群名红测转绿，共享字段审查，真实SQLite跨队列乱序/回滚/重开；已追加授权发布并更新客户端dev-message/dev。
- 仓库/分支：Helix `reactor/MessageV3-helix`；基线`9c9fef1158d7d93b911f91c73ea9ddaa65fa8709`，开局干净。
- 源码diff+新增测试指纹SHA256：`17e6dca8db112953c384fdb1fb9a81d4e057d4b1c439fe8ccf47b64efff8bfec`；最终提交由本交接所在commit定位。
- 当前：本批实现与本地验收完成；所有channel乱序风险未宣称全关闭，残余见[审查.md](审查.md)。
- 实现：独立`snapshot_updated_at`水位与迁移；共享属性条件UPDATE，成员/计数独立；无类型快照不插入空频道；缺省保留、显式空/0/false有效。
- 入口：WS普通PATCH、increment/hydration、member携带完整channel和HTTP设置authority。HTTP旧回包另有SQLite红测，已统一守卫与读回。
- 回调：原子写成功后精确读channel；数据库值/版本与已观察版本/代次共同拒旧；同版本迟到回调也不发布旧成功。HTTP维持channel-update出口。
- 归属：RenderScope仅设置与补水冷路径登记精确读回；普通消息不增加整行解析。归属读回也拒绝较低持久版本。
- 证据目录：`artifacts/1021-channel-ordering/`；群名`red.log`与HTTP`http-red.log`均先失败在旧值覆盖断言。
- 最终`cargo test -p helix-driver-host --test channel_scalar_ordering_probe --test pinned_cache_fence`：9+4通过，`sqlite-final.log`。
- SQLite覆盖：不同table-set逆序、两向snapshot/PATCH乱序、迁移、旧/无版本拒绝、稀疏/同版本补齐、独立计数、晚到回调、HTTP成功/失败、事务后段失败回滚、文件重开。
- 最终IM：`cargo test -p helix-im --lib --test message_v3_mv3_g09d --test message_v3_g16c --test message_v3_mv3_g10d --test render_scope_test --test scope_hydration_test --test p3b_channel_outbound_test --test channel_empty_semantics_regression_test --test p4_ws_events_test --test one_line_server_contracts_test --no-fail-fast`，604通过，`im-focused-final.log`。
- 先前广扫：`cargo test -p helix-im --no-fail-fast -- --skip stress_inorder_100k`，1862通过/1旧结构断言失败/11忽略；失败目标随后修正并纳入上述复验，不能把广扫冒称最终源码全量实跑。
- `stress_inorder_100k`已在更早本轮广扫通过；不为收尾重复运行，非真实容量验收。
- 五crate check通过（core/im/native/ffi/web，`check-final.log`）；Web WASM build通过（`wasm-final.log`）。
- core28+doc1通过/1doc忽略（`core.log`）；Web新增>=守卫测试1通过（`web-guard.log`）。这些通过后的相关core/Web守卫代码未变，复用证据。
- `git diff --check`通过；core依赖树零运行时；热路径脚本对HEAD作函数/模式差异核对，无新增命中，已有命中未伪称清零（`hot-path-delta.log`）。
- 风险：Go时间戳非严格revision；同毫秒不同内容、纯无版本写之间不能证明持久全序。专用close/leave、make-topic等与共享更新交叉未验收。
- 实现/本地验证已完成；真实Go/Tauri/双设备UI、CI、容量未验收。发布状态见下。
- 规模约束：5000人/群、每群5RPS、多群并行、1秒可达是目标；本批每频道固定主键操作，不按成员展开。

- 发布完成：四公共crate 0.1.82已发布crates.io且可解析；版本提交311caa9e753570046ae606a6494fb9b1b4727389，注释Tag helix-v-0.1.82及reactor/MessageV3-helix已推送回读。
- 客户端dev-message：7f117ada52ac8c75ed7d2ebfc35024a5eb2186db；dev：9eb2f52320f4d6e6e186b2f99b9f69dc5b113fca；两个远端分支均回读一致，仅四依赖与lock更新。
- 发布校验：workspace八crate check、四包dry-run/上传/registry解析通过；release-0.1.82.log。
- 客户端：cargo update仅更新四包；cargo check --locked -p helix-driver-native通过（2m15s）；两分支manifest/lock逐字节相同，dev离线locked metadata通过，四包archive checksum匹配lock。证据client-082-*.log及client-dev-082-metadata.*。
- 本轮无安装包构建/部署，CI及真实Tauri/UI未验收；共享客户端脏改动未夹带提交。
