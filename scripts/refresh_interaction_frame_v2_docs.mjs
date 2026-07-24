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

for (const uc of ucDefinitions) {
  write(`${uc.dir}/README.md`, ucReadme(uc));
}

console.log(
  `refreshed ${ucDefinitions.length} UC folders / ${includedGateIds.length} gates at ${relative(repoRoot, docsRoot)}`,
);

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
  V1 文档已下线。本目录不再用“V1 清退中”描述当前状态。
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

## 业务 UC

| UC 文件夹 | 机器 Gate | 当前业务 |
|---|---|---|
${rows}

\`G03/G03a\` 媒体上传不在本次 25 Gate 分母。旧目录中的 \`G17\` 是“会话列表 row delta”
提案，但 canonical \`G-17\` 实际是频道置顶；为避免 Gate 冲突，该提案不再放在本目录。
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

function integrityContract() {
  return `# 文档完整性检查

## 结构不变量

- 19 个业务 UC 文件夹；
- 25 个且仅 25 个 machine Gate；
- \`G03/G03a\` 不进入分母；
- 每个 UC 文件夹只保留一个当前代码入口 \`README.md\`；
- 不存在文件名或正文把 Frame V1 描述为当前/可选路径；
- 不复制逐 UC Spec/Plan/Tasks，唯一实施历史位于 \`specs/018-*\`；
- 根矩阵中的 command、effect、projection、persistence 必须来自当前
  \`strict-matrix.v1.json\`。

## 建议校验

\`\`\`bash
node scripts/refresh_interaction_frame_v2_docs.mjs
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
  return `# ${uc.dir}：${uc.title}

> 当前代码页；Frame V1 已物理下线。本页只描述当前 canonical 路径，不保存迁移前快照。

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
