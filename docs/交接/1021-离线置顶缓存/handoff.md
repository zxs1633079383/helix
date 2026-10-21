# 离线置顶缓存修复交接

- 目标：6个置顶红测转绿，真实SQLite跨队列逆序/回滚/重开，继续核验其他channel风险；本地提交，不发布。
- 仓库/分支：Helix `reactor/MessageV3-helix`；验证基线`f690b9662569dc27c720b2937db9839c09f6327c`加本批diff。
- 源码diff+新增SQLite测试指纹SHA256：`f41ed11881ebd97f44fd9672798ddf2e38e0d2c858ecf92aeb5a80e48548d313`；最终提交由本交接所在commit定位。
- 当前：置顶实现及定向验证完成，可提交；独立共享字段缺口已复现、未修复。详见[审查.md](审查.md)。
- 实现：新增账号/频道持久fence表；失效与恢复事实原子提交；HTTP成功读取revision后CAS写列表+计数；全部异步边界epoch拒旧，重连不归零。
- 同步入口：普通sync、terminal、Snapshot、TooLong、increment；在线post_pin/NOTICE同样失效。无版本发布或core/driver原语变更。
- 聚焦命令：`cargo test -p helix-im --test read_relay_test --test p4_ws_events_test --test increment_flow_test --test channel_write_program_test --test http6_sync_reply_unwrap_test --test s6_sync_internal_test`，94通过。
- `cargo test -p helix-im --lib pinned_projection`：5通过；`cargo test -p helix-driver-host --test pinned_cache_fence`：4通过。
- SQLite覆盖：不同table-set写集逆序；CAS拒绝旧写；重开后仍拒绝；列表/失效事务后段失败均回滚body、revision、cursor。
- 五crate check、Web wasm32 build、core28项+doc1项通过；1项既有doc忽略。core依赖树零运行时；热路径脚本与diff check通过。
- 证据：`artifacts/1021-pinned-sync/`，红/绿日志及独立探针；平台检查后的非平台差异及复验说明见审查。
- 其他属性：共享群名旧快照覆盖新PATCH已用真实SQLite复现，`channel-scalar-red.log`及同目录probe源码；不计入置顶绿测。话题/未读有版本机制；公告低版本拒旧已测，客户端也监听消息更新重拉。
- 下一步：普通共享字段需独立权威版本/稀疏兼容合同及修复；真实Go/Tauri/双设备UI与容量未验收。
- 授权/风险：只本地提交，不push、不发布。红色独立审查探针移存证据目录，未伪称修复；测试仅清理run-owned临时SQLite文件。
- 性能约束：单群5000人/每群5RPS、多群并行、1秒可达是目标，未实测；新增主键操作不随成员数放大。
