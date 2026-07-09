# Dialog Projection Sync Closure Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: 本计划默认按用户要求使用 inline execution，不再使用蜂群。若未来重新授权 agentic workers，才使用 superpowers:subagent-driven-development 或 superpowers:executing-plans task-by-task。Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `im:channel:update-by-post` 与 `im:sync:too_long` 从四仓临时实现收敛成可合并、可验证、无 LoopForge 依赖的正式链路。

**Architecture:** Helix 是唯一业务投影真源：在线 post / 离线 sync 写 storage 后输出 render-ready DialogPatch；Go 只提供事实字段和 sync 契约；mobile 与 cses-client 只做事件 schema 校验、透传和 UI 渲染。当前 Helix 已移除 `readLastSeq/msgCount` 空占位，因此前端和 mobile 必须把这两个字段改为“可选诊断字段”，直到 `NEED_HELIX_CHANNEL_MEMBER_FIX.md` 对应 schema/write path 完成。

**Tech Stack:** Rust workspace (`helix-im`, `helix-core`, drivers), Go `cses-im-server`, C++/QuickJS mobile bridge, Angular message-v4, Jest/Playwright, Cargo/Go tests.

## Global Constraints

- 不再使用蜂群；后续按本计划 inline 小步执行，必要时只用本地命令、`rg`、`apply_patch`、测试和手工 review。
- LoopForge 完全剔除；本计划不读、不改、不验证 `/System/Volumes/Data/workspace/rust/loopforge-tauri-im`。
- 排除域固定：`bot/agent`、`search`、`vote/average`。
- 前端只渲染 Helix 投影，不计算 `unread/mention/urgent/lastPost`，不通过 `lastPost` 兜底生成消息列表。
- cses-client 不再分析或依赖前端数据库表；未来存储由 Helix storage 接管。
- `DialogPatch.unreadCount/mentionCount/urgentCount` 必须是 Helix storage 写后累计终值，不允许前端或 mobile `+1`。
- `too_long` 必须是 drop-and-reload：Helix 删除本地 channel messages 派生态、发 `im:sync:too_long` loading 指令，并主动触发首屏覆盖重拉。
- `readLastSeq/msgCount` 在 Helix `channel_member` schema/write path 闭环前不得作为必填 render-ready 字段输出；消费者只能 optional 透传或隐藏。
- 每个仓提交前必须确认 worktree branch：`codex/dialog-projection-sync-spec-20260709`；cses-client 最终合并目标是 `feat/helix-reactor-im`，其他仓是 `main`。
- commit message 必须 Conventional Commit + 中文结构化 body，包含 `# 影响范围`、`# 改动影响面`、`# 功能改进/开发/新增`、`# 验证`。

---

## Current Branch State

| 仓库 | worktree | 当前 head | review 状态 |
|---|---|---:|---|
| helix | `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709` | `69bcc88` | fix 完成，需 inline re-review |
| cses-im-server | `/System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709` | `3fd7e5a` | task review PASS；DB 集成因无 `CSES_IM_TEST_DSN` skip |
| mobile-quick | `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709` | `80c33cf` | fix 完成，需 inline re-review |
| cses-client | `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709` | `b978ece06` | re-review PASS；还需对齐 `readLastSeq/msgCount` optional |

## File Structure

Helix:
- Modify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-dialog-projection-and-sync-reset-spec.md`
- Verify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/NEED_HELIX_CHANNEL_MEMBER_FIX.md`
- Verify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/crates/helix-im/docs/projection-schema.md`

cses-client:
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/projection.types.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/view-model.types.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/projection-store.service.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/components/channel-list/channel-list.component.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/__tests__/projection-store.service.test.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/tests/e2e/message-helix-hard-cut.spec.ts`

mobile-quick:
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/js/im_contract.js`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/contract/mobile_public_events.json`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/cpp/MobileImSdk.cpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/tests/gtest/MobileJsSpecTest.cpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/tests/gtest/MobileSdkSpecTest.cpp`

cses-im-server:
- Verify only unless drift appears: `/System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-helix-dialog-projection-backend-contract.md`

---

### Task 1: Contract Drift Fix For `readLastSeq/msgCount`

**Files:**
- Modify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-dialog-projection-and-sync-reset-spec.md`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/projection.types.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/view-model.types.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/projection-store.service.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/components/channel-list/channel-list.component.ts`
- Modify: `/System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709/src/pages/message-v4/__tests__/projection-store.service.test.ts`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/js/im_contract.js`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/contract/mobile_public_events.json`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/tests/gtest/MobileJsSpecTest.cpp`
- Modify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/tests/gtest/MobileSdkSpecTest.cpp`

**Interfaces:**
- Consumes: Helix `NEED_HELIX_CHANNEL_MEMBER_FIX.md` says `readLastSeq/msgCount` are intentionally removed from render-ready DialogPatch.
- Produces: cses-client/mobile accept absent `readLastSeq/msgCount` without defaulting to misleading `0`; when future Helix restores fields, consumers still pass them through.

- [ ] **Step 1: Write failing cses-client regression test**

Add a test in `projection-store.service.test.ts` that applies a DialogPatch without `readLastSeq/msgCount` and asserts the UI model keeps them absent.

```ts
it('does not invent readLastSeq or msgCount when Helix omits channel_member fields', () => {
  store.applyEnvelope({
    event: 'im:channel:update-by-post',
    data: {
      op: 'dialog.patchByPost',
      channel_id: 'ch-no-member-count',
      channelId: 'ch-no-member-count',
      event_seq: 301,
      eventSeq: 301,
      msg_id: 'p-no-member-count',
      msgId: 'p-no-member-count',
      messageClass: 'normal',
      senderUserId: 'u2',
      dialogPatch: {
        id: 'ch-no-member-count',
        channelId: 'ch-no-member-count',
        lastMessage: 'no member count yet',
        unreadCount: 1,
        mentionCount: 0,
        urgentCount: 0,
        lastEventSeq: 301
      }
    }
  });

  const dialog = store.dialogs().find((item) => item.channelId === 'ch-no-member-count');
  expect(dialog?.readLastSeq).toBeUndefined();
  expect(dialog?.msgCount).toBeUndefined();
});
```

- [ ] **Step 2: Run cses-client test and confirm failure**

Run:

```bash
cd /System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709
npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts --runInBand
```

Expected before implementation: FAIL because `ProjectionStore` currently defaults `readLastSeq/msgCount` to `0`.

- [ ] **Step 3: Make cses-client fields optional**

Change `DialogVm` in `view-model.types.ts`:

```ts
readLastSeq?: number;
msgCount?: number;
```

Change `ProjectionStore` upsert logic to preserve absence:

```ts
readLastSeq: input.readLastSeq ?? existing?.readLastSeq,
msgCount: input.msgCount ?? existing?.msgCount,
```

Change `ChannelListComponent` data attrs to avoid publishing false zero:

```html
[attr.data-read-last-seq]="dialog.readLastSeq ?? null"
[attr.data-msg-count]="dialog.msgCount ?? null"
```

- [ ] **Step 4: Make mobile required contract optional**

In `js/im_contract.js` and `contract/mobile_public_events.json`, remove `readLastSeq` and `msgCount` from required DialogPatch fields. Keep the C++ pass-through code in `MobileImSdk.cpp` so present fields are still stored:

```cpp
patchRawField("readLastSeq", topLevelRawField(patch_source, "readLastSeq")),
patchRawField("msgCount", topLevelRawField(patch_source, "msgCount")),
```

Add invalid/valid tests:

```cpp
// valid without readLastSeq/msgCount
auto event = makeDialogPatchEvent("normal", "sync_events", "");
sdk.applyEventProjectionForTest(event);
EXPECT_EQ(store.channelValue("ch-dialog", "message_class"), "normal");
EXPECT_TRUE(store.channelValue("ch-dialog", "read_last_seq").empty());
EXPECT_TRUE(store.channelValue("ch-dialog", "msg_count").empty());
```

- [ ] **Step 5: Update Helix spec wording**

In `2026-07-09-dialog-projection-and-sync-reset-spec.md`, add a row:

```md
| `dialogPatch.readLastSeq/msgCount` | 当前不输出；等 `NEED_HELIX_CHANNEL_MEMBER_FIX.md` 完成 Helix `channel_member` schema/write path 后再恢复，消费者不得默认 0 |
```

- [ ] **Step 6: Run focused tests**

Run:

```bash
cd /System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709
npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts --runInBand
/System/Volumes/Data/workspace/angular/cses-client/node_modules/@playwright/test/node_modules/.bin/playwright test -c tests/e2e message-helix-hard-cut.spec.ts
git diff --check
```

Run:

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709
./scripts/run-gtest-js-spec.sh
make real-chain-manifest
git diff --check
```

Expected: all PASS; `make gtest-js-spec` may still fail on pinned Helix rev and must be recorded as a non-blocking pin risk.

- [ ] **Step 7: Commit per repo**

cses-client:

```bash
cd /System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709
git add src/pages/message-v4/projection.types.ts src/pages/message-v4/view-model.types.ts src/pages/message-v4/projection-store.service.ts src/pages/message-v4/components/channel-list/channel-list.component.ts src/pages/message-v4/__tests__/projection-store.service.test.ts tests/e2e/message-helix-hard-cut.spec.ts
git commit -m "fix(message-v4): 将 channel_member 读数投影改为可选字段" -m "# 影响范围
- message-v4 ProjectionStore/DialogVm/ChannelListComponent 的 DialogPatch 消费路径
- projection-store focused tests 与 message-helix-hard-cut gate

# 改动影响面
- im:channel:update-by-post 缺 readLastSeq/msgCount 时不再默认 0，避免 UI 把 Helix 尚未闭环的 channel_member 字段误显示为真实值
- 已存在字段到达时仍透传给 DialogVm 和 DOM data attrs
- 不影响 im:post:received / im:messages:* 消息列表投影

# 功能改进/开发/新增
- 类型: fix
- 触发条件: Helix 修复后移除了 readLastSeq/msgCount 的 null 占位
- 根因类别: 协议字段状态漂移
- 行为变化: 缺字段时 UI 隐藏诊断字段，不再显示 0

# 验证
- npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts --runInBand PASS
- playwright test -c tests/e2e message-helix-hard-cut.spec.ts PASS
- git diff --check PASS"
```

mobile:

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709
git add js/im_contract.js contract/mobile_public_events.json tests/gtest/MobileJsSpecTest.cpp tests/gtest/MobileSdkSpecTest.cpp
git commit -m "fix(dialog): 将 channel_member 读数从必填契约降级为可选透传" -m "# 影响范围
- mobile DialogPatch contract gate、QuickJS/C++ focused tests

# 改动影响面
- im:channel:update-by-post 不再要求 readLastSeq/msgCount 必填
- 字段存在时仍进入 channel_row read_last_seq/msg_count
- 不影响 too_long loading/drop gate

# 功能改进/开发/新增
- 类型: fix
- 触发条件: Helix 当前未完成 channel_member read/count schema 闭环
- 根因类别: 协议字段状态漂移
- 行为变化: mobile 不再因缺 readLastSeq/msgCount 拒收有效 DialogPatch

# 验证
- ./scripts/run-gtest-js-spec.sh PASS
- make real-chain-manifest PASS
- git diff --check PASS"
```

---

### Task 2: Inline Re-Review Helix Fix Commit

**Files:**
- Read: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/.superpowers/sdd/review-8e17a0d..8c8c6c6.diff`
- Create: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/.superpowers/sdd/review-8c8c6c6..69bcc88.diff`
- Verify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/REPORT_HELIX.md`

**Interfaces:**
- Consumes: Helix fix commit `69bcc88`.
- Produces: inline review verdict and optional fix patch.

- [ ] **Step 1: Generate fix review package**

Run:

```bash
cd /System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709
/Users/xpa-12/.codex/plugins/cache/claude-plugins-official/superpowers/6.1.1/skills/subagent-driven-development/scripts/review-package 8c8c6c6 HEAD
```

Expected: writes `.superpowers/sdd/review-8c8c6c6..69bcc88.diff`.

- [ ] **Step 2: Manually inspect the five prior findings**

Run:

```bash
rg -n "unreadCount|mentionCount|urgentCount|unreadDelta|mentionDelta|urgentDelta|too_long|getLatestPost|visible|viewers|sender|readLastSeq|msgCount" crates/helix-im crates/helix-core crates/helix-driver-host NEED_HELIX_CHANNEL_MEMBER_FIX.md
```

Expected checks:
- `unreadCount/mentionCount/urgentCount` come from write-after `Get(channel)` or equivalent persisted projection, not event delta.
- Delta fields, if present, are named `unreadDelta/mentionDelta/urgentDelta`.
- `too_long` emits `im:sync:too_long` and schedules active reload effect.
- Sync type1 checks visible/viewers before message upsert and UI emit.
- Sender self-visible is allowed, own echo unread delta is 0.
- `readLastSeq/msgCount` are absent from render-ready patch and documented in NEED.

- [ ] **Step 3: Run Helix verification suite**

Run:

```bash
cd /System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709
cargo check -p helix-im
cargo check -p helix-driver-host
cargo test -p helix-im
cargo test -p helix-driver-host
cargo test -p helix-core
cargo test -p helix-im --test dialog_projection_test -- --nocapture
cargo test -p helix-im --test c3_sync_mutation_emits_test -- --nocapture
cargo test -p helix-im --test s6_sync_internal_test -- --nocapture
cargo test -p helix-im --test stress_test -- --nocapture
git diff --check
```

Expected: PASS. Existing `network_shared.rs unused_mut` warning is non-blocking if unchanged.

- [ ] **Step 4: Fix any inline review blocker**

If a blocker appears, make the smallest patch in Helix and add a test. For example, if `too_long` only emits reload effect but lacks payload channel id, add a test:

```rust
#[test]
fn too_long_reload_effect_targets_the_reset_channel() {
    let effects = apply_too_long_for_test("ch1", 9001);
    assert!(effects.iter().any(|effect| effect.is_get_latest_post_for("ch1")));
}
```

Run the failing test first, then patch, then rerun Task 2 Step 3.

- [ ] **Step 5: Commit Helix review fix if needed**

If no code change is needed, do not create a commit. If a fix is needed:

```bash
git add crates/helix-im crates/helix-core crates/helix-driver-host REPORT_HELIX.md
git commit -m "fix(im): 收紧 DialogPatch 收口验证缺口" -m "# 影响范围
- Helix DialogPatch/too_long 收口路径与对应测试

# 改动影响面
- 保持 write-after projection、viewer gate、too_long reload effect 语义
- 不影响 Go payload 字段、不影响 cses-client/mobile render-only 消费

# 功能改进/开发/新增
- 类型: fix
- 触发条件: inline review 发现 spec 收口缺口
- 根因类别: 投影闭环遗漏
- 行为变化: 仅修正 Helix 输出/测试闭环

# 验证
- cargo check -p helix-im PASS
- cargo test -p helix-im PASS
- cargo test -p helix-driver-host PASS
- git diff --check PASS"
```

---

### Task 3: Inline Re-Review Mobile Fix Commit

**Files:**
- Create: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/.superpowers/sdd/review-6cdf3fc..80c33cf.diff`
- Verify: `/System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709/REPORT_MOBILE.md`

**Interfaces:**
- Consumes: mobile fix commit `80c33cf` and Task 1 optional field contract.
- Produces: mobile review verdict and optional fix patch.

- [ ] **Step 1: Generate mobile fix review package**

Run:

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++/worktrees/dialog-projection-sync-spec-20260709
/Users/xpa-12/.codex/plugins/cache/claude-plugins-official/superpowers/6.1.1/skills/subagent-driven-development/scripts/review-package 6cdf3fc HEAD
```

- [ ] **Step 2: Verify payload gate behavior**

Run:

```bash
rg -n "dialogPatch|dropLocalMessages|mode|raw event|dialogProjectionCases|readLastSeq|msgCount" cpp js contract tests/gtest real-chain scripts/real-chain
```

Expected:
- Missing `dialogPatch` does not write `channel_row` or `message_row`.
- Invalid `too_long` does not clear local messages.
- `dialogProjectionCases` is consumed by `scripts/real-chain/check-manifest.mjs`.
- After Task 1, `readLastSeq/msgCount` are optional.

- [ ] **Step 3: Run mobile verification**

Run:

```bash
./scripts/run-gtest-js-spec.sh
make real-chain-manifest
git diff --check
```

Expected: PASS. `make gtest-js-spec` may fail only on Helix pinned rev mismatch; record exact line if so.

- [ ] **Step 4: Fix any mobile blocker**

If invalid events still mutate VM, add a focused gtest:

```cpp
TEST(MobileDialogProjectionTest, InvalidTooLongDoesNotDropMessages) {
  MobileImSdk sdk = makeSdkWithMessage("ch-invalid", "p-existing");
  sdk.applyEventProjectionForTest(R"({"event":"im:sync:too_long","data":{"channelId":"ch-invalid"}})");
  EXPECT_TRUE(sdk.snapshotJson().find("p-existing") != std::string::npos);
  EXPECT_TRUE(sdk.snapshotJson().find("syncLoading") == std::string::npos);
}
```

Then patch projector and rerun Step 3.

---

### Task 4: Go Backend Contract Re-Check With Helix Final Shape

**Files:**
- Read: `/System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709/REPORT_CSES_IM_SERVER.md`
- Read: `/System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-helix-dialog-projection-backend-contract.md`

**Interfaces:**
- Consumes: Helix final event shape after Task 1/2.
- Produces: backend contract confirmation; no code change unless Helix final shape requires a missing post/sync fact.

- [ ] **Step 1: Re-run focused Go tests**

Run:

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709
go test ./internal/app -run 'TestPublishPostEvent_IncludesHelixProjectionFields|TestBuildMessagesMap_NilPostSkip|TestSyncChannels_(TooLongMock|ReadEventNoMessageBodyMock)$' -count=1
go test ./internal/app/sync -run TestPostToSyncMessage_KeepsHelixProjectionFields -count=1
go test ./internal/ws -run 'TestResolveViewers_(AllBranches|PointerStringSlice|PqStringArray|UnknownShape)$' -count=1
git diff --check
```

Expected: PASS.

- [ ] **Step 2: Run DB integration only if DSN is available**

Run:

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server/worktrees/dialog-projection-sync-spec-20260709
if [ -n "$CSES_IM_TEST_DSN" ]; then
  go test ./internal/app -run 'TestSyncChannels_(KindEvents|KindTooLong|EventsEmptyMessages)$' -count=1 -v
else
  echo "SKIP: CSES_IM_TEST_DSN is not set"
fi
```

Expected without DSN: explicit SKIP, not green.

- [ ] **Step 3: Confirm no backend table deletion**

Run:

```bash
git diff --name-only 876464f1a02af82a84e4627bc312c313ed4e6114..HEAD
rg -n "DROP TABLE|drop table|DeleteTable|schema.*remove" internal docs
```

Expected: no destructive schema cleanup.

---

### Task 5: Cross-Repo Contract Matrix

**Files:**
- Create or modify: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-dialog-projection-closure-matrix.md`

**Interfaces:**
- Consumes: final outputs from Tasks 1-4.
- Produces: one concise matrix showing field ownership and verification result.

- [x] **Step 1: Write closure matrix**

Create a Markdown table with these rows:

```md
| Contract | Go | Helix | mobile | cses-client | Result |
|---|---|---|---|---|---|
| post `eventSeq/event_seq` | emits both | consumes both | pass-through | pass-through | PASS |
| viewers/phantom | WS narrows + sync messages filter | local recheck | no business calc | no business calc | PASS |
| unread/mention/urgent counts | source facts | write-after cumulative projection | render only | render only | PASS |
| too_long | returns resetTo | drop + active reload | loading only | loading only | PASS |
| readLastSeq/msgCount | channel_member source | NEED pending | optional only | optional only | PARTIAL |
```

- [x] **Step 2: Include exact verification commands**

Append the final command list from Tasks 1-4 and mark each as `PASS`, `SKIP`, or `CONCERN`.

- [x] **Step 3: Commit matrix in Helix**

```bash
cd /System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709
git add docs/specs/2026-07-09-dialog-projection-closure-matrix.md docs/superpowers/plans/2026-07-09-dialog-projection-sync-closure.md
git commit -m "docs(im): 补齐 Dialog 投影收口计划与矩阵" -m "# 影响范围
- Helix docs/specs DialogProjection 收口计划和跨仓矩阵

# 改动影响面
- 记录 Go/Helix/mobile/cses-client 字段责任边界
- 明确 readLastSeq/msgCount 为 PARTIAL，避免消费者误判已完成
- 不改运行时代码

# 功能改进/开发/新增
- 类型: docs
- 触发条件: 四仓实现后需要按 spec 收敛合并顺序
- 根因类别: 跨仓契约沉淀缺口
- 行为变化: 无运行时行为变化

# 验证
- git diff --check PASS
- markdown table scan PASS"
```

---

### Task 6: Real-Chain Acceptance Smoke

**Files:**
- Create: `/System/Volumes/Data/workspace/rust/helix/worktrees/dialog-projection-sync-spec-20260709/docs/specs/2026-07-09-dialog-projection-acceptance-report.md`

**Interfaces:**
- Consumes: merged or worktree-local runnable artifacts from all four repos.
- Produces: pass/partial report for the six spec acceptance cases.

- [ ] **Step 1: Define six acceptance cases**

Write these cases into the report:

```md
1. normal: A sends normal text; B sees `messageClass=normal`, lastMessage, unreadCount cumulative.
2. mention: A @B; B sees `messageClass=mention`, mentionCount cumulative, mentioned user ids.
3. urgent: A sends urgent; B sees `messageClass=urgent`, urgentCount cumulative.
4. own echo: A sees lastPost update, unreadCount unchanged.
5. sync visible/phantom: visible message emits DialogPatch; phantom only advances cursor.
6. too_long: local messages drop, loading event emits, Helix active reload emits replacement projection.
```

- [ ] **Step 2: Run available non-live gates**

Run all focused commands from Tasks 1-4. Expected: PASS except explicitly documented DSN/pin concerns.

- [ ] **Step 3: Run live E2E only when credentials and services are ready**

If the local environment has two valid users/cookies and Go backend is reachable, run the existing project live command. If no command exists, record the missing command instead of inventing green:

```bash
cd /System/Volumes/Data/workspace/angular/cses-client/worktrees/dialog-projection-sync-spec-20260709
rg -n "444|678|message-v4|real-chain|live" tests scripts package.json
```

Expected if no ready script: report `LIVE_E2E_NOT_RUN` with exact missing pieces: user auth, backend URL, command name, fixture channel.

- [ ] **Step 4: Write final acceptance status**

Use only these statuses:

```md
| Case | Status | Evidence |
|---|---|---|
| normal | PASS/PARTIAL/BLOCKED | command + file |
```

No case can be `PASS` from worker self-report alone.

---

### Task 7: Merge And Cleanup

**Files:**
- No source files unless conflict resolution requires it.

**Interfaces:**
- Consumes: review-clean worktree branches.
- Produces: branches merged to targets and obsolete worktrees removed.

- [ ] **Step 1: Pre-merge conflict analysis**

Run:

```bash
git -C /System/Volumes/Data/workspace/rust/helix diff --name-only main..codex/dialog-projection-sync-spec-20260709 > /tmp/helix-dialog-files.txt
git -C /System/Volumes/Data/workspace/golang/cses-im-server diff --name-only main..codex/dialog-projection-sync-spec-20260709 > /tmp/go-dialog-files.txt
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ diff --name-only main..codex/dialog-projection-sync-spec-20260709 > /tmp/mobile-dialog-files.txt
git -C /System/Volumes/Data/workspace/angular/cses-client diff --name-only feat/helix-reactor-im..codex/dialog-projection-sync-spec-20260709 > /tmp/cses-client-dialog-files.txt
```

Expected: each file list contains only this feature's files. If target branch has changed the same files since baseline, stop and resolve manually.

- [ ] **Step 2: Merge Helix to main**

```bash
cd /System/Volumes/Data/workspace/rust/helix
git switch main
git merge --no-ff codex/dialog-projection-sync-spec-20260709
cargo test -p helix-im --test dialog_projection_test -- --nocapture
git diff --check HEAD~1..HEAD
```

- [ ] **Step 3: Merge Go to main**

```bash
cd /System/Volumes/Data/workspace/golang/cses-im-server
git switch main
git merge --no-ff codex/dialog-projection-sync-spec-20260709
go test ./internal/app ./internal/app/sync ./internal/ws -count=1
git diff --check HEAD~1..HEAD
```

- [ ] **Step 4: Merge mobile to main**

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
git switch main
git merge --no-ff codex/dialog-projection-sync-spec-20260709
./scripts/run-gtest-js-spec.sh
make real-chain-manifest
git diff --check HEAD~1..HEAD
```

- [ ] **Step 5: Merge cses-client to `feat/helix-reactor-im`**

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
git switch feat/helix-reactor-im
git merge --no-ff codex/dialog-projection-sync-spec-20260709
npm test -- --runTestsByPath src/pages/message-v4/__tests__/projection-store.service.test.ts src/pages/message-v4/__tests__/projection-handler.registry.test.ts --runInBand
/System/Volumes/Data/workspace/angular/cses-client/node_modules/@playwright/test/node_modules/.bin/playwright test -c tests/e2e message-helix-hard-cut.spec.ts
git diff --check HEAD~1..HEAD
```

- [ ] **Step 6: Cleanup merged worktrees**

Only after all merges and verification pass:

```bash
git -C /System/Volumes/Data/workspace/rust/helix worktree remove --force worktrees/dialog-projection-sync-spec-20260709
git -C /System/Volumes/Data/workspace/golang/cses-im-server worktree remove --force worktrees/dialog-projection-sync-spec-20260709
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ worktree remove --force worktrees/dialog-projection-sync-spec-20260709
git -C /System/Volumes/Data/workspace/angular/cses-client worktree remove --force worktrees/dialog-projection-sync-spec-20260709
```

Do not delete the feature branches until the user confirms no rollback is needed.

---

## Self-Review

**Spec coverage:** The plan covers online post, DialogPatch render-ready fields, viewers/phantom, normal/mention/urgent/own echo, sync type1, too_long drop-and-reload, Go backend facts, mobile/cses-client render-only consumers, excluded domains, and merge targets.

**Known partial:** `readLastSeq/msgCount` remains intentionally partial because Helix `channel_member` schema/write path is not implemented. The plan prevents false completion by making consumers optional-only and recording `NEED_HELIX_CHANNEL_MEMBER_FIX.md`.

**Placeholder scan:** No task uses unfinished placeholder wording. All steps include exact paths and commands.

**Execution mode:** User explicitly disabled swarm. Execute inline with checkpoints, and ask before merge if any target branch has same-file conflicts or if live E2E credentials are missing.
