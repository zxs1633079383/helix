# 成员投影冷恢复修复（2026-10-12）

状态：实现与定向验证完成；本批按用户授权整理提交，未 push、提版或发布。新 Go 上线后的完整冷恢复仍待复验。

## 行为

- Go increment 的 `memberProjection` 复用 canonical 21 字段合同。Helix 校验完整性、当前 user/channel、正 i64 revision/effect，以及外层 revision/readSeq/unreadCount 一致后写入 canonical 成员状态。
- 旧 Go 只有外层字段时复用 legacy 显示写，不把外层 revision/effect 当完整水位。初次恢复仍显示已给出的普通未读、回应计数和摘要；仅 revision=0 的行可按原 event-seq 守卫更新，已有完整态不会被旧快照覆盖。纯主键 seed 不旁路覆盖摘要。
- 已受影响的同版本空 effect 本地行，只能由完整权威字段在同 user/channel、恰好同 revision、effect='' 下原子补齐。不同非空 effect、较高版本和普通稀疏投影不能触发修复。正常新版本继续严格 Greater，未改成 >=，不做 revision+1。
- 频道分页从持久化成员态读回；空 effect 或零版本不对外携带 projectionRevision，防止 UI 把不完整显示态当作已完整应用。补齐后同页及频道更新事件携带真实 revision 和 reactionUnreadCount。
- query/ack 原有 PersistOk→真实读回→revision/effect/字段相等校验保留。较新推送在写后读回前到达，仍返回 `REACTION_UNREAD_PROJECTION_CHANGED`，不返回过时成功。
- mentionUser/urgentMentionUser 的空串作为明确清除值落库，避免“完整字段”被忽略为空缺。

## 存储与性能边界

`ScopedGuardedBumpSpec` 增加纯数值 `GuardComparison::{Greater,Equal}`，默认枚举值 Greater，原有调用方全部显式 Greater。Host 非原子/原子 SQL、Web Session Store 统一执行；这是内存 Rust Effect 合同，没有序列化 wire 变更。Native/FFI 共用 Host。

修复使用已有复合主键定位单个 viewer/channel，完整投影多一次条件 UPDATE，不增加查询、不扫描全群、不新增表/列/迁移、不使用应用层读改写。完整字段的 payload 大小仍随既有列表内容变化。单群5000人、每群5RPS、多大群、1秒可达是目标，本次未压测，不能据定向测试宣称已达标。全局性能规则指向的 xpa-12 路径及本机同名规则文件不可用，未伪称已读取。

## 验证

证据：`artifacts/member-projection-cold-recovery-20261012/`，源码身份见 `source-identity.json`。

- 红测：原实现未写完整 effect、旧快照抢占 revision、同版空 effect 不自愈；兼容补充红测显示计数0≠7。
- 最终真实 SQLite increment target：16通过，1个存量失败。新增7条 cold 测试涵盖完整快照、21字段逐一缺失、跨身份/版本不符、legacy显示及完整态保护、同版空 effect/不同 effect/旧新版/i64最大值、query/ack 真实持久化和读回竞态、群外分页计数与版本。
- `cargo test -p helix-im --lib`：471通过。
- G13 + HTTP sync envelope + increment flow：按日志全部通过（11/5/8项）。
- G08 + GS1/GS2/GS3/GS4：13通过。
- Host storage conformance：23通过；Web Session Store：27通过。
- core tests：28普通测试通过；doc test 1通过、1个既有ignored。
- 三端 `cargo check`（core/im/native/ffi/web）通过；Web wasm32 build通过；core dependency tree无tokio/sqlx/reqwest/web-sys/tauri。
- `git diff --check` 通过。

全量限制：

1. `deleted_channel_pages_continue_to_live_tail_without_zero_revision_items`：删除频道分页返回20条而预期0。相同测试在隔离 git archive HEAD `81f5dcc182d477694d53cd73503133b96a31ca39` 同样失败，未扩修。
2. 全量 helix-im 首次因 Phase1 reference_root 指向不存在的旧机器路径失败。使用既有 MESSAGE_V3_PHASE1_REFERENCE_ROOT / MESSAGE_V3_PHASE3_CLIENT_ROOT 指向本机真实客户端后，该路径问题消失；随后 `message_v3_mv3_g05a::outbound_reference_event_names_match_the_runtime_projection` 出现字段合同漂移（多 id/userId）。隔离 HEAD 对应单测同样失败，未修改参考样本或放宽断言。全量不宣称绿色。

## 客户端候选

本机四个公共 crate 仍为0.1.55：`/Users/mac17/workspace/rust/helix/crates/{helix-core,helix-im,helix-driver-host,helix-driver-native}`。隔离客户端构建须 patch 全部四个包并核对 Cargo metadata 解析到这些本地路径，不能仅看 manifest 判断已使用修改。

旧线上 Go + 新 Helix 可显示 legacy 快照给出的计数并在 query/ack 同版自愈；完整冷快照及其 authority 字段仍待 Go 本次修正部署。禁止逐群 query 伪装群外冷恢复通过。未执行真实用户本地库修复或线上迁移。

## 提交前核验

2026-10-12：11个代码/测试文件与 source-identity.json 完全一致；重新执行真实SQLite cold_ 7项全通过，core/im/native/ffi/web cargo check通过。其余已通过检查按相同源码复用，前述两项存量失败保留。客户端在线主/独立窗口回应跳转已验收；新Go完整快照尚未上线，冷恢复和大群性能目标未据此认领。
