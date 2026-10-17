# 置顶写结果回传

🎯 EXPECT：复用撤回的 correlated HTTP result，pin/unpin 在 Go 拒绝、网络失败及取消时结束等待，不把 accepted 当完成；不生成置顶事实或 NOTICE。当前分支 reactor/MessageV3-helix，用户授权跨仓修复，不提交、不提版、不发布。2026-10-18。

| 生命周期 | 合同 |
| --- | --- |
| 入口 | im_set_message_top / im_set_message_untop；channel_id、post_id、可选 req_id；ImModule::handle |
| 分类 | 远程写；HTTP result 只确认远端业务结果，持久化置顶事实仍沿用 WS/Gate |
| 成功 | 同 req_id 的 im:read:result，Go status SUCCESS；无本地伪 NOTICE |
| 失败 | 非法字段无 HTTP；HTTP/网络/超时/业务失败回传 error；on_stop 取消等待 |
| 关联清理 | corr_map 一次结算，重复/迟到不重复结果；Host waiter 复用 QueryObservation 的 timeout/drop 清理 |
| Persist | 此 HTTP 确认不产生本地 Persist，PersistErr 属现有 WS/Gate 链，N/A 于本次结果回传 |
| 验证 | 公开 seam 先红后绿；本地候选真实 Tauri + Go 验证，发布依赖仍待版本授权 |

## 验证记录

- `cargo test -p helix-im --test read_relay_test`：40 通过，新增 pin/unpin 公共入口覆盖业务拒绝、网络、超时、stop、重复终态及 req_id 校验。先红后绿日志在 CSES `artifacts/pinned-interaction-20261017/helix-{red,green,relay}.log`。
- `cargo check -p helix-im`：通过，9 条已有 warning。
- CSES 真实 Tauri 本地 patch 候选：群主三条置顶/回读/取消通过；普通成员 pin、unpin 均得到真实服务端权限拒绝，未伪造置顶或移除原条目。见同目录 `final-owner/live-result.json`、`final-member/denial-result.json`。
- 未提交、未发布。CSES 默认仍精确依赖 registry 0.1.72；候选的本地 patch 仅用于验证，不代表默认依赖已交付。版本路线已询问用户。

## 发布授权（2026-10-18）

用户明确选择“Helix 升 patch 并发布，再更新 CSES 精确依赖”。plan核验当前0.1.72四包已发布、0.1.73四包均未占用。沿用脚本拓扑发布及仓内版本事务，按发布规则提交修复/版本并创建注释Tag，不push。发布前新增完整IM与三端检查，已验证的局部真实桌面证据继续复用。

发布检查：五crate check通过；core 29通过/1文档忽略；WASM构建通过。IM全量库491通过，初次因旧绝对参考路径停止；使用支持的本机参考根后推进至G13a，3项会话列表断言失败。独立registry测试夹具直接依赖已发布0.1.72，运行相同G13a测试，同样4通过/3失败（filter与scope_values断言），确认为存量，不扩大修复。全量未全绿；发布继续使用本次置顶40项及真实桌面成功/拒绝链证据。日志见CSES artifacts/pinned-interaction-20261017/release-*.log 与 registry-baseline/test.log。
