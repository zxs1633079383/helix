#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { basename, join, relative } from 'node:path';

const repoRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], {
  encoding: 'utf8',
}).trim();
const clientRoot = '/System/Volumes/Data/workspace/angular/cses-client';
const docsRoot = join(repoRoot, 'docs/interaction-frame-v2-全量替换-按UC');
const matrixPath = join(
  repoRoot,
  'crates/helix-im/contracts/strict-matrix.v1.json',
);
const matrixSource = readFileSync(matrixPath, 'utf8');
const matrix = JSON.parse(matrixSource);

const helixHead = git(repoRoot, ['rev-parse', 'HEAD']);
const clientHead = git(clientRoot, ['rev-parse', 'HEAD']);
const clientCargo = readFileSync(join(clientRoot, 'src-tauri/Cargo.toml'), 'utf8');
const pinnedRevisions = [
  ...clientCargo.matchAll(
    /helix-(?:core|im|driver-host|driver-native)\s*=\s*\{[^}]*rev\s*=\s*"([0-9a-f]{40})"/g,
  ),
].map((match) => match[1]);
const uniquePins = [...new Set(pinnedRevisions)];
if (uniquePins.length !== 1) {
  throw new Error(`cses-client Helix pins are not unified: ${uniquePins.join(', ')}`);
}
const clientHelixPin = uniquePins[0];

const ucDefinitions = [
  {
    dir: 'GS01',
    gates: ['G-S1'],
    title: '连接建立后的全量增量恢复',
    entry: '连接建立、Surface attach 或全量恢复',
    ingress: 'HostObservation / Surface resync -> direct command',
    components: '`MessageComponent`、`MessageSurfaceRuntimeViewComponent`、`MessageUiSurfaceDialogListComponent`',
    clientFlow: '`ShellService.start/requestInteractionFrameSnapshot` -> `im_query_dialog_list`',
    result: '会话列表只在权威快照可用后出现；HTTP 受理或 `increment_channel_end` 不能单独晋升 ready。',
    helixTest: 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs',
    clientTest: 'src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts',
  },
  {
    dir: 'GS02',
    gates: ['G-S2'],
    title: 'sync notify 离线恢复',
    entry: '离线 cursor 补齐或显式同步',
    ingress: 'Inbound / Timer / direct command',
    components: '`MessageComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`ShellService.recoverSyncGap` -> `im_sync_channels`',
    result: 'cursor、消息事实和受影响会话 Surface 以同一提交边界收敛。',
    helixTest: 'crates/helix-im/tests/gs_persist_barrier_test.rs',
    clientTest: 'src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts',
  },
  {
    dir: 'GS03',
    gates: ['G-S3'],
    title: 'pong gap 有界补偿',
    entry: '`pong.gaps` 或 hash mismatch',
    ingress: 'Inbound + Timer；不是 UI ActionBinding',
    components: '`MessageComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: 'Native transport -> Helix sync scheduler；Angular 只消费 Frame runtime',
    result: '恢复状态来自 `InteractionRuntimeVm`；普通 pong 不伪造业务完成。',
    helixTest: 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs',
    clientTest: 'src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts',
  },
  {
    dir: 'GS04',
    gates: ['G-S4'],
    title: '陌生频道 hydrate',
    entry: '选择未在本地投影中的 `channelId`',
    ingress: 'HostObservation / Surface resync -> direct command',
    components: '`MessageComponent`、`MessageUiHeaderComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`ShellService.ensureChannelLoaded` -> `im_channel_load_increment_by_channel_id`',
    result: '频道摘要必须由 Helix hydrate 后进入 Surface，客户端不从路由参数拼频道事实。',
    helixTest: 'crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs',
    clientTest: 'src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts',
  },
  {
    dir: 'G01',
    gates: ['G-01'],
    title: '发送文本',
    entry: 'composer 输入文本并提交',
    ingress: 'direct command；当前没有 `message.create` ActionBinding',
    components: '`MessageUiComposerComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`MessageComponent.submitComposer` -> `ShellService.sendText` -> `im_send_message`',
    result: '当前 canonical Frame 只刷新 timeline；composer draft 属于 shell state，不能把 Dialog/Unread 影响面写成已提交 mutation。',
    helixTest: 'crates/helix-im/tests/p1_send_build_test.rs',
    clientTest: 'src/pages/message-v4/__tests__/shell.service.test.ts',
  },
  {
    dir: 'G02',
    gates: ['G-02'],
    title: '发送文档消息',
    entry: 'composer 选择文档内容并提交',
    ingress: 'direct command',
    components: '`MessageUiComposerComponent`、`MessageDocumentContentComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`MessageComponent.submitComposer` -> `ShellService.sendDocument` -> `im_send_message`',
    result: '文档行只读取 typed `SurfaceMessageVm.content=document`，不解析 raw props 还原业务状态。',
    helixTest: 'crates/helix-im/tests/p1_send_build_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts',
  },
  {
    dir: 'G04',
    gates: ['G-04', 'G-04a'],
    title: '在线撤回与离线撤回重放',
    entry: '消息菜单撤回；observer 收到离线 type-3 事实',
    ingress: 'actor 使用 ActionBinding；observer 使用 Inbound/sync',
    components: '`MessageUiActionMenuComponent`、`MessageUiSurfaceThreadDrawerComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`requestSurfaceAction` -> `im_invoke_action`；离线事实不经过 UI command',
    result: 'actor/observer 都只按提交后的 timeline Frame 显示撤回；raw WS 不直接改 DOM。',
    helixTest: 'crates/helix-im/tests/g04_revoke_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-g04-revoke.component.test.ts',
  },
  {
    dir: 'G05',
    gates: ['G-05'],
    title: '单条与多选转发',
    entry: '消息行或多选工具条发起转发',
    ingress: '单条优先 ActionBinding；多选仍走 direct command',
    components: '`MessageUiForwardDialogComponent`、`MessageUiSurfaceForwardDetailComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`submitForward` -> `im_invoke_action` 或 `ShellService.relayMessages` -> `im_relay_messages`',
    result: '目标结果来自 Frame causation；HTTP accepted 不能关闭 overlay 或冒充所有目标成功。',
    helixTest: 'crates/helix-im/tests/g05_forward_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-g05-forward.component.test.ts',
  },
  {
    dir: 'G06',
    gates: ['G-06'],
    title: 'reaction / 快捷回复',
    entry: '点击 emoji',
    ingress: 'ActionBinding `react`',
    components: '`MessageUiSurfaceQuickActionsComponent`、`MessageUiActionMenuComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`MessageComponent.sendReaction` -> `im_invoke_action`',
    result: '客户端不本地累加 emoji 计数；权威聚合由 timeline Frame 替换。',
    helixTest: 'crates/helix-im/tests/g06_reaction_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-g06-reaction.component.test.ts',
  },
  {
    dir: 'G07',
    gates: ['G-07'],
    title: '加急与确认',
    entry: '发起加急或点击确认',
    ingress: 'ActionBinding `urgent/urgentConfirm`',
    components: '`MessageUiUrgentComponent`、`MessageUiReadReceiptComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`submitUrgent/actionRequested` -> `im_invoke_action`',
    result: '消息态与参与者确认态分别由 timeline/participant-receipts typed VM 提供。',
    helixTest: 'crates/helix-im/tests/g07_urgent_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-g07-urgent.component.test.ts',
  },
  {
    dir: 'G08',
    gates: ['G-08'],
    title: '创建定时消息',
    entry: 'composer 打开定时对话框并提交',
    ingress: '当前主要 direct command；代码保留“有 enabled binding 则用”的过渡分支',
    components: '`MessageUiScheduleDialogComponent`、`MessageUiOverlayHostComponent`、`MessageComponent`',
    clientFlow: '`submitSchedule` -> binding（若存在）或 `ShellService.createSchedule` -> `im_create_schedule`',
    result: '当前 ActionDispatch 未审计 `createSchedule`；direct 分支通过 bounded dialog resync 等待 Frame。',
    helixTest: 'crates/helix-im/tests/g08_schedule_create_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-schedule-dialog.component.test.ts',
  },
  {
    dir: 'G09',
    gates: ['G-09'],
    title: '取消定时消息',
    entry: '频道菜单确认取消定时',
    ingress: '当前主要 direct command；代码保留 binding 优先分支',
    components: '`MessageUiConfirmDialogComponent`、`MessageSurfaceRuntimeViewComponent`、`MessageComponent`',
    clientFlow: '`confirmCancelSchedule` -> binding（若存在）或 `ShellService.cancelSchedule` -> `im_cancel_schedule`',
    result: '取消终态等待 channel-summary Frame，不根据 command receipt 直接移除标记。',
    helixTest: 'crates/helix-im/tests/g09_schedule_cancel_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts',
  },
  {
    dir: 'G10',
    gates: ['G-10', 'G-10a'],
    title: '首屏窗口与向前补窗',
    entry: '切换频道；上拉更早历史',
    ingress: 'Surface attach/resync + direct query',
    components: '`MessageSurfaceRuntimeViewComponent`、`MessageComponent`',
    clientFlow: '`activateChannel/loadOlder` -> `im_query_messages_by_channel` / `im_load_older_context`',
    result: '首屏与补窗都由完整 `TimelineWindowVm` 驱动；older receipt 只触发 authoritative refresh。',
    helixTest: 'crates/helix-im/tests/g10_timeline_window_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts',
  },
  {
    dir: 'G11',
    gates: ['G-11'],
    title: '按 postId 定位',
    entry: '点击定位消息',
    ingress: '当前 MessageV4 handler 走 direct query；Helix timeline 也可声明 `locate` binding',
    components: '`MessageSurfaceRuntimeViewComponent`、`MessageComponent`',
    clientFlow: '`locateMessage` -> `ShellService.locatePost` -> `im_get_posts`',
    result: '定位结果和 reveal directive 必须来自同一 Frame，客户端不自行插入孤立消息。',
    helixTest: 'crates/helix-im/tests/g11_locate_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts',
  },
  {
    dir: 'G12',
    gates: ['G-12'],
    title: '回复线程与分支',
    entry: '打开回复抽屉、加载分支或发送回复',
    ingress: '查询与回复发送均为 direct command',
    components: '`MessageUiSurfaceThreadDrawerComponent`、`MessageUiReplyDrawerComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`openThread/loadReplyBranch/sendReply` -> `im_get_replies` / `im_get_reply_branch` / `im_send_message`',
    result: '线程节点、顺序和回复总数由 `ThreadDetailVm` 提供；发送后重查仍只消费 Frame。',
    helixTest: 'crates/helix-im/tests/g12_reply_thread_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/message-ui-reply-drawer.component.test.ts',
  },
  {
    dir: 'G13',
    gates: ['G-13'],
    title: '频道已读',
    entry: '打开频道或显式标记频道已读',
    ingress: 'direct command；`markChannelRead` ActionBinding 仍未审计',
    components: '`MessageSurfaceRuntimeViewComponent`、`MessageComponent`',
    clientFlow: '`readChannel` -> `ShellService.readChannel` -> `im_channels_view`',
    result: '频道摘要与参与者回执由各自 Surface 提交，客户端不从 raw readBits 推导全局事实。',
    helixTest: 'crates/helix-im/tests/g13_channel_read_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/g13-channel-read-interaction.test.ts',
  },
  {
    dir: 'G14',
    gates: ['G-14', 'G-14a'],
    title: '单条已读与模板已收到',
    entry: '单条可见上报；模板确认已收到',
    ingress: 'direct command；receipt popover 查询可使用独立 binding',
    components: '`MessageUiReadReceiptComponent`、`MessageSurfaceRuntimeViewComponent`、`MessageComponent`',
    clientFlow: '`markMessageRead/markTemplateReceived` -> `im_post_read` / `im_template_received`',
    result: '已读参与者与模板状态分别进入 participant-receipts/timeline Frame，不能混成一个本地 bool。',
    helixTest: 'crates/helix-im/tests/g14_receipt_template_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/g14-receipt-template-interaction.test.ts',
  },
  {
    dir: 'G15',
    gates: ['G-15', 'G-15a'],
    title: '创建话题与群聊',
    entry: '话题创建器或新建群聊对话框提交',
    ingress: 'direct command',
    components: '`MessageUiCreateTopicComponent`、`MessageUiCreateChatComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`createTopic/createChat` -> `im_make_topic` / `im_create_channel`',
    result: '新频道、成员和 thread link 由 channel-summary/member-roster/thread-detail Frame 建立。',
    helixTest: 'crates/helix-im/tests/g15_topic_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/g15-topic-interaction.test.ts',
  },
  {
    dir: 'G16',
    gates: ['G-16', 'G-16b', 'G-16c'],
    title: '频道关闭、退出与设置',
    entry: '群设置、关闭/退出确认、属性修改',
    ingress: 'direct command；治理 ActionBinding 当前保持 disabled',
    components: '`MessageUiGroupSettingsDrawerComponent`、`MessageUiChannelManagementComponent`、`MessageSurfaceRuntimeViewComponent`',
    clientFlow: '`confirmGovernanceAction/updateGroupSettings` -> `im_channel_close` / `im_channel_leave` / `im_channel_change_*`',
    result: '频道生命周期、成员 roster 与 settingsVersion 均由 Frame 权威返回；版本冲突不由 UI 猜测覆盖。',
    helixTest: 'crates/helix-im/tests/g16_governance_interaction_test.rs',
    clientTest: 'src/pages/message-v4/components/__tests__/g16-governance-interaction.test.ts',
  },
];

const includedGateIds = ucDefinitions.flatMap((uc) => uc.gates);
const g01DeepDocs = new Set([
  '08-G01全领域字段级类图.mmd',
  '09-G01发送全链路时序图.mmd',
  '10-G01三个Frame与状态纠错.mmd',
  '11-G01字段级数据合同与测试证据.md',
  '12-G01-完整主链路.mmd',
  '13-G01-Helix-PC-Mobile-V2验收合同.md',
  'AGENTS.md',
  'handoff.md',
]);
const deepUcDirs = new Set([
  'G02',
  'G04',
  'G05',
  'G06',
  'G07',
  'G08',
  'G10',
  'G13',
  'G14',
  'G15',
]);
const deepUcProfiles = {
  G02: {
    stages: [
      'document-command',
      'uploading-or-sending-frame',
      'terminal-timeline-frame',
      'observer-post-frame',
    ],
    observable:
      '`MessageContentVm::Document{documentId,title,summary,access}`；`DeliveryVm` 只允许 uploading/sending/sent/failed',
    invariant:
      '`type=DOCUMENT` 且 `props.document` 必须由 Helix 解析为 typed document；PC/Mobile 不读取 raw props 兜底',
    drift:
      '当前 focused Rust 真源仍复用发送测试，尚无 G02 独立 canonical Frame Fixture；upload effect 与 Document VM 的阶段对应需由新 replay 固定',
    mobileTest: 'tests/gtest/uc/Uc02DocumentSendTest.cpp',
  },
  G04: {
    stages: [
      'revoke-action-or-command',
      'actor-recalled-frame',
      'observer-online-recalled-frame',
      'observer-offline-replay-frame',
    ],
    observable:
      '`MessageContentVm::Recalled{text}`、`DeliveryVm::Recalled`；原 message id/channel/author/createAt 保持同一逻辑行',
    invariant:
      '撤回文案、可见性和生命周期均由 Helix 给出；在线与离线重放必须收敛到同一 recalled VM',
    drift:
      'G04a 已覆盖原子 Persist 屏障，但尚无按 online/offline 分层的 Helix canonical Frame Fixture 与新 WDIO lane',
    mobileTest: 'tests/gtest/uc/Uc05RevokeTest.cpp',
  },
  G05: {
    stages: [
      'forward-command',
      'source-read-assembly',
      'http-target-acceptance',
      'per-target-authority-frame',
      'batch-terminal-report',
    ],
    observable:
      '`MessageContentVm::Forwarded{summary,items}`；每个 target 以 channelId/postId/reqId 独立相关',
    invariant:
      'HTTP accepted 不是投递完成；只有可信 target `post` 经 Persist/readback 后才能关闭该 target',
    drift:
      'Strict Matrix 声明 canonical timeline，但 focused test 还观察到条件性 left-dialog-list Frame；同时 Matrix 未列 optional req_id，而源码接受 req_id。Fixture 前必须先消除两处 Matrix/source 漂移',
    mobileTest: 'tests/gtest/uc/Uc06RelayTest.cpp',
  },
  G06: {
    stages: [
      'react-action',
      'http-acceptance',
      'post-update-inbound',
      'timeline-reaction-frame',
    ],
    observable:
      '`MessageVm.reactions=[{emoji,count}]` 与 `actions[kind=react]`；count 为权威聚合后的非负整数',
    invariant:
      'renderer 不发送 `desired_selected/user_id`，也不本地增减 count；actor 由鉴权 session 注入',
    drift:
      '当前 Rust focused test 固定 command/wire/action binding，但缺少 post_update 到 reaction Frame 的 canonical replay Fixture',
    mobileTest: 'tests/gtest/uc/Uc07QuickReplyTest.cpp',
  },
  G07: {
    stages: [
      'urgent-action',
      'urgent-timeline-frame',
      'confirm-action',
      'participant-receipts-frame',
    ],
    observable:
      '`MessageVm.urgency` + `ParticipantReceiptsVm{kind=urgentConfirmation,summary,completed,pending,page,state}`',
    invariant:
      'targetIds、required/completed 数量与可确认权限由 Helix 裁决；PC/Mobile 不从数组自行计数',
    drift:
      '当前 focused test 固定 urgent/confirm/cancel binding 与 wire，但完整 timeline + receipts 双 Surface replay 尚未固化为 Fixture；Strict Matrix 还漏列源码已接受的 urgent message/req_id 与 confirm 兼容输入 channel_id',
    mobileTest: 'tests/gtest/uc/Uc08UrgentTest.cpp',
  },
  G08: {
    stages: [
      'schedule-command',
      'http-authority',
      'post-schedule-created-inbound',
      'schedule-summary-frame',
    ],
    observable:
      '声明 Surface 为 `message.channel-summary`；owner-only schedule hydrate 至少包含 channelId/scheduleId/scheduleOwnerUserId/scheduleMessage',
    invariant:
      'observer 与跨频道不得读取 owner-only schedule 内容；UI 不可先写本地 scheduled badge',
    drift:
      '`ChannelSummaryVm` 当前没有显式 schedule 字段，focused test 也只验证 owner hydrate；这是 Fixture/WDIO 前必须解决的 CONTRACT_DRIFT',
    mobileTest: 'tests/gtest/uc/Uc09ScheduleTest.cpp',
  },
  G10: {
    stages: [
      'surface-attach',
      'local-coverage-decision',
      'timeline-snapshot',
      'older-command',
      'replace-view-patch',
    ],
    observable:
      '`TimelineWindowVm{channelId,anchor,items,firstUnreadMessageId?,targetMessageId?,page,state}`；page 含 windowToken/hasOlder/hasNewer/hasMore',
    invariant:
      'Complete 分支零 HTTP；Partial/Miss 才回源；补窗保持 windowToken 并用完整 bounded replace-view，不由 UI merge/sort',
    drift:
      'Rust 已有窗口 patch focused test，Mobile 也有 UC10 command GTest；按 Complete/Partial/Miss 的 canonical Frame inventory 与 WDIO 新 lane 尚未建立',
    mobileTest: 'tests/gtest/uc/Uc10HistoryTest.cpp',
  },
  G13: {
    stages: [
      'channel-view-command',
      'post-read-or-update-channel-authority',
      'receipt-frame',
      'channel-summary-frame',
    ],
    observable:
      '`ParticipantReceiptsVm(kind=read)` 与 `ChannelSummaryVm`；read completion 必须绑定 viewer/channel authority',
    invariant:
      '未读清零和 readBits 不在 UI 先写；客户端不得提交 userId/unreadCount 冒充服务端权威',
    drift:
      'Strict Matrix 声明 receipts + channel-summary，但 ChannelSummaryVm 无 unread 字段；必须在 producer Fixture 中证明真正承载值或修正 Matrix',
    mobileTest: 'tests/gtest/uc/Uc13ReadTest.cpp',
  },
  G14: {
    stages: [
      'single-read-command',
      'read-receipts-frame',
      'template-received-command',
      'template-timeline-frame',
    ],
    observable:
      '`ParticipantReceiptsVm(kind=read)`；模板使用 `MessageVm.templateConfirmation`，不得与 read receipt 合并成本地 bool',
    invariant:
      '`im_post_read` 只接受 channel_id + posts；`im_template_received` 只声明 received，不冒充 template confirmation authority',
    drift:
      '当前 template confirmation projector 在 authority unavailable 时 fail closed；需要分别固化 received 与 confirmation 的正负 Fixture。Matrix 说明仍写 camel postId，而当前 builder/test canonical 输入为 snake_case post_id',
    mobileTest: 'tests/gtest/uc/Uc14TemplateReceivedTest.cpp',
  },
  G15: {
    stages: [
      'create-channel-or-topic-command',
      'http-authority',
      'channel-member-persist',
      'channel-or-thread-frame',
      'observer-frame',
    ],
    observable:
      '`ChannelSummaryVm`、`MemberRosterVm`、`ThreadDetailVm(detailContext=topic)`；topic/channel/member/root/revision 必须同源',
    invariant:
      'actor 为隐式 session owner；userIds 只传显式邀请者；topic root/reply 关系不得由 renderer 拼接',
    drift:
      'focused Rust `g15_topic_interaction_test` 当前验证 `TopicListVm` 不变量，而 Matrix 声明 channel-summary/thread-detail/member-roster；需先对齐 producer 与 Matrix',
    mobileTest: 'tests/gtest/uc/Uc15ChannelCreateTest.cpp',
  },
};
if (includedGateIds.length !== 25 || new Set(includedGateIds).size !== 25) {
  throw new Error('cutover scope must contain exactly 25 unique machine gates');
}

const canonicalCases = new Map(matrix.uc_cases.map((item) => [item.gate_id, item]));
for (const gateId of includedGateIds) {
  if (!canonicalCases.has(gateId)) {
    throw new Error(`canonical strict matrix is missing ${gateId}`);
  }
}
for (const uc of ucDefinitions) {
  assertFile(join(repoRoot, uc.helixTest), `${uc.dir} Helix test`);
  assertFile(join(clientRoot, uc.clientTest), `${uc.dir} cses-client test`);
}

verifyV2OnlySources();
resetDocumentationTree();

const generatedAt = '2026-07-24 PDT';
write('README.md', rootReadme());
write('00-源码核验与版本.md', sourceAudit());
write('01-按UC当前代码矩阵.md', currentMatrix());
write('02-V2当前公共架构.md', commonArchitecture());
write('03-协议版本边界.md', protocolBoundary());
write('04-验收证据边界.md', evidenceBoundary());
write('05-文档完整性检查.md', integrityContract());
write('06-G02-G15-FrameV2字段与验收总索引.md', deepGateIndex());

for (const uc of ucDefinitions) {
  write(`${uc.dir}/README.md`, ucReadme(uc));
  write(`${uc.dir}/01-当前类图.mmd`, ucClassDiagram(uc));
  write(`${uc.dir}/02-当前执行流程.mmd`, ucExecutionFlow(uc));
  write(`${uc.dir}/03-用户交互流程.mmd`, ucUserFlow(uc));
  write(`${uc.dir}/04-Angular组件与交互合同.md`, ucAngularContract(uc));
  write(`${uc.dir}/05-Helix命令与Frame合同.md`, ucHelixContract(uc));
  write(`${uc.dir}/06-Go接口与WS推送.md`, ucGoContract(uc));
  write(`${uc.dir}/07-远端PG与本地SQLite.md`, ucStorageContract(uc));
  if (deepUcDirs.has(uc.dir)) {
    write(`${uc.dir}/08-${uc.dir}全领域字段级类图.mmd`, deepClassDiagram(uc));
    write(`${uc.dir}/09-${uc.dir}完整主链路时序图.mmd`, deepSequenceDiagram(uc));
    write(`${uc.dir}/10-${uc.dir}-Frame阶段与接受值.mmd`, deepFrameStages(uc));
    write(`${uc.dir}/11-${uc.dir}字段级数据合同与测试证据.md`, deepFieldContract(uc));
    write(`${uc.dir}/12-${uc.dir}-完整主链路.mmd`, deepMainChain(uc));
    write(
      `${uc.dir}/13-${uc.dir}-Helix-PC-Mobile-V2验收合同.md`,
      deepAcceptanceContract(uc),
    );
    write(`${uc.dir}/handoff.md`, deepHandoff(uc));
  }
}

console.log(
  `refreshed ${ucDefinitions.length} UC folders / ${includedGateIds.length} gates at ${relative(repoRoot, docsRoot)}`,
);

// Builds the discoverable root index without promoting historical runs to current evidence.
function rootReadme() {
  const rows = ucDefinitions
    .map(
      (uc) =>
        `| [${uc.dir}](./${uc.dir}/README.md) | ${uc.gates.join('、')} | ${uc.title} |`,
    )
    .join('\n');
  return `# InteractionFrame V2 当前代码对接：按 UC

> 源码审计时间：${generatedAt}。本目录只描述当前生产代码，不再保存 Frame V1 历史页，
> 也不复制已完成的 Spec/Plan/Tasks。正式实施历史与验收任务见
> [018 InteractionFrame V2 cutover](../../specs/018-interaction-frame-v2-cutover/)。

## 结论

- Helix Frame producer 是 V2-only：唯一公开类型为 \`InteractionFrameV2\`，常量
  \`INTERACTION_FRAME_SCHEMA_VERSION=2\`，一个 \`Effect::Emit\` 携带一个完整 Frame。
- cses-client Native sink 只接受 \`schemaVersion=2\` 并 opaque 转发；Angular
  \`decodeInteractionFrame\` 同样 fail-closed，随后由 \`SurfaceRuntime\` 原子 apply。
- Frame V1 type/codec/writer/reader/normalize/fixture 已从当前生产树物理消失，因此各 UC 的
  V1 文档已下线；当前 V2 类图、流程图与分层合同仍按 UC 保留。
- 交互入口还不是全量 ActionBinding。当前既有 typed \`ActionBinding -> im_invoke_action\`，
  也有 \`ShellService -> HELIX_COMMAND_NAMES\` direct command；每个 UC 页按实际代码标明。
- \`ActionInvocationEnvelope/v1\`、\`strict-matrix.v1.json\` 和测试报告 \`schemaVersion=1\`
  是独立版本域，不是 Frame V1，禁止误删。

## 依赖方向

\`MessageV4 intent -> CommandPort/ActionInvocation -> Tauri transport -> Helix Module -> Effect -> PersistOk/authority read -> InteractionFrameV2 -> Native opaque sink -> ProjectionBus -> ProjectionStore -> SurfaceRuntime -> typed component\`

Helix 拥有业务事实、权限、排序、ActionBinding 与完整 Surface VM；Angular 只保留 route、draft、
overlay 和诊断等 shell state，不从 raw event 重建业务事实。

## 阅读顺序

1. [源码核验与版本](./00-源码核验与版本.md)
2. [19 UC / 25 Gate 当前矩阵](./01-按UC当前代码矩阵.md)
3. [V2 当前公共架构](./02-V2当前公共架构.md)
4. [协议版本边界](./03-协议版本边界.md)
5. [验收证据边界](./04-验收证据边界.md)
6. [文档完整性检查](./05-文档完整性检查.md)
7. [G02-G15 FrameV2 字段与验收总索引](./06-G02-G15-FrameV2字段与验收总索引.md)
8. [26 Gate 三仓纵向闭环总 Spec 索引](./07-26Gate三仓纵向闭环总Spec索引.md)
9. [26 Gate Plan/Task 实施索引](./08-26Gate-Plan-Task实施索引.md)

## 业务 UC

| UC 文件夹 | 机器 Gate | 当前业务 |
|---|---|---|
${rows}

当前代码文档矩阵仍是 25 Gate 历史切换口径；新的三仓实施总控将 \`G03a\` 失败重发纳入
Active，因此总分母为 26，入口见上方总 Spec 索引。\`G03\` 文件/媒体上传继续进入 Gap。
\`G17-G27\` 的 19 个扩展 Gate 同样进入 Gap，不与本轮逐 UC 实施混算。
`;
}

function sourceAudit() {
  return `# 源码核验与版本

## 当前锁定

| 仓库 | 当前 HEAD / pin | 代码事实 |
|---|---|---|
| Helix | \`${helixHead}\` | 当前生产树只定义 \`InteractionFrameV2\` |
| cses-client | \`${clientHead}\` | MessageV4 当前工作树；Native/Angular 为 V2-only consumer |
| cses-client Helix pin | \`${clientHelixPin}\` | \`src-tauri/Cargo.toml\` 四个 Helix crate 统一 pin |

当前 Helix HEAD 可能包含 pin 之后的文档或测试提交；产品 Native 实际运行的 Rust 代码以
cses-client pin 为准。本文同时核对当前 Helix 源码与 pin 所在提交，不能把 Helix main 的后续
变化自动当成客户端已接入。

## V2-only 代码证据

- Helix schema/type：\`crates/helix-im/src/interaction_frame.rs\`
- Helix builder/emitter：\`crates/helix-im/src/projector/mod.rs\`
- Helix post-commit composition：\`crates/helix-im/src/surface_composer.rs\`
- Native V2 guard/opaque forwarding：
  \`/System/Volumes/Data/workspace/angular/cses-client/src-tauri/src/features/helix_im/event_sink.rs\`
- Angular strict codec：
  \`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts\`
- Angular atomic runtime：
  \`/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts\`

静态扫描在 Frame 生产/消费目录中未发现 \`InteractionFrameV1\`、
\`normalizeLegacyFrame\`、\`RenderNodeV1\` 或 \`InteractionFrame/v1\`。发现的
\`schemaVersion=1\` 均属于 ActionInvocation、strict evidence/report 等独立合同。

## 当前 Surface 集

\`message.left-dialog-list\`、\`message.unread-summary\`、\`message.timeline\`、
\`message.thread-detail\`、\`message.channel-summary\`、\`message.composer\`、
\`message.member-roster\`、\`message.topic-list\`、\`message.participant-receipts\`。

## 当前入口并非单一路径

- typed action：Angular 从 Frame 中读取 opaque \`ActionBinding\`，复制
  \`surfaceEpoch/observedRevision/actionRef\` 组成 \`ActionInvocationEnvelope/v1\`；
  Tauri 注入 actor 后交给 Helix private command。
- direct command：尚未进入 ActionDispatch 的 UC 仍由 \`ShellService\` 调用
  \`HELIX_COMMAND_NAMES\`。command receipt 只证明调用受理，最终 UI 仍等待 V2 Frame。
- inbound/observation：同步、pong gap、observer 撤回等没有 UI action；由 transport 或
  host observation 进入 Helix，再由提交后的 V2 Frame 更新 MessageV4。

所以“Frame 已全 V2”是真的，“所有交互都已统一成 ActionBinding”是假的。
`;
}

function currentMatrix() {
  const rows = ucDefinitions
    .flatMap((uc) =>
      uc.gates.map((gateId) => {
        const item = canonicalCases.get(gateId);
        const commands = item.commands.map((command) => `\`${command.name}\``).join(' / ');
        const effects = [
          ...new Set(item.commands.flatMap((command) => command.effect)),
        ]
          .map((effect) => `\`${effect}\``)
          .join('、');
        const surfaces = [
          ...new Set(item.projections.map((projection) => projection.render_kind)),
        ]
          .map((surface) => `\`${surface}\``)
          .join('、');
        const persistence = item.persistence.map((name) => `\`${name}\``).join('、');
        return `| ${gateId} | [${uc.dir}](./${uc.dir}/README.md) | ${uc.ingress} | ${commands} | ${effects} | ${surfaces} | ${persistence} |`;
      }),
    )
    .join('\n');
  return `# 19 UC / 25 Gate 当前代码矩阵

本表直接读取 \`crates/helix-im/contracts/strict-matrix.v1.json\` 当前 46 Gate 合同，并只选取
018 cutover 的 25 Gate。canonical Matrix 现在已经包含 \`G-04a\`；旧文档“缺 G04a”的说法
已失效。

| Gate | UC | 当前入口 | Helix command | Effect 类别 | 当前 canonical Frame Surface | 合同 persistence 名称 |
|---|---|---|---|---|---|---|
${rows}

## 字段解释

- “当前 canonical Frame Surface”只列 Strict Matrix 的真实 projection 行，不把“可能受影响”
  的 Surface 扩写成已经同帧提交。例如 G01 当前只列 \`message.timeline\`。
- persistence 列是领域合同名，不自动等于物理 SQLite 表。特别是 \`pending_send\` 是
  Helix 内存 \`PendingSend\` 聚合；持久发送状态写在 \`message\` 表，没有
  \`pending_send\` SQLite 表。
- 写路径只有在匹配的持久化回执或权威 readback 后才能发最终 Frame；纯本地查询可以直接从
  已提交事实投影，但不能凭 command receipt 造业务 VM。
`;
}

function commonArchitecture() {
  return `# InteractionFrame V2 当前公共架构

\`\`\`mermaid
flowchart LR
    UI["MessageV4 component intent"] --> MODE{"当前 UC 入口"}
    MODE -->|"typed binding"| ACTION["ActionInvocationEnvelope/v1"]
    MODE -->|"尚未迁移"| DIRECT["HELIX_COMMAND_NAMES direct command"]
    MODE -->|"sync/observer"| INBOUND["Inbound / HostObservation / Timer"]
    ACTION --> TAURI["Tauri protocol adapter"]
    DIRECT --> TAURI
    INBOUND --> HELIX["ImModule::handle"]
    TAURI --> HELIX
    HELIX --> EFFECT["Effect: HTTP / Storage / Timer"]
    EFFECT --> BARRIER{"PersistOk 或 authority read"}
    BARRIER --> COMPOSE["projector / SurfaceComposer"]
    COMPOSE --> FRAME["InteractionFrameV2 schemaVersion=2"]
    FRAME --> SINK["Native V2 guard + opaque forward"]
    SINK --> BUS["ProjectionBusService"]
    BUS --> STORE["ProjectionStore"]
    STORE --> RUNTIME["SurfaceRuntime atomic stage/apply"]
    RUNTIME --> VIEW["typed MessageV4 components"]
    RUNTIME --> ACK["FrameApplied"]
    ACK --> TAURI
\`\`\`

## 所有权

- Helix domain/application：业务事实、权限、排序、ActionBinding、revision、surfaceEpoch、
  causation、directive 与完整 VM。
- driver/infrastructure：HTTP/WS/SQLite 执行 Effect；不重建业务事实。
- Tauri interface：只校验 Frame envelope/schema 并 opaque 转发；不 reduce mutation。
- Angular application/interface：codec 校验、slot staging、原子发布、ack 与渲染；draft、route、
  overlay 可留在 shell，但不能成为服务端事实真源。

## 复杂度

Helix 每帧对 mutation 单次验证，Angular 用 \`Map\` 按 canonical slot key 查找并先 stage
后 publish，因此 apply 为 \`O(m)\` 时间、\`O(m)\` staging 空间。不得退化为每个 Surface
扫描全部 mutation 的 \`O(s*m)\`，也不得逐 mutation 穿越 Tauri/FFI 边界。
`;
}

function protocolBoundary() {
  return `# 协议版本边界

| 版本域 | 当前版本 | 是否 Frame V1 | 处理 |
|---|---:|---|---|
| \`InteractionFrame\` | 2 | 是本目录目标 | producer/consumer 只接受 2 |
| \`ActionInvocationEnvelope\` | 1 | 否 | 保留；它版本化的是 action ingress |
| \`helix-im-strict-matrix/v1\` | v1 | 否 | 保留；它版本化的是合同文件 |
| WDIO/strict evidence/report | 1 | 否 | 保留；它版本化的是测试产物 |
| actionRef 文本前缀 \`act.v1.*\` | v1 | 否 | opaque capability，不按 Frame V1 删除 |

## V2 envelope

\`schemaVersion + transactionId + causationId? + runtime + mutations[] + directives[]\`

每个 mutation 还必须携带 \`surface + scope + surfaceEpoch + revision + baseRevision? +
causationId? + mutation\`。Helix 与 Angular 都拒绝未知根字段、空 mutation、重复 slot、
scope/surface 不匹配、非法 revision 和不完整 payload。

## 已下线内容

Frame V1 type、writer、reader、legacy normalize、V1 fixture、V1 capability 声明以及各 UC
的 V1 文档页。Git 历史仍可追溯迁移过程，但当前文档不再把历史结构伪装成可选生产路径。
`;
}

function evidenceBoundary() {
  return `# 验收证据边界

## 静态代码结论

本次文档更新可以证明当前源码与 pin 的 Frame producer/consumer 为 V2-only，也可以证明
每个 UC 当前使用 direct、ActionBinding 或 inbound 哪一种入口。

## 不能由静态代码单独证明

- 某次真实运行是否经过 fresh Go HTTP/WS；
- PG 与当前服务实际写入 DSN 是否同源；
- SQLite 是否完成匹配事务；
- Actor/Observer 是否收到同一业务事实；
- Frame 是否被 Native 转发、Angular applied 并回送 matching \`FrameApplied\`；
- DOM/截图是否展示该 fresh marker。

这些运行态结论必须绑定同一 \`runId/reqId/transactionId/causationId\`，分别保留 Go、PG、
SQLite、Frame、DOM 和双端证据。Mock fixture、command receipt、HTTP 200 或单端 DOM
都不能升级为 dual-real-chain Green。

## 当前证据入口

- Helix typed contract：\`crates/helix-im/tests/interaction_frame_contract_test.rs\`
- Helix 25 Gate tests：各 UC README 的专属测试
- cses-client codec/runtime：\`surface-contract.codec.test.ts\`、
  \`surface-runtime.service.test.ts\`
- Native/WDIO：\`/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/\`
- 018 历史验收：\`specs/018-interaction-frame-v2-cutover/\`
`;
}

// Defines the generated documentation inventory and its fail-closed verification commands.
function integrityContract() {
  return `# 文档完整性检查

## 结构不变量

- 19 个业务 UC 文件夹；
- 25 个且仅 25 个 machine Gate；
- \`G03/G03a\` 不进入分母；
- 每个 UC 文件夹保留 README 索引和 7 个当前代码细分面；
- G02/G04/G05/G06/G07/G08/G10/G13/G14/G15 各追加 7 个字段级、主链路、
  三端验收与 handoff 文档；
- 不存在文件名或正文把 Frame V1 描述为当前/可选路径；
- 不复制逐 UC Spec/Plan/Tasks，唯一实施历史位于 \`specs/018-*\`；
- 根矩阵中的 command、effect、projection、persistence 必须来自当前
  \`strict-matrix.v1.json\`。

## 建议校验

\`\`\`bash
node scripts/refresh_interaction_frame_v2_docs.mjs
python3 scripts/verify-interaction-frame-v2-index.py
find docs/interaction-frame-v2-全量替换-按UC -type f | sort
rg -n 'InteractionFrameV1|normalizeLegacyFrame|RenderNodeV1|InteractionFrame/v1' \\
  crates/helix-im/src \\
  /System/Volumes/Data/workspace/angular/cses-client/src-tauri/src/features/helix_im \\
  /System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4
git diff --check
\`\`\`

刷新脚本会先验证双仓 V2-only sentinel、统一 pin 和 25 Gate，再机械重建当前代码页；
任一前置漂移都会 fail-closed，不会继续写文档。
`;
}

// Builds one UC landing page and links the deep contract set only where it is generated.
function ucReadme(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commands = [
    ...new Set(cases.flatMap((item) => item.commands.map((command) => command.name))),
  ];
  const effects = [
    ...new Set(
      cases.flatMap((item) =>
        item.commands.flatMap((command) => command.effect),
      ),
    ),
  ];
  const surfaces = [
    ...new Set(
      cases.flatMap((item) =>
        item.projections.map((projection) => projection.render_kind),
      ),
    ),
  ];
  const persistence = [
    ...new Set(cases.flatMap((item) => item.persistence)),
  ];
  const http = cases
    .map((item) => `- ${item.gate_id}: ${item.http.request}`)
    .join('\n');
  const ws = cases
    .map((item) => `- ${item.gate_id}: ${item.ws.events}`)
    .join('\n');
  const matrixEvidence = cases
    .map((item) => {
      const line = lineOfGate(item.gate_id);
      return `\`crates/helix-im/contracts/strict-matrix.v1.json:${line}\``;
    })
    .join('、');
  const deepDocs = uc.dir === 'G01'
    ? `

## G01 字段级深挖

8. [全领域字段级类图](./08-G01全领域字段级类图.mmd)
9. [发送全链路时序图](./09-G01发送全链路时序图.mmd)
10. [三个 Frame 与状态纠错](./10-G01三个Frame与状态纠错.mmd)
11. [字段级数据合同与测试证据](./11-G01字段级数据合同与测试证据.md)

最重要的当前事实：普通发送入口是 direct \`im_send_message\`，不是
\`ActionBinding(message.create)\`；producer-backed direct 成功路径是 \`sending\` 与
\`sent\` 两个 \`message.timeline\` Frame。若随后收到可 apply 的 WS \`post\`，
\`DialogPatchByPost\` 还可能产生含 \`message.left-dialog-list\`（未读变化时配对
\`message.unread-summary\`）的独立 Frame；它不是固定第三帧，也不能假定与
\`sent\` 的先后。`
    : deepUcDirs.has(uc.dir)
      ? `

## ${uc.dir} 字段级主链路与三端验收

8. [全领域字段级类图](./08-${uc.dir}全领域字段级类图.mmd)
9. [完整主链路时序图](./09-${uc.dir}完整主链路时序图.mmd)
10. [Frame 阶段与接受值](./10-${uc.dir}-Frame阶段与接受值.mmd)
11. [字段级数据合同与测试证据](./11-${uc.dir}字段级数据合同与测试证据.md)
12. [完整主链路总览](./12-${uc.dir}-完整主链路.mmd)
13. [Helix、PC、Mobile V2 验收合同](./13-${uc.dir}-Helix-PC-Mobile-V2验收合同.md)
14. [当前 handoff](./handoff.md)

这些文档固定区分 \`CURRENT/HISTORICAL/TARGET/BLOCKED/FORBIDDEN\`。历史 25-Gate
Native 产物只作 \`HISTORICAL\` 输入；没有用当前 pin 重新运行时不得写成当前 Green。`
      : '';
  return `# ${uc.dir}：${uc.title}

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

## 细分文档

1. [当前类图](./01-当前类图.mmd)
2. [当前执行流程](./02-当前执行流程.mmd)
3. [用户交互流程](./03-用户交互流程.mmd)
4. [Angular 组件与交互合同](./04-Angular组件与交互合同.md)
5. [Helix 命令与 Frame 合同](./05-Helix命令与Frame合同.md)
6. [Go 接口与 WS 推送](./06-Go接口与WS推送.md)
7. [远端 PG 与本地 SQLite](./07-远端PG与本地SQLite.md)${deepDocs}

## 当前对接

| 项目 | 当前代码事实 |
|---|---|
| Machine Gate | ${uc.gates.map((gate) => `\`${gate}\``).join('、')} |
| 用户/系统入口 | ${uc.entry} |
| 入口模式 | ${uc.ingress} |
| Helix command | ${commands.map((command) => `\`${command}\``).join('、')} |
| Effect 类别 | ${effects.map((effect) => `\`${effect}\``).join('、')} |
| canonical Frame Surface | ${surfaces.map((surface) => `\`${surface}\``).join('、')} |
| 合同 persistence | ${persistence.map((name) => `\`${name}\``).join('、')} |
| MessageV4 组件 | ${uc.components} |

## 实际调用链

${uc.clientFlow}

\`-> Tauri command/event adapter -> ImModule::handle -> Effect -> matching PersistOk/authority read
-> projector/SurfaceComposer -> InteractionFrameV2 -> Native opaque sink -> ProjectionBusService
-> ProjectionStore -> SurfaceRuntime -> typed component\`

纯本地已提交事实查询可以不新增持久化；任何写入型事实都不能用 HTTP/command receipt 代替
匹配的提交屏障。${uc.result}

## HTTP / WS

${http}

${ws}

HTTP/WS 是基础设施证据，不是 DOM 真源；MessageV4 不直接消费这些 raw payload。

## 源码证据

- Gate 合同：${matrixEvidence}
- Helix Frame schema：\`crates/helix-im/src/interaction_frame.rs\`
- Helix 专属测试：\`${uc.helixTest}\`
- cses-client command/action：\`src/pages/message-v4/shell.service.ts\`、
  \`src/pages/message-v4/actions/surface-action-invocation.ts\`
- cses-client 组件：\`src/pages/message-v4/message.component.ts\`、
  \`src/pages/message-v4/components/message-surface-runtime-view.component.ts\`
- cses-client 专属测试：\`${uc.clientTest}\`

## 当前边界

- Frame producer/Native reader/Angular codec 均只接受 V2。
- 本页 Surface 只按 canonical Matrix 列出；更广的业务影响面必须等代码真实提交对应 mutation
  后才能加入。
- \`pending_send\` 若出现在合同中，指 Helix 内存聚合；物理发送状态在 SQLite \`message\`
  行，不存在 \`pending_send\` 表。
`;
}

function ucClassDiagram(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commands = unique(
    cases.flatMap((item) => item.commands.map((command) => command.name)),
  );
  const surfaces = unique(
    cases.flatMap((item) =>
      item.projections.map((projection) => projection.render_kind),
    ),
  );
  const persistence = unique(cases.flatMap((item) => item.persistence));
  return `classDiagram
direction LR

class MessageV4 {
  +intent()
  +renderTypedVm()
}
class CommandIngress {
  +commands: ${mermaidText(commands.join(', '))}
}
class ImModule {
  +handle()
  +produceEffects()
}
class PersistBarrier {
  +contracts: ${mermaidText(persistence.join(', ') || 'none')}
}
class InteractionFrameV2 {
  +schemaVersion: 2
  +surfaces: ${mermaidText(surfaces.join(', '))}
}
class SurfaceRuntime {
  +stage()
  +atomicApply()
  +frameApplied()
}

MessageV4 --> CommandIngress : intent
CommandIngress --> ImModule : command/inbound
ImModule --> PersistBarrier : Effect
PersistBarrier --> InteractionFrameV2 : committed facts
InteractionFrameV2 --> SurfaceRuntime : opaque frame
SurfaceRuntime --> MessageV4 : typed VM
`;
}

function ucExecutionFlow(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commands = unique(
    cases.flatMap((item) => item.commands.map((command) => command.name)),
  ).join(' / ');
  const effects = unique(
    cases.flatMap((item) =>
      item.commands.flatMap((command) => command.effect),
    ),
  ).join(' / ');
  const surfaces = unique(
    cases.flatMap((item) =>
      item.projections.map((projection) => projection.render_kind),
    ),
  ).join(' / ');
  return `flowchart LR
    A["${mermaidText(uc.entry)}"] --> B["${mermaidText(uc.ingress)}"]
    B --> C["${mermaidText(commands)}"]
    C --> D["ImModule::handle"]
    D --> E["Effect: ${mermaidText(effects)}"]
    E --> F{"PersistOk / authority read"}
    F --> G["InteractionFrameV2 schemaVersion=2"]
    G --> H["Surface: ${mermaidText(surfaces)}"]
    H --> I["Native opaque sink"]
    I --> J["SurfaceRuntime atomic apply"]
    J --> K["${mermaidText(uc.title)} UI"]
`;
}

// Renders the base UC sequence while hardening only the selected deep-GN labels.
function ucUserFlow(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const sequenceLabel = deepUcDirs.has(uc.dir) ? mermaidSequenceText : mermaidText;
  const commands = unique(
    cases.flatMap((item) => item.commands.map((command) => command.name)),
  ).join(' / ');
  const surfaces = unique(
    cases.flatMap((item) =>
      item.projections.map((projection) => projection.render_kind),
    ),
  ).join(' / ');
  return `sequenceDiagram
    actor User as 用户或系统
    participant UI as MessageV4
    participant Port as CommandPort
    participant Helix as Helix IM
    participant Infra as HTTP/WS/SQLite
    participant Runtime as SurfaceRuntime

    User->>UI: ${sequenceLabel(uc.entry)}
    UI->>Port: ${sequenceLabel(uc.clientFlow)}
    Port->>Helix: ${sequenceLabel(commands)}
    Helix->>Infra: Effect batch
    Infra-->>Helix: PersistOk / authority result
    Helix-->>Runtime: InteractionFrameV2 (${sequenceLabel(surfaces)})
    Runtime-->>UI: atomic typed VM publish
    Runtime-->>Helix: FrameApplied
`;
}

function ucAngularContract(uc) {
  return `# ${uc.dir} Angular 组件与交互合同

## 当前组件

${uc.components}

## 当前入口

- 用户/系统入口：${uc.entry}
- 入口模式：${uc.ingress}
- 调用链：${uc.clientFlow}

## 消费边界

- Angular 只接受 \`schemaVersion=2\`。
- \`ProjectionBusService -> ProjectionStore -> SurfaceRuntime\` 先 stage 全帧，再原子 publish。
- 组件只读取 typed Surface VM，不从 raw HTTP、WS、props 或 command receipt 重建业务事实。
- route、draft、overlay 可留在 shell；服务端事实、权限、排序、revision 和 ActionBinding 属于 Helix。

## 验证

- 专属测试：\`${uc.clientTest}\`
- 公共 codec/runtime：\`surface-contract.codec.test.ts\`、\`surface-runtime.service.test.ts\`
`;
}

function ucHelixContract(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const rows = cases
    .flatMap((item) =>
      item.commands.map((command) => {
        const effects = command.effect.map((effect) => `\`${effect}\``).join('、');
        return `| \`${item.gate_id}\` | \`${command.name}\` | ${effects} |`;
      }),
    )
    .join('\n');
  const surfaces = unique(
    cases.flatMap((item) =>
      item.projections.map((projection) => projection.render_kind),
    ),
  )
    .map((surface) => `\`${surface}\``)
    .join('、');
  return `# ${uc.dir} Helix 命令与 Frame 合同

## 命令与 Effect

| Gate | command | Effect |
|---|---|---|
${rows}

## Frame

- producer：\`ImModule::handle -> projector/SurfaceComposer -> Effect::Emit\`
- schema：\`InteractionFrameV2\`，\`schemaVersion=2\`
- canonical Surface：${surfaces}
- 写路径只有在 matching \`PersistOk\` 或权威 readback 后才能发布终态 Frame。
- 一个 \`Effect::Emit\` 携带一个完整 Frame，driver 不逐 mutation 拆分。

## 源码与测试

- \`crates/helix-im/src/interaction_frame.rs\`
- \`crates/helix-im/src/projector/mod.rs\`
- \`${uc.helixTest}\`
`;
}

function ucGoContract(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const http = cases
    .map((item) => `| \`${item.gate_id}\` | ${item.http.request} |`)
    .join('\n');
  const ws = cases
    .map((item) => `| \`${item.gate_id}\` | ${item.ws.events} |`)
    .join('\n');
  return `# ${uc.dir} Go 接口与 WS 推送

## HTTP

| Gate | 当前 Strict Matrix 请求 |
|---|---|
${http}

## WS

| Gate | 当前 Strict Matrix 事件 |
|---|---|
${ws}

## 边界

- HTTP accepted 只证明基础设施受理，不能代替 Helix 提交终态。
- raw WS 进入 Helix inbound/sync，不允许 MessageV4 直接修改 DOM。
- 运行态 Green 必须用同一 correlation 串联 fresh Go、PG、SQLite、Frame、DOM 和双端证据。
`;
}

function ucStorageContract(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const persistence = unique(cases.flatMap((item) => item.persistence));
  const rows = persistence
    .map((name) => {
      const physical =
        name === 'pending_send'
          ? 'Helix 内存 PendingSend；SQLite 状态位于 message 行'
          : '领域 persistence 名称；物理表须以 StorageOp/schema 当前代码核验';
      return `| \`${name}\` | ${physical} |`;
    })
    .join('\n');
  return `# ${uc.dir} 远端 PG 与本地 SQLite

## 当前 persistence 合同

| 合同名 | 物理边界 |
|---|---|
${rows || '| 无 | 本 Gate 不声明持久化合同 |'}

## 提交规则

- Strict Matrix 的 persistence 是领域合同名，不自动等于 SQLite 或 PG 表名。
- 本地写入必须由 matching \`PersistOk\` 关闭 barrier；远端写入还需权威响应或后续 WS/sync 事实。
- cursor 不得在消息或投影尚未持久化时单独前移。
- 静态文档不证明真实 PG DSN、SQLite 文件或当前进程已写入；运行验收必须单独取证。
`;
}

// Builds the cross-Gate field registry shared by every deep acceptance document.
function deepGateIndex() {
  const selectedCases = ucDefinitions
    .filter((uc) => deepUcDirs.has(uc.dir))
    .flatMap((uc) => uc.gates.map((gateId) => canonicalCases.get(gateId)));
  const commandRows = selectedCases
    .flatMap((item) =>
      item.commands.map((command) => {
        const required = command.input.required.map((field) => `\`${field}\``).join('、') || '无';
        const optional = command.input.optional.map((field) => `\`${field}\``).join('、') || '无';
        const effects = command.effect.map((effect) => `\`${effect}\``).join('、');
        return `| \`${item.gate_id}\` | \`${command.name}\` | ${required} | ${optional} | ${command.input.forbid_unknown ? '是' : '否'} | ${effects} |`;
      }),
    )
    .join('\n');
  const surfaceRows = [
    ['message.timeline', 'Timeline{channelId,windowToken}', 'TimelineSnapshot/TimelinePatch', 'TimelineWindowVm'],
    ['message.channel-summary', 'Channel{channelId}', 'ChannelSummarySnapshot/ChannelSummaryPatch', 'ChannelSummaryVm'],
    ['message.thread-detail', 'Thread{channelId,rootMessageId}', 'ThreadDetailSnapshot/ThreadDetailPatch', 'ThreadDetailVm'],
    ['message.member-roster', 'MemberRoster{channelId,cursor}', 'MemberRosterSnapshot/MemberRosterPatch', 'MemberRosterVm'],
    ['message.participant-receipts', 'ParticipantReceipts{channelId,messageId,receiptKind}', 'ParticipantReceiptsSnapshot/ParticipantReceiptsPatch', 'ParticipantReceiptsVm'],
  ]
    .map((row) => `| \`${row[0]}\` | \`${row[1]}\` | \`${row[2]}\` | \`${row[3]}\` |`)
    .join('\n');
  return `# G02-G15 InteractionFrameV2 字段与验收总索引

> 查证日期：${generatedAt}。范围只包含 G02、G04、G05、G06、G07、G08、G10、
> G13、G14、G15 及其 a 子 Gate；G01 仅作为平台能力和文档形态参考。

## 1. 统一所有权

\`\`\`text
UI intent / Mobile JS intent
-> canonical command 或 ActionInvocationEnvelope
-> Helix Tick::Command / Tick::Inbound / PortReply
-> Effect batch
-> matching PersistOk 或权威 readback
-> InteractionFrameV2
-> PC SurfaceRuntime / Mobile SurfaceRuntime 原子 apply
-> FrameApplied
-> typed DOM / im.surface.view
\`\`\`

Helix 统一拥有权限、排序、计数、状态、typed VM 和 action。Go 拥有 HTTP、WS 与远端
持久化事实。PC/Mobile 只做协议转换、slot apply、ack 与展示，不得各自生长业务 reducer。

## 2. InteractionFrameV2 envelope

| 字段 | 类型与允许值 | 约束 |
|---|---|---|
| \`schemaVersion\` | \`2\` | 其他值 fail closed |
| \`transactionId\` | 非空 string | apply/ack/去重主键 |
| \`causationId?\` | 非空 string | 可选；command/request/authority correlation |
| \`runtime.connection\` | \`connecting/online/offline\` | \`recovered\` 时必须 online |
| \`runtime.initialSync\` | \`idle/syncing/ready/failed\` | renderer 不推导 |
| \`runtime.recovery\` | \`idle/syncing/recovered/failed/blocked\` | 与 surfaces 同帧原子发布 |
| \`mutations\` | 非空数组 | 同一帧禁止重复 canonical slot |
| \`directives\` | \`[{directiveId,kind}]\` | 两字段均非空；执行失败回 ack |

## 3. SurfaceMutationV2

| 字段 | 类型与允许值 | 约束 |
|---|---|---|
| \`surface\` | 下表固定枚举 | 必须与 mutation kind 匹配 |
| \`scope\` | surface 专属 tagged union | 所有身份字段非空 |
| \`revision\` | \`1..=9007199254740991\` | slot 内单调增加 |
| \`baseRevision?\` | 同上 | patch 必填且 \`revision > baseRevision\`；snapshot 禁止 |
| \`surfaceEpoch\` | \`1..=9007199254740991\` | 只能由 Helix 生产 |
| \`causationId?\` | 非空 string | mutation 级 authority correlation |
| \`mutation\` | typed tagged union | patch 为 \`replaceView\`，不暴露局部业务 reducer |

| Surface | Scope | mutation.kind | View |
|---|---|---|---|
${surfaceRows}

## 4. 关键 typed VM 字段

### TimelineWindowVm

\`channelId, anchor{messageId?,mode=latest|locate|unread}, items[], firstUnreadMessageId?,
targetMessageId?, page{windowToken,hasOlder,hasNewer,hasMore},
state=loading|ready|empty|forbidden|failed\`

每个 \`MessageVm\`：

\`id, channelId, eventSeq, author{userId}, createdAt, content, delivery, actions,
quickActions, replyId?, replyRootId?, replyFirstLevelId?, replySummary, replyThread?,
reactions[{emoji,count}], readReceipt, templateConfirmation?, urgency?, pinned,
bookmarked, topicLink?\`

\`content.kind\` 允许：

- \`text{text}\`
- \`document{documentId,title,summary,access}\`
- \`media{mediaType,resource,thumbnail?,fileName,size,availability}\`
- \`forwarded{summary,items}\`
- \`systemEvent{eventKind,memberId?,text}\`
- \`recalled{text}\`

\`delivery.kind\` 允许：
\`uploading{progressPercent=0..100}/sending/sent/failed{reason,retryable}/recalled\`。

### ChannelSummaryVm

\`channelId,title{text},avatar{resource?,userIds,fallbackText},noticeSummary?,purpose?,
lifecycle{state,reason?},memberCount,viewerRole=owner|admin|member,onlineCount?,
topicCount,notificationMode=all|mentions|muted,actions,settingsVersion\`

### ThreadDetailVm

\`channelId,detailContext,root,orderedReplyIds,nodes,replyCount,page,state\`。
\`detailContext\` 为 \`reply\`，或
\`topic{parentChannelId,topicChannelId,rootMessageId,title}\`。

### MemberRosterVm

\`channelId,items,page,memberCount,onlineCount?,state\`；每个成员为
\`user{userId},nickname?,role,presence=online|away|offline,
membershipState=active|left|removed,actions\`。

### ParticipantReceiptsVm

\`channelId,messageId,kind=read|templateConfirmation|urgentConfirmation,summary,
completed[],pending[],page,state\`；每行包含
\`user{userId},memberRole,status=pending|completed,completedAt?\`。

## 5. canonical inbound command

| Gate | command | required | optional | 禁未知字段 | Effect |
|---|---|---|---|---|---|
${commandRows}

上表是 Helix canonical snake_case command。Mobile/Angular 公共 facade 可以接收 camelCase
intent，但必须通过 Helix 共享 normalization 生成同一 canonical payload；不得在两端分别维护
字段别名和默认值。

## 6. ActionInvocationEnvelope

仅 ActionBinding 路径使用：

\`schemaVersion=1, requestId, actorId, surface, scope, surfaceEpoch,
observedRevision, invocation{actionRef,input}\`。

\`schemaVersion=1\` 是 action ingress 版本，不是 InteractionFrame V1。requestId/actorId/
surfaceEpoch/observedRevision/actionRef 均必须非空或正数；stale epoch/revision 必须拒绝。

## 7. 统一证据等级

| 等级 | 能证明 | 不能证明 |
|---|---|---|
| Helix unit/replay | Tick/Effect/Persist barrier/完整 Frame | Tauri、Go、SQLite executor、DOM |
| Fixture parity | 三仓 byte/hash/semantic contract 一致 | 真实 Rust/HTTP/WS |
| WDIO Mock | 真 Angular bus/runtime/DOM 消费完整 Frame | Tauri/Helix/Go 已执行 |
| real invoke | 真 \`window.__TAURI__.core.invoke\` 进入 Rust | Observer/WS/PG/双端完成 |
| single-real | Actor 的 Go/SQLite/Frame/DOM | Observer delivery |
| dual-real | Actor/Observer 同一 authority 闭环 | 未运行的失败/恢复分支 |
| Mobile GN GTest | QuickJS/C++/FFI/V2 apply/ack/view | 真机网络或 PC DOM |
| Venus real | Venus API 到真实 helix-driver-ffi 与 SurfaceRuntime | PC、Go 或双账号自动成立 |

## 8. 当前总 Handoff

- \`CURRENT\`：Helix V2 类型、selected Gate focused tests、PC 旧 desktop scenarios、
  Mobile UC command GTest/real-chain manifest、通用 Mobile/Venus V2 consumer。
- \`HISTORICAL\`：2026-07-22 的旧 Native Gate artifacts；只能帮助定位场景，不能证明当前
  pin、WDIO 平台或 Mobile V2 consumer。
- \`TARGET\`：每个 GN 的 Helix canonical Frame Fixture、provenance/hash、WDIO
  mock/integration/single/dual、GN GTest、Venus Frame 终态断言和统一 Report。
- \`BLOCKED\`：在未重新运行当前 pin 前，所有 GN 的 PC-Mobile canonical Green 均保持
  blocked/unknown。
- \`FORBIDDEN\`：Fixture、command receipt、HTTP 200、旧 artifacts 或单端 DOM 互相替代。
`;
}

// Builds the field-level cross-domain class diagram for one selected GN.
function deepClassDiagram(uc) {
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commands = unique(cases.flatMap((item) => item.commands.map((command) => command.name)));
  const surfaces = unique(cases.flatMap((item) => item.projections.map((projection) => projection.render_kind)));
  return `classDiagram
direction LR

class Intent {
  +gate: ${mermaidText(uc.gates.join(', '))}
  +entry: ${mermaidText(uc.entry)}
}
class CanonicalCommand {
  +name: ${mermaidText(commands.join(', '))}
  +payload: strict object
}
class Tick {
  +Command
  +Inbound
  +PortReply
}
class Effect {
  +Http
  +Persist
  +ScheduleTimer
  +Emit
}
class InteractionFrameV2 {
  +schemaVersion: 2
  +transactionId: string
  +causationId: string?
  +runtime: InteractionRuntimeVm
  +mutations: SurfaceMutationV2[]
  +directives: UiDirective[]
}
class SurfaceMutationV2 {
  +surface: ${mermaidText(surfaces.join(', '))}
  +scope: SurfaceScope
  +revision: u64
  +baseRevision: u64?
  +surfaceEpoch: u64
  +causationId: string?
  +mutation: TypedSurfaceMutation
}
class PcSurfaceRuntime {
  +stage(frame)
  +atomicApply()
  +frameApplied()
  +typedDom()
}
class MobileSurfaceRuntime {
  +ingest(frame)
  +atomicApply()
  +ackNext()
  +viewJson()
}
class Venus {
  +dispatchCanonicalCommand()
  +pumpDomainEvents()
  +im.surface.view()
}

Intent --> CanonicalCommand
CanonicalCommand --> Tick
Tick --> Effect
Effect --> InteractionFrameV2 : committed authority
InteractionFrameV2 *-- SurfaceMutationV2
InteractionFrameV2 --> PcSurfaceRuntime
InteractionFrameV2 --> MobileSurfaceRuntime
MobileSurfaceRuntime --> Venus
`;
}

// Builds the complete authority-to-render sequence without assuming every stage emits a Frame.
function deepSequenceDiagram(uc) {
  const profile = deepUcProfiles[uc.dir];
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commands = unique(cases.flatMap((item) => item.commands.map((command) => command.name))).join(' / ');
  const ws = unique(cases.map((item) => item.ws.events)).join('；');
  return `sequenceDiagram
  actor U as 用户/系统
  participant PC as PC MessageV4
  participant MOB as Mobile QuickJS/Venus
  participant PORT as Tauri/FFI Port
  participant H as Helix ImModule
  participant STORE as SQLite Driver
  participant GO as cses-im-server/PG
  participant WS as Go WS/sync
  participant FRAME as InteractionFrameV2
  participant SR as PC/Mobile SurfaceRuntime

  U->>PC: ${mermaidSequenceText(uc.entry)}
  U->>MOB: 同一业务 intent
  PC->>PORT: ${mermaidSequenceText(uc.ingress)}
  MOB->>PORT: canonical facade command
  PORT->>H: ${mermaidSequenceText(commands)}
  H-->>PORT: command/action receipt 仅表示受理
  H->>STORE: Persist/Scan/Get authority
  STORE-->>H: matching PortReply
  H->>GO: HTTP effect（仅需要时）
  GO-->>H: authority response
  GO-->>WS: durable event/outbox
  WS-->>H: ${mermaidSequenceText(ws)}
  H->>STORE: inbound fact atomic persist/readback
  STORE-->>H: PersistOk
  H-->>FRAME: Effect::Emit full frame
  FRAME->>SR: decode + validate + atomic apply
  SR-->>PORT: FrameApplied(transactionId)
  SR-->>PC: typed DOM
  SR-->>MOB: im.surface.view typed JSON
`;
}

// Builds a Gate-specific stage graph that makes partial ordering and blocked stages explicit.
function deepFrameStages(uc) {
  const profile = deepUcProfiles[uc.dir];
  const nodes = profile.stages
    .map((stage, index) => `  S${index + 1}["${index + 1}. ${mermaidText(stage)}"]`)
    .join('\n');
  const edges = profile.stages
    .slice(1)
    .map((_, index) => `  S${index + 1} --> S${index + 2}`)
    .join('\n');
  return `flowchart TD
  START["${uc.dir} scenario start"] --> S1
${nodes}
${edges}
  S${profile.stages.length} --> ACK["FrameApplied + typed render"]
  ACK --> REPORT["stage report + evidence"]

  VALUES["接受值：${mermaidText(profile.observable)}"]
  RULE["不变量：${mermaidText(profile.invariant)}"]
  DRIFT["当前漂移：${mermaidText(profile.drift)}"]
  VALUES -.约束.-> ACK
  RULE -.fail closed.-> REPORT
  DRIFT -.BLOCKED until reconciled.-> REPORT
`;
}

// Builds the detailed command, wire, Frame, VM, and test evidence contract for one GN.
function deepFieldContract(uc) {
  const profile = deepUcProfiles[uc.dir];
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const commandRows = cases
    .flatMap((item) =>
      item.commands.map((command) => {
        const required = command.input.required.map((field) => `\`${field}\``).join('、') || '无';
        const optional = command.input.optional.map((field) => `\`${field}\``).join('、') || '无';
        return `| \`${item.gate_id}\` | \`${command.name}\` | ${required} | ${optional} | ${command.input.forbid_unknown ? '拒绝' : '允许'} |`;
      }),
    )
    .join('\n');
  const authorityRows = cases
    .map(
      (item) =>
        `| \`${item.gate_id}\` | ${item.http.request} | ${item.http.response} | ${item.ws.events} |`,
    )
    .join('\n');
  const surfaceRows = cases
    .flatMap((item) => item.projections.map((projection) => `| \`${item.gate_id}\` | \`${projection.render_kind}\` | \`${projection.schema}\` |`))
    .join('\n');
  const negative = unique(cases.flatMap((item) => item.negative_assertions))
    .map((assertion) => `- ${assertion}`)
    .join('\n');
  const pcScenarios = uc.gates
    .map((gateId) => `tests/e2e/scenarios/${gateId.toLowerCase()}-${legacyScenarioSlug(gateId)}.scenario.ts`)
    .map((path) => `- \`${path}\``)
    .join('\n');
  return `# ${uc.dir} 字段级数据合同与测试证据

> 状态：合同已梳理；新 WDIO、per-GN Fixture 和 Mobile Frame GTest 尚未因此自动完成。

## 1. Canonical inbound

| Gate | command | required | optional | unknown |
|---|---|---|---|---|
${commandRows}

canonical payload 使用 snake_case，且仅允许表中字段。camelCase 只可存在于公共 facade
intent；PC 与 Mobile 必须委托 Helix 共享 normalization，不能各写一套字段转换。

## 2. HTTP、WS 与远端权威

| Gate | HTTP request | HTTP response | WS/sync inbound |
|---|---|---|---|
${authorityRows}

HTTP 200/accepted 不是 Frame 终态。写路径必须等待 matching PersistOk 或权威 readback；
observer 必须依赖自身 WS/sync/恢复链，不能读取 actor 的 intent。

## 3. InteractionFrameV2

完整 envelope 必须包含：

\`schemaVersion=2, transactionId, causationId?, runtime{connection,initialSync,recovery},
mutations[], directives[]\`。

每个 mutation 必须包含：

\`surface, scope, revision, baseRevision?, surfaceEpoch, causationId?, mutation\`。

| Gate | canonical Surface | schema |
|---|---|---|
${surfaceRows}

本 GN 的 typed 接受值：

${profile.observable}

业务不变量：

${profile.invariant}

## 4. Frame inventory

${profile.stages.map((stage, index) => `${index + 1}. \`${stage}\``).join('\n')}

上述是阶段 inventory，不代表每一阶段都必然独立出 Frame。Fixture producer 必须从真实
\`Tick/PortReply/Inbound -> ImModule::handle -> Effect::Emit\` 捕获 actual Frame，再固定
允许的部分顺序、可选 Frame 和终态。

## 5. 负向合同

${negative}
- schemaVersion 非 2、未知字段、重复 slot、scope/surface 不匹配、patch 缺 baseRevision
  均必须 fail closed。
- PC/Mobile 不得从 raw WS、HTTP、SQLite row 或 command receipt 重建 typed VM。

## 6. 当前测试证据

- Helix focused：\`${uc.helixTest}\`
- cses-client focused：\`${uc.clientTest}\`
- 历史 desktop scenario：
${pcScenarios}
- Mobile command/UC GTest：\`${profile.mobileTest}\`
- Mobile real-chain manifest：\`real-chain/uc-manifest.json\`
- Mobile/Venus 通用 V2 runtime：\`cpp/SurfaceRuntime.cpp\`、
  \`venus/HelixImQuickJsBinding.cpp\`、\`venus/api/ImSurfaceApi.cpp\`

## 7. 当前 CONTRACT_DRIFT

${profile.drift}

在该漂移被 producer replay、Matrix 和三端 Fixture 同时对齐前，本 GN 最大 claim 为
\`contract-declared/source-focused\`，不能写 \`PC-Mobile canonical Green\`。
`;
}

// Builds the compact end-to-end flow used as the implementation and review entry point.
function deepMainChain(uc) {
  const profile = deepUcProfiles[uc.dir];
  return `flowchart LR
  I["Intent<br/>${mermaidText(uc.entry)}"]
  C["Canonical ingress<br/>${mermaidText(uc.ingress)}"]
  H["Helix<br/>Tick -> Effect -> Persist barrier"]
  A["Authority<br/>Go HTTP / WS / SQLite / PG"]
  F["InteractionFrameV2<br/>${mermaidText(profile.observable)}"]
  PC["PC<br/>WDIO Mock -> real invoke -> dual real"]
  MOB["Mobile<br/>GN GTest -> FFI -> Venus"]
  R["Unified report<br/>command/effect/store/frame/ack/render"]

  I --> C --> H
  H --> A --> H
  H --> F
  F --> PC --> R
  F --> MOB --> R

  X["FORBIDDEN<br/>端侧业务 reducer / raw fallback / receipt 冒充终态"]
  X -.fail closed.-> PC
  X -.fail closed.-> MOB
`;
}

// Builds the executable Helix-PC-Mobile acceptance contract while keeping claims layered.
function deepAcceptanceContract(uc) {
  const profile = deepUcProfiles[uc.dir];
  const cases = uc.gates.map((gateId) => canonicalCases.get(gateId));
  const surfaces = unique(cases.flatMap((item) => item.projections.map((projection) => projection.render_kind)));
  return `# ${uc.dir} Helix、PC、Mobile V2 验收合同

> 本文冻结目标验收合同，不把已有 focused test、旧 desktop artifact 或 command GTest
> 写成当前三端 Green。

## 1. 完成定义

\`${uc.dir}\` canonical Green =

\`Helix producer replay ∩ canonical Fixture/provenance ∩ PC WDIO Mock ∩ real invoke
∩ single/dual real ∩ console+Tauri/Helix logs ∩ FrameApplied+DOM
∩ Mobile GN GTest ∩ Venus real FFI/apply/ack/view ∩ unified report\`。

## 2. Helix producer

- 从真实 Tick/PortReply/Inbound 生成完整 Frame，不手写 JSON 作为 producer。
- 断言 command、Effect 顺序、Persist barrier、authority correlation、Frame inventory、
  typed VM 与负向/幂等/恢复路径。
- Fixture inventory 至少覆盖：${profile.stages.map((stage) => `\`${stage}\``).join('、')}。
- Fixture 带 Helix commit、contract epoch、raw SHA-256、semantic SHA-256、surface inventory。
- 普通测试只比较 Golden；显式 UPDATE 开关才允许更新。

当前 focused 入口：\`${uc.helixTest}\`。

## 3. PC WDIO

| Lane | Required assertion | 最大 claim |
|---|---|---|
| contracts | schema/provenance/hash/inventory/negative decode | fixture-contract |
| mock | Accepted 与 Frame replay 分离；真实 bus/runtime/DOM | mock |
| integration | 真 Tauri invoke 进入 Helix；日志可相关 | integration |
| single-real | Actor Go/SQLite/FrameApplied/DOM | single-real-chain |
| dual-real | Actor/Observer 同 authority；各自 FrameApplied/DOM | dual-real-chain |
| report | stage、artifact、redaction、claim boundary | reporting-contract |

禁止在 \`wdio.conf.ts\` 写 ${uc.dir} 分支。Platform 提供 lane/candidate/hooks；通用模块提供
Mock、real invoke、multiremote、assertion、report、observability；${uc.dir} domain 只提供
Fixture、page workflow、业务 stage 与 typed assertion。

## 4. PC 断言

- Surface：${surfaces.map((surface) => `\`${surface}\``).join('、')}。
- 接受值：${profile.observable}。
- 不变量：${profile.invariant}。
- 每个 required transaction 都有 matching FrameApplied，无 FrameRejected。
- DOM 只在 apply 后标绿，不通过 fixed sleep、raw event 或 store mutation 取巧。
- Actor/Observer profile、auth、device、SQLite、WebDriver port 和 WS session 隔离。

## 5. Mobile GN GTest

目标文件：

\`tests/gtest/surface_runtime/${uc.dir}FrameContractTest.cpp\`

必须覆盖：

1. 同源 Fixture byte/hash parity。
2. \`schemaVersion=2\` decode；V1/未知字段/重复 slot/坏 revision fail closed。
3. C++ \`SurfaceRuntime::ingest\` 原子 apply 与 transaction ack。
4. QuickJS facade 产生与 PC 同一 canonical command/payload。
5. \`im.surface.view\` 返回与 PC typed VM 等价的字段和值。
6. duplicate/out-of-order/patch base mismatch/late authority 幂等。
7. 不出现 raw business reducer 或 Mobile-owned counter/status。

现有 command 层入口：\`${profile.mobileTest}\`；它不能替代上述 Frame GTest。

## 6. Venus 真实接码

\`\`\`text
Venus ImRootApi
-> HelixImQuickJsBinding::dispatchCommand
-> HelixImDriverRuntime / helix-driver-ffi
-> CppHostVtable.on_domain_events_batch
-> enqueueDomainEvents / pumpDomainEvents
-> mobile_surface_runtime_ingest_event
-> SurfaceRuntime atomic apply
-> mobile_surface_runtime_ack_next
-> __helix_frame_applied
-> im.surface.view
\`\`\`

Venus 必须使用同一 Helix command builder 与 Frame consumer，禁止针对 ${uc.dir} 在 JS/C++
增加业务状态机。

## 7. 日志与 Report

同一 \`suiteRunId/scenarioRunId\` 至少关联：

\`gateId,operationId,requestId/reqId,causationId,transactionId,channelId,
messageId/postId,eventSeq,surface,scope,revision,surfaceEpoch,FrameApplied,DOM/view\`。

Browser console 保存 action、IPC admission、FrameReceived/Applied/Rejected、DOM latch。
Tauri/Helix 保存 command、Tick、Effect、Persist/HTTP/Inbound、Effect::Emit。
Go/PG/WS 保存 track、commit/outbox/delivery/readback。日志不得保存 token、cookie、密码或
完整敏感 payload；redaction 失败即 run 失败。

报告 stage：

${profile.stages.map((stage) => `- \`${uc.dir.toLowerCase()}.${stage}\``).join('\n')}
- \`${uc.dir.toLowerCase()}.pc-mobile-parity\`
- \`${uc.dir.toLowerCase()}.correlation\`

每个 stage 输出 \`status,startedAt,completedAt,durationMs,expected,actual,assertions,
evidence,blockedBy,claimBoundary\`。

## 8. 当前 stop condition

${profile.drift}

只有 producer replay、Matrix、Fixture、PC 与 Mobile 断言对同一字段/Surface/终态达成一致，
才进入真实运行；真实前置缺失时保持 BLOCKED，不用 Mock 或历史 artifact 代替。
`;
}

// Builds the current-state handoff with explicit ownership and stop conditions.
function deepHandoff(uc) {
  const profile = deepUcProfiles[uc.dir];
  return `# ${uc.dir} FrameV2 三端 Handoff

> 更新时间：${generatedAt}。本文区分当前事实、历史证据、目标与阻塞。

## CURRENT

- Helix V2 schema 与 typed Surface 已存在。
- Helix focused test：\`${uc.helixTest}\`。
- cses-client typed component test：\`${uc.clientTest}\`。
- cses-client 仍有旧 desktop scenario/artifact，但新 WDIO Platform 下的 ${uc.dir}
  fixture/mock/real specs 尚未因此自动存在。
- Mobile 已有 command/UC GTest \`${profile.mobileTest}\`、real-chain manifest 与通用
  SurfaceRuntime/Venus V2 consumer。

## HISTORICAL

2026-07-22 的 Native Gate artifacts 可用于识别旧场景和失败签名；它们不证明当前 Helix
pin、新 WDIO candidate、Mobile Frame consumer 或当前 Go 进程仍为 Green。

## TARGET

1. Helix 新建 ${uc.dir} scenario replay 与 canonical V2 Fixture。
2. Fixture provenance/hash 同步到 PC 与 Mobile。
3. PC domain 接入 contracts/mock/integration/single-real/dual-real/report。
4. Mobile 新建 \`tests/gtest/surface_runtime/${uc.dir}FrameContractTest.cpp\`。
5. Venus 用真实 helix-driver-ffi 完成 command -> frame -> apply -> ack -> view。
6. 两端对 ${profile.observable} 做字段级等价断言。

## BLOCKED / CONTRACT_DRIFT

${profile.drift}

因此当前最大 claim：

\`source-focused + command-contract + generic-v2-consumer-present\`。

不得声明：

\`current ${uc.dir} PC-Mobile canonical Green\`。

## FORBIDDEN

- PC/Mobile 各自实现状态、排序、计数、权限、分页或恢复业务逻辑。
- 把 AcceptedCommand、ActionReceipt、HTTP 200、raw WS 或 Fixture replay 当业务终态。
- 固定 sleep、固定 Frame 序号，或从旧 artifact 复制动态 ID。
- Fixture 没有 Helix producer provenance/hash。
- 通用 WDIO/Mobile runtime 反向依赖 ${uc.dir} 业务字段。

## NEXT OWNERS

| 工作 | Owner | Handoff 输出 |
|---|---|---|
| Frame producer/Fixture | Helix gate frame producer | actual Frame、Golden、provenance |
| WDIO lane/platform | WDIO Platform | domain-neutral lane/candidate/hooks |
| ${uc.dir} PC workflow | PC Gate domain | page workflow、typed assertions、stage report |
| Mobile GN GTest | Mobile Gate contract | fixture parity、FFI/apply/ack/view |
| Venus integration | Mobile V2 runtime | real driver batch ingress 与 raw fallback=0 |
| 最终裁决 | Cross-platform reviewer | evidence plane 交集与最大 claim |

## STOP CONDITION

- 若 producer/Matrix/typed VM 不一致：标记 \`CONTRACT_DRIFT\`，停止生成 Fixture。
- 若 Fixture provenance/hash 不完整：停止 PC/Mobile Mock。
- 若当前 Go/凭据/PG/双账号不可用：real lane 标记 \`BLOCKED\`，不降级。
- 只有 Helix、PC、Mobile 三方字段与终态一致且报告无越级，才允许 canonical Green。
`;
}

// Maps a machine Gate to the existing legacy desktop scenario filename suffix.
function legacyScenarioSlug(gateId) {
  const slugs = {
    'G-02': 'document',
    'G-04': 'revoke',
    'G-04a': 'offline-revoke',
    'G-05': 'forward',
    'G-06': 'quick-reply',
    'G-07': 'urgent',
    'G-08': 'schedule-create',
    'G-10': 'first-window',
    'G-10a': 'load-older',
    'G-13': 'channel-read',
    'G-14': 'post-read',
    'G-14a': 'template-received',
    'G-15': 'create-topic',
    'G-15a': 'create-channel',
  };
  return slugs[gateId] ?? 'unknown';
}

function unique(values) {
  return [...new Set(values)];
}

function mermaidText(value) {
  return String(value)
    .replaceAll('"', "'")
    .replaceAll('`', '')
    .replaceAll('\n', ' ')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;');
}

// Removes sequence-control tokens while preserving the source label's business meaning.
function mermaidSequenceText(value) {
  return mermaidText(value).replaceAll('-&gt;', '→').replaceAll(';', '；');
}

function verifyV2OnlySources() {
  const helixFrame = readFileSync(
    join(repoRoot, 'crates/helix-im/src/interaction_frame.rs'),
    'utf8',
  );
  const helixProjector = readFileSync(
    join(repoRoot, 'crates/helix-im/src/projector/mod.rs'),
    'utf8',
  );
  const nativeSink = readFileSync(
    join(clientRoot, 'src-tauri/src/features/helix_im/event_sink.rs'),
    'utf8',
  );
  const angularTypes = readFileSync(
    join(
      clientRoot,
      'src/pages/message-v4/surface-runtime/surface-contract.types.ts',
    ),
    'utf8',
  );
  const angularCodec = readFileSync(
    join(
      clientRoot,
      'src/pages/message-v4/surface-runtime/surface-contract.codec.ts',
    ),
    'utf8',
  );

  assertIncludes(
    helixFrame,
    'pub const INTERACTION_FRAME_SCHEMA_VERSION: u32 = 2;',
    'Helix V2 schema constant',
  );
  assertIncludes(
    helixFrame,
    'pub struct InteractionFrameV2',
    'Helix V2 frame type',
  );
  assertIncludes(
    helixProjector,
    'pub(crate) fn emit_interaction_frame(',
    'Helix V2 emitter',
  );
  assertIncludes(
    nativeSink,
    'if schema.data.schema_version != 2',
    'Native V2 guard',
  );
  assertIncludes(
    angularTypes,
    'export const INTERACTION_FRAME_SCHEMA_VERSION = 2 as const;',
    'Angular V2 schema constant',
  );
  assertIncludes(
    angularCodec,
    'if (value.schemaVersion !== INTERACTION_FRAME_SCHEMA_VERSION)',
    'Angular strict schema guard',
  );

  const productionSources = [
    ['Helix Frame', helixFrame],
    ['Helix projector', helixProjector],
    ['Native sink', nativeSink],
    ['Angular types', angularTypes],
    ['Angular codec', angularCodec],
  ];
  const forbidden = [
    'InteractionFrameV1',
    'normalizeLegacyFrame',
    'RenderNodeV1',
    'InteractionFrame/v1',
  ];
  for (const [label, source] of productionSources) {
    for (const token of forbidden) {
      if (source.includes(token)) {
        throw new Error(`${label} still contains forbidden Frame V1 token ${token}`);
      }
    }
  }

  const forbiddenPattern = forbidden.join('|');
  assertNoMatches(
    repoRoot,
    forbiddenPattern,
    ['crates/helix-im/src'],
    '当前 Helix 生产树',
  );
  assertNoMatches(
    clientRoot,
    forbiddenPattern,
    [
      'src-tauri/src/features/helix_im',
      'src/pages/message-v4',
    ],
    '当前 cses-client MessageV4 生产树',
  );

  const pinnedFrame = git(repoRoot, [
    'show',
    `${clientHelixPin}:crates/helix-im/src/interaction_frame.rs`,
  ]);
  assertIncludes(
    pinnedFrame,
    'pub const INTERACTION_FRAME_SCHEMA_VERSION: u32 = 2;',
    'cses-client pin Helix V2 schema constant',
  );
  assertIncludes(
    pinnedFrame,
    'pub struct InteractionFrameV2',
    'cses-client pin Helix V2 frame type',
  );
  let pinnedLegacyMatches = '';
  try {
    pinnedLegacyMatches = execFileSync(
      'git',
      [
        'grep',
        '-n',
        '-E',
        forbiddenPattern,
        clientHelixPin,
        '--',
        'crates/helix-im/src',
      ],
      { cwd: repoRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] },
    ).trim();
  } catch (error) {
    if (error.status !== 1) throw error;
  }
  if (pinnedLegacyMatches) {
    throw new Error(
      `cses-client Helix pin still contains Frame V1 tokens:\n${pinnedLegacyMatches}`,
    );
  }
}

function assertNoMatches(cwd, pattern, roots, label) {
  let matches = '';
  try {
    matches = execFileSync('rg', ['-n', pattern, ...roots], {
      cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    }).trim();
  } catch (error) {
    if (error.status !== 1) throw error;
  }
  if (matches) {
    throw new Error(`${label}仍含 Frame V1 token:\n${matches}`);
  }
}

// Removes generated files while retaining the manually curated G01 deep contracts.
function resetDocumentationTree() {
  const keepDirectories = new Set(ucDefinitions.map((uc) => uc.dir));
  for (const entry of readdirSync(docsRoot, { withFileTypes: true })) {
    const path = join(docsRoot, entry.name);
    if (entry.isDirectory()) {
      if (!keepDirectories.has(entry.name)) {
        rmSync(path, { recursive: true, force: true });
        continue;
      }
      for (const child of readdirSync(path)) {
        if (entry.name === 'G01' && g01DeepDocs.has(child)) continue;
        rmSync(join(path, child), { recursive: true, force: true });
      }
      continue;
    }
    rmSync(path, { force: true });
  }
  for (const dir of keepDirectories) {
    mkdirSync(join(docsRoot, dir), { recursive: true });
  }
}

function write(path, content) {
  const absolute = join(docsRoot, path);
  mkdirSync(join(absolute, '..'), { recursive: true });
  writeFileSync(absolute, `${content.trim()}\n`, 'utf8');
}

function lineOfGate(gateId) {
  const lines = matrixSource.split('\n');
  const needle = `"gate_id": "${gateId}"`;
  const index = lines.findIndex((line) => line.includes(needle));
  if (index < 0) throw new Error(`cannot locate ${gateId} in strict matrix source`);
  return index + 1;
}

function assertIncludes(source, needle, label) {
  if (!source.includes(needle)) {
    throw new Error(`${label} sentinel is missing: ${needle}`);
  }
}

function assertFile(path, label) {
  if (!existsSync(path)) {
    throw new Error(`${label} does not exist: ${path}`);
  }
}

function git(cwd, args) {
  if (!existsSync(cwd)) throw new Error(`missing repository ${cwd}`);
  return execFileSync('git', args, { cwd, encoding: 'utf8' }).trim();
}
