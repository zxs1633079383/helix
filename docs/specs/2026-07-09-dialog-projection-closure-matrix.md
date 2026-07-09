# Dialog Projection Cross-Repo Contract Matrix

日期：2026-07-09

## 目标

收口 Go、Helix、mobile、cses-client 四仓在 DialogProjection / sync reset 上的职责边界、验证口径与剩余 partial，避免把 focused gate、mock-only 或 DB integration SKIP 误写成 live green。

## Contract Matrix

| Contract | Go | Helix | mobile | cses-client | Result |
|---|---|---|---|---|---|
| post `eventSeq/event_seq` | 在线 `publishPostEvent` 同时发 camelCase `eventSeq` 与 snake_case `event_seq`，保留兼容锚点 | `emit_channel_update_by_post` / `emit_dialog_patch_for_post` 同时消费在线 post 与 sync type1 口径 | 只校验 schema 并透传，不做 seq 补算 | 只消费/渲染，不自行补齐 seq | PASS |
| viewers / phantom | WS fanout 用 `viewers + sender` 收窄；sync 只过滤 `messages` map，不过滤 `events` | gate 与 sync 本地二次可见性复核；phantom 只推进 cursor，不落可见 message / DialogPatch | 不做可见性业务计算，只接收 Helix 结果 | 不做可见性业务计算，只渲染 Helix 投影 | PASS |
| unread / mention / urgent counts | 提供 `channel_member` / `channel` 事实源与事件上下文 | 写后读 channel 累计终值后再 emit `dialogPatch`，不把 delta 伪装成累计值 | render-only；字段存在则落本地 VM，不 `+1` | render-only；字段存在则直接渲染，不 `+1` | PASS |
| `too_long` | sync gap 超阈值时返回 `kind=too_long` 与 `resetTo=maxSeq` | drop local messages + emit `im:sync:too_long(mode=drop_and_reload)` + 主动 `getLatestPost` reload | 只执行 loading/reset UI 语义，不发 HTTP 拼装 | 只执行 loading/reset UI 语义，不发 HTTP 拼装 | PASS |
| `readLastSeq` / `msgCount` | 来源仍在 `channel_member` 事实层 | `NEED_HELIX_CHANNEL_MEMBER_FIX.md` pending；当前 render-ready `dialogPatch` 故意不输出 | 仅 optional 透传；缺字段合法，不伪造 `0` | 仅 optional 透传；缺字段合法，不伪造 `0` | PARTIAL |

## 验证命令矩阵

| Repo | Command | Status | 说明 |
|---|---|---|---|
| Helix | `cargo test -p helix-im --test dialog_projection_test -- --nocapture` | PASS | DialogPatch 主链路回归 |
| Helix | `cargo test -p helix-im --test projection_schema_test -- --nocapture` | PASS | schema 与 camel/snake 锚点冻结 |
| Helix | `cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture` | PASS | sync type1 visible / phantom contract |
| Helix | `cargo test -p helix-im --test s6_sync_internal_test -- --nocapture` | PASS | `too_long` drop-and-reload |
| Helix | `cargo test -p helix-im --test http6_sync_reply_unwrap_test -- --nocapture` | PASS | sync reply unwrap |
| Helix | `cargo test -p helix-im test_gate_buffer_capped_proactive_too_long_e6 -- --nocapture` | PASS | proactive too_long gate |
| Helix | `cargo test -p helix-im proactive_too_long_requests_latest_post_reload_e6 -- --nocapture` | PASS | too_long 主动 reload |
| Helix | `cargo test -p helix-driver-host --test storage_conformance guarded_bump_extra_counts_share_the_same_guard_s3 -- --nocapture` | PASS | 同 guard 计数累计 |
| Helix | `cargo test -p helix-driver-host --test storage_conformance delete_where_deletes_only_matching_scope_rows -- --nocapture` | PASS | drop local messages 只删目标 scope |
| Helix | `cargo test -p helix-driver-ffi --test mobile_contract_test mobile_im_send_alias -- --nocapture` | PASS | Task 2 `im_send` alias 归一 |
| Helix | `cargo test -p helix-driver-ffi --test mobile_contract_test -- --nocapture` | PASS | Task 3 `helix_capabilities` / `helix_version` |
| Helix | `cargo check -p helix-driver-ffi` | PASS | FFI 边界编译检查 |
| Go | `G1` 在线 post / messages map / read / too_long mock | PASS | 见下方 Go 命令块 |
| Go | `go test ./internal/app/sync -run TestPostToSyncMessage_KeepsHelixProjectionFields -count=1` | PASS | sync message 保留 Helix 需要字段 |
| Go | `G2` viewers fanout 收窄 | PASS | 见下方 Go 命令块 |
| Go | `G3` DB integration 条件验证 | SKIP | 未配置 `CSES_IM_TEST_DSN`，只能记为 DB integration SKIP，不是 live green |
| mobile | `./scripts/run-gtest-js-spec.sh` | PASS | DialogProjection JS/C++ focused gate |
| mobile | `make real-chain-manifest` | PASS | manifest 46/46 + dialog projection cases |
| mobile | `make gtest-js-spec` | CONCERN | pinned Helix rev 与共享 checkout 不一致，脚本主动拒绝；非本轮 contract 失败 |
| cses-client | `npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts src/pages/message-v4/__tests__/projection-handler.registry.test.ts --runInBand` | PASS | render-only focused store / registry tests |
| cses-client | `/System/Volumes/Data/workspace/angular/cses-client/node_modules/@playwright/test/node_modules/.bin/playwright test -c tests/e2e message-helix-hard-cut.spec.ts` | PASS | message-v4 静态 gate |
| All relevant repos | `git diff --check` | PASS | Helix / Go / mobile / cses-client 本轮 focused diff 均过格式检查 |

### Go 命令原文

```bash
# G1
go test ./internal/app -run 'TestPublishPostEvent_IncludesHelixProjectionFields|TestBuildMessagesMap_NilPostSkip|TestSyncChannels_(TooLongMock|ReadEventNoMessageBodyMock)$' -count=1

# G2
go test ./internal/ws -run 'TestResolveViewers_(AllBranches|PointerStringSlice|PqStringArray|UnknownShape)$' -count=1

# G3
if [ -n "$CSES_IM_TEST_DSN" ]; then
  go test ./internal/app -run 'TestSyncChannels_(KindEvents|KindTooLong|EventsEmptyMessages)$' -count=1 -v
else
  echo 'SKIP: CSES_IM_TEST_DSN is not set'
fi
```

## Pending / Concerns

| Item | Status | Evidence |
|---|---|---|
| `readLastSeq` / `msgCount` Helix `channel_member` schema + write path | NEED | [NEED_HELIX_CHANNEL_MEMBER_FIX.md](/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/NEED_HELIX_CHANNEL_MEMBER_FIX.md) 明确当前只允许 optional 消费 |
| Go DB integration | SKIP | `CSES_IM_TEST_DSN` 未设置，Task 4 只读复核明确禁止写成 live DB green |
| mobile pinned rev gate | CONCERN | `make gtest-js-spec` 因共享 Helix checkout rev 与 pinned rev 不一致被脚本拒绝，不影响 focused contract PASS |

## 结论

- 四仓在 `eventSeq/event_seq`、viewers/phantom、累计计数、`too_long` 四条主契约上已收口，可作为 merge 前 contract 对照表。
- `readLastSeq/msgCount` 仍是显式 `PARTIAL`，消费者只能 optional-only；在 Helix 补齐 `channel_member` schema / send-read-sync 写路径前，任何 live green 结论都不成立。
