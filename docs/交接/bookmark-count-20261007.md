# 书签计数缓存（2026-10-07）

基线 reactor/MessageV3-helix / ff6292bb。用户已批准跨仓协议及缓存方案；未改版本、driver、core、Gate 分母或发布。

入口 `Tick::Command(im_bookmark_count,{req_id})`：本地会话缓存命中直接返回现有 `im:read:result`；否则合并等待者，通过 HTTP `/post/bookmark/count` 校准。响应必须 SUCCESS、userId/companyId 匹配且 total 非负。持久化 N/A：计数是可校准会话缓存，不是同步 cursor/消息事实。

create/delete 的原读回路径保留，新增消费 Go `affectedCount/countDelta`；只应用一次实际有符号增量。缺字段、失败、负数下溢不猜增量，而是失效。校准发出后有任何写入，拒绝旧快照，等待写入结束重新读取。重复 PortReply 被 corr_map 消费机制丢弃。

首次、断线后下次读、五分钟过期时校准；错误三十秒退避。账号/企业切换及 stop 清理全部 coalesced 等待者和旧 correlation，A→B→A 的迟到结果不晋升。HTTP/超时沿用 driver PortError；非法输入返回 ModuleError；无 Persist/WS mutation 副作用。其他设备/权限变化靠五分钟兜底，不声明实时 WS 同步。

验证：先运行公开 seam 红测，缺命令时失败（/tmp/bookmark-count-helix-red.log）；后 `cargo test -p helix-im --test read_relay_test --test p3a_posts_read_test` 64 项通过，覆盖缓存、并发合并、写读两种交错、重复回包、增减零变更、scope/输入错误、失败退避、stop、断线和身份切换。`cargo check -p helix-im`、局部 rustfmt、diff 检查通过，保留九条既有 warning。

证据是 sans-IO 模块回放；Go 隔离 SQL 验证在 Go 仓，不能当成真实网络/SQLite/Tauri 全链。当前运行实例没有接入本候选，也没有重启或占用其他会话实例。新 Go、Helix 和客户端需一起接入后完成隔离实机验收。

生命周期和状态实现位于 `crates/helix-im/src/bookmark_count.rs`，共享读取与取消接线位于 module/trait_impl.rs、port_reply/dispatch_read.rs。复用现有 Harness 约束，无新约束卡。
