# MessageV3 统一事件内核收敛交接

## 当前结论

Spec 051 的实施 Issue `#135`～`#142` 已完成代码收敛、focused review、Native SQLite 与当前 Go 18066 gap 真实链验证；GitLab 实时核对确认父 Issue `#134` 与全部子 Issue `#135`～`#142` 均为 `closed`。

本结论只覆盖 Spec 051 的统一事件应用内核，不代表 MessageV3 Phase1～4 全部 80 Gate 或文字接龙 scoped 8 Gate 整体 Green。客户端整体 review 仍为 `PARTIAL`，具体阻断见“整体 Review 边界”。

工作路径：

- Helix：`/System/Volumes/Data/workspace/worktrees/sync-event-alignment/helix`
- 客户端：`/System/Volumes/Data/workspace/worktrees/sync-event-alignment/cses-client`
- Go：`/System/Volumes/Data/workspace/worktrees/sync-event-alignment/cses-im-server`

三仓主目录均未修改；Go 与客户端 worktree 没有业务源码 diff。

## 发布与客户端消费

- Helix `0.1.2` 已由 `scripts/publish_crates.py --publish` 按 `core → im → host → native` 顺序发布；`cses-helix-core`、`helix-im`、`helix-driver-host`、`helix-driver-native` 四包均经 registry 轮询确认可解析。
- Helix 8 个 workspace manifest 与 `Cargo.lock` 已同步到同批依赖图，发布提交为 `c9643f27`；未创建 tag、未 push。
- 客户端 worktree 已把四个 Helix 依赖切到精确 crates.io `=0.1.2`，提交为 `27583637e6`；`cargo metadata` 确认四包 source 均为 registry。
- 用户级 `.zshrc` 中唯一持久 `HELIX_ROOT` 已移除，新登录 shell 为 unset；常规启动包装器 `--check` 通过，不再把 registry manifest 临时切回 path。
- 客户端完整 `cargo check` 已解析并编译 Helix `0.1.2`，随后因既有本机资源 `resources/macos/staged/libonnxruntime.dylib` 缺失而停止；这不是 Helix registry 解析失败，客户端整包仍不能据此判 Green。

## 最终实现

1. 在线 type=1 与 Routine Sync 共用 `message_v3_post_mutation_ops`，同一 `PersistAtomic` 写入 message、共享 channel 摘要、当前 viewer `channel_member`、coverage/ledger 与 cursor。
2. viewer 未读以 `(channel_id,user_id)` 为唯一作用域，新增 `last_unread_event_seq` 作为业务序号 guard；同毫秒、重复、乱序事件均由 SQLite 严格前进条件约束。
3. 空 viewer identity 与超出 SQLite INTEGER 范围的 eventSeq fail closed，不再创建空 `user_id` 成员行或产生负 guard。
4. 本端 gap echo 不再提前 `PersistFire + Emit`，必须等待与远端 post 相同的 gate、原子提交和读回屏障。
5. ordinary type=2 编辑与 reaction/urgent/template 共用 MessageV3 buffer；前置事件提交后，buffer 中 edit 形成新的 `PostUpdateAtomic`，matching `PersistOk` 后才推进 cursor、Emit 并继续下一条。
6. Hydration 有 increment unread authority 时由 per-viewer guard 阻止历史重复计数；缺少 authority 时由 history compiler 重建 durable viewer 未读，但仍抑制逐消息业务动画。
7. increment viewer snapshot 使用 seed + guarded absolute set，旧/重复快照不会回退 unread/guard；global increment 批次由非原子 `Persist` 改为 `PersistAtomic`。
8. schema migration 只新增 guard 列且默认 0，不把 channel-scoped cursor 复制成所有 viewer 的成员态 authority。

## Issue 关闭证据

| Issue | 关闭证据 |
|---|---|
| `#135` | online/Sync type=1 统一 compiler；viewer scoped readback；sender/receiver/echo 语义测试通过 |
| `#136` | 小 gap、大 gap、PersistErr、错误/重复 corr；Native cursor 与 unread 1→2 |
| `#137` | PongGap 合并读回、Hydration authority/无 authority、旧 schema migration 与重启 SQLite |
| `#138` | ordinary type2 连续与 gap 释放均只走 `PostUpdateAtomic`；失败不 Emit |
| `#139` | type3/type6 混批、重复执行、缺失 revoke target 幂等 no-op，未读保持不变 |
| `#140` | 1/10/100/500 每批恰一业务 `PersistAtomic`，写集 O(N)，失败与读回错误 fail closed |
| `#141` | 当前 18066 run-owned 群、真实 HTTP/WS、Helix、SQLite、EventSink gap 链；最终 unread 1→2；Go tracked diff=0 |
| `#142` | Standards/Spec focused review 已处理 self-echo、legacy type2 gate、Hydration、viewer scope、migration、atomicity、overflow 与旧测试身份缺口 |

## 本轮验证

通过：

```text
cargo test -p helix-driver-native --test message_v3_sync_native -- --nocapture --test-threads=1
13 passed, 0 failed

cargo test -p helix-im --test engagement_post_update_projection_test
5 passed, 0 failed

cargo test -p helix-im --test message_v3_mv3_g13b
10 passed, 0 failed

cargo test -p helix-im --test s5_post_echo_test
5 passed, 0 failed

cargo test -p helix-im --test message_v3_g06
4 passed, 0 failed

cargo test -p helix-im --test increment_flow_test --test golden_replay_test
11 passed, 0 failed

cargo test -p helix-im -- --nocapture
all unit, integration, stress and doc tests passed

cargo fmt --all -- --check
passed
```

`message_v3_sync_native` 的 13 项包含：当前 Go 18066 gap 链、旧 schema migration、同毫秒 guard、小 gap、失败/重复 corr、PongGap、type2、type3/type6、1/10/100/500 复杂度和成员读回失败。

原先阻断全包的旧测试构造已按当前身份合同补齐 `client_platform`，并同步修正全量运行暴露的旧事件名、查询投影、权威 WS echo 与原子持久化断言。`cargo test -p helix-im -- --nocapture` 现已全绿，包含 100,000 条顺序事件、5,000 条 WS gap flush、5,000 条 sync backfill 等 stress 场景；两个既有格式问题也已由 `cargo fmt --all` 收敛，`cargo fmt --all -- --check` 通过。

补充执行 `cargo test --workspace` 时，本地 unit/integration targets 均已通过；`helix-driver-native --test message_v3_domain` 的 73 个真实链 Gate 因默认访问 `http://127.0.0.1:8066/api/cses/health` 而被外部前置阻断。当前 Go worktree 实际由 Docker Compose 运行在宿主 `18066`（映射容器 `8066`，health 200），因此这是测试入口地址尚未显式对齐的运行配置缺口，不是本轮 Helix 单元、stress、格式或 Spec 051 focused 链失败，也不能据此宣称 Phase1～4 整体 Green。

## 真实链边界

当前 `#141` 证明的是“在线首条 + 中间事件缺失 + Sync gap 恢复”，不是物理断网、进程重启或客户端 UI 离线恢复。HTTP health 200、WS 建连和 Cargo Green 都没有单独计作业务 PASS；PASS 由同一 run 的 Go HTTP/WS、Helix correlation、Native SQLite readback 和 EventSink 共同构成。

## 整体 Review 边界

客户端 Phase manifest 机器分母校验通过：Phase1/2/3/4 分别为 33/20/15/12，合计 80，open decision 为 0；Phase5 scoped 分母为 8。

但整体 Review 仍为 `PARTIAL/BLOCKED`：

- `yarn message-v3:hard-cut` 为 7/8，现存 `src/app/core/services/tauri/im-event-bus.ts` 的 `messageV3RawEventConsumer=1`。
- Phase1～4 contract 分别存在 6/3/8/2 个失败，主要是 worktree 缺 `.wdio/credentials.local.json`、`tsx`、node_modules 或路径前置；coarse acceptance 24/24 通过不能替代真实 Gate。
- Phase2～5 typecheck 因 `tsc: command not found` 未形成有效证据。
- 当前 ignored `release.local.json` 已将 `cses.dev.imApiHost` 配为 `http://127.0.0.1:18066`，Phase1/2/3 release-local 均解析为 `http://127.0.0.1:18066/api/cses`；对应 Go worktree Docker health 为 200。该健康检查只证明前置可达。
- WDIO 仍缺 `node_modules`、只读 `.wdio/credentials.local.json`、candidate、`dist/browser`、完整 Xcode 与 ONNX runtime staged dylib；Phase1/2/3 candidate builder 还会为测试候选临时使用 sibling Helix path，不能把该候选当作已发布 registry `0.1.2` 的消费证明。
- 本轮未运行完整 WDIO real lane，Phase1 文档仍明确 G01i/G01j 未获真实 lane 证明。

因此关闭 `#134`～`#142` 只表示 Spec 051 的 focused 实施与 review 完成，不改变上述 Phase Gate 的非 Green 状态。
