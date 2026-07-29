import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const FEATURE_DIR = resolve(SCRIPT_DIR, '..');
const ROOT = resolve(FEATURE_DIR, '..', '..');
const MATRIX_PATH = join(ROOT, 'crates/helix-im/contracts/strict-matrix.v1.json');
const HELIX_TESTS = join(ROOT, 'crates/helix-im/tests');
const WDIO_SPECS =
  '/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/specs';
const OUTPUT_DIR = join(FEATURE_DIR, '.v1');

const GATE_GROUPS = {
  'sync/recovery': ['G-S1', 'G-S2', 'G-S3', 'G-S4'],
  post: [
    'G-01',
    'G-02',
    'G-03a',
    'G-04',
    'G-04a',
    'G-05',
    'G-06',
    'G-07',
    'G-14',
    'G-14a',
  ],
  schedule: ['G-08', 'G-09'],
  'timeline/read': [
    'G-10',
    'G-10a',
    'G-10b',
    'G-10c',
    'G-10d',
    'G-11',
    'G-11a',
    'G-12',
    'G-13',
  ],
  'channel/channel-member': ['G-15a', 'G-15', 'G-16', 'G-16b', 'G-16c'],
};

const GATE_IDS = Object.values(GATE_GROUPS).flat();

const EVENT_SOURCES = {
  'im:post:sending': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:post:received': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:updated': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:deleted': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:read': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:channel:read_echo': 'crates/helix-im/src/acl/projection_message_effects.rs',
  'im:post:send-failed': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:posts:batch-result': 'crates/helix-im/src/query/render_ready/batch_result.rs',
  'im:post:batch-updated': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:sync:state': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:sync:too_long': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channels:loaded': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channel:increment': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:channel:update': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:operation:status': 'crates/helix-im/src/acl/projection_control_effects.rs',
  'im:messages:query_result': 'crates/helix-im/src/query/core.rs',
  'im:read:result': 'crates/helix-im/src/query/read_relay.rs',
  'im:channel:replies': 'crates/helix-im/src/query/render_ready/replies.rs',
  'im:channel:members': 'crates/helix-im/src/query/render_ready/members.rs',
  'im:post:readers': 'crates/helix-im/src/query/render_ready/receipts.rs',
  'im:channel:created': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:closed': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:left': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:member-updated': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:settings-updated': 'crates/helix-im/src/acl/to_effect_s1.rs',
  'im:channel:schedule-created': 'crates/helix-im/src/acl/to_effect_s1/schedule.rs',
  'im:channel:schedule-canceled': 'crates/helix-im/src/acl/to_effect_s1/schedule.rs',
};

const GATE_EMITS = {
  'G-S1': {
    current: ['im:channel:increment', 'im:channels:loaded'],
    required: ['每个可见频道的增量事实', '全批持久提交后的 loaded 终态'],
  },
  'G-S2': {
    current: [
      'im:post:received',
      'im:post:updated',
      'im:post:deleted',
      'im:post:batch-updated',
      'im:channel:update',
      'im:sync:state',
    ],
    required: ['离线 mutation 的逐事实 emit', 'cursor 与全批提交后的 sync state'],
  },
  'G-S3': {
    current: ['im:sync:state', 'im:sync:too_long'],
    required: ['gap 起止状态', '补偿失败或 too-long 状态'],
  },
  'G-S4': {
    current: ['im:channel:increment', 'im:channel:update', 'im:post:received'],
    required: ['陌生频道补全事实', '触发补全的原消息事实'],
  },
  'G-01': {
    current: ['im:post:sending', 'im:post:received', 'im:post:send-failed'],
    required: ['sender optimistic', 'sender terminal', 'receiver authority'],
  },
  'G-02': {
    current: ['im:post:sending', 'im:post:received', 'im:post:send-failed'],
    required: ['文档 optimistic/terminal', 'receiver 文档 authority'],
  },
  'G-03a': {
    current: ['im:post:sending', 'im:post:received', 'im:post:send-failed'],
    required: ['同 temporaryId 重试状态', '唯一 server id 终态'],
  },
  'G-04': {
    current: ['im:post:updated', 'im:post:deleted', 'im:channel:update'],
    required: ['actor 撤回终态', 'observer 同 postId 撤回事实'],
  },
  'G-04a': {
    current: ['im:post:deleted', 'im:post:batch-updated', 'im:channel:update'],
    required: ['离线撤回重放', '恢复后的频道摘要事实'],
  },
  'G-05': {
    current: ['im:posts:batch-result', 'im:post:received'],
    required: ['每个目标的批结果', '各目标频道 receiver authority'],
  },
  'G-06': {
    current: ['im:post:updated'],
    required: ['reaction 绝对态及 actor/observer 一致 postId'],
  },
  'G-07': {
    current: ['im:post:updated', 'im:post:readers', 'im:channel:update'],
    required: ['加急状态', '确认成员集合', 'viewer-local 频道计数'],
  },
  'G-08': {
    current: ['im:channel:schedule-created', 'im:operation:status'],
    required: ['创建者排程事实', '参与者在真正发送前不得收到 post'],
  },
  'G-09': {
    current: ['im:channel:schedule-canceled', 'im:operation:status'],
    required: ['取消者排程终态', '参与者不得收到已取消 post'],
  },
  'G-10': {
    current: ['im:messages:query_result'],
    required: ['latest window、bounds、revision 与 continuation token'],
    inferred: ['现有 query_result 需要补齐窗口元数据，不能依赖 UI 层另算'],
  },
  'G-10a': {
    current: ['im:messages:query_result'],
    required: ['older page、头部边界与 nextOlder token'],
    inferred: ['当前源码未证明 older 专用终态字段完整'],
  },
  'G-10b': {
    current: ['im:messages:query_result', 'im:post:received'],
    required: ['newer page、尾部边界与 nextNewer token'],
    inferred: ['当前源码未证明 newer 专用终态字段完整'],
  },
  'G-10c': {
    current: ['im:post:received'],
    required: ['append 后窗口淘汰结果、bounds 与 revision'],
    inferred: ['缺少表达头部淘汰后的 raw emit 合同'],
  },
  'G-10d': {
    current: ['im:post:received'],
    required: ['离底 anchor 保持与 new-message indicator'],
    inferred: ['缺少表达 anchor/提示状态的 raw emit 合同'],
  },
  'G-11': {
    current: ['im:read:result', 'im:messages:query_result'],
    required: ['目标 post authority 与仅 actor 生效的定位元数据'],
    inferred: ['当前源码未证明 locate directive 的 raw emit 等价物'],
  },
  'G-11a': {
    current: ['im:messages:query_result', 'im:post:received'],
    required: ['未缓存目标、双向 token、anchor 与 stale-navigation rejection'],
    inferred: ['当前源码未证明定位后双向分页元数据完整'],
  },
  'G-12': {
    current: ['im:read:result', 'im:channel:replies'],
    required: ['thread snapshot、reply ids、revision 与请求关联键'],
  },
  'G-13': {
    current: ['im:channel:read_echo', 'im:channel:update'],
    required: ['actor viewer-local 已读绝对态', 'observer 计数不变'],
  },
  'G-14': {
    current: ['im:post:read', 'im:post:readers'],
    required: ['单 post 已读终态', 'sender/reader 各自可见 receipt'],
  },
  'G-14a': {
    current: ['im:post:updated'],
    required: ['templateReceived、confirmed count 与 reader ids'],
  },
  'G-15a': {
    current: ['im:channel:created', 'im:channel:members'],
    required: ['creator channel authority', '每个 member 的 roster authority'],
  },
  'G-15': {
    current: ['im:channel:created', 'im:post:received', 'im:post:updated'],
    required: ['topic channel authority', 'root post 与参与者可见性'],
  },
  'G-16': {
    current: ['im:channel:closed', 'im:channel:left'],
    required: ['close 与 self-left 两种不可混淆终态', 'observer 对应广播事实'],
  },
  'G-16b': {
    current: ['im:channel:member-updated', 'im:channel:members'],
    required: ['退出成员变化', '各 observer 的新 roster authority'],
  },
  'G-16c': {
    current: ['im:channel:settings-updated', 'im:channel:update'],
    required: ['actor 保存终态', 'observer 最新 settingVersion 与字段绝对态'],
  },
};

/** 将 Gate ID 转成测试文件使用的稳定小写前缀。 */
function gatePrefix(gateId) {
  return gateId.toLowerCase().replaceAll('-', '');
}

/** 返回 Gate 所属业务分组，未知 Gate 立即失败而不是静默归入其他。 */
function gateGroup(gateId) {
  const entry = Object.entries(GATE_GROUPS).find(([, ids]) => ids.includes(gateId));
  if (!entry) {
    throw new Error(`Unknown gate group: ${gateId}`);
  }
  return entry[0];
}

/** 递归列出目录下的文件，目录不存在时返回空列表。 */
function walkFiles(directory) {
  if (!existsSync(directory)) {
    return [];
  }
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? walkFiles(path) : [path];
  });
}

/** 判断文件或 Spec 目录名称是否精确属于目标 Gate，避免 G-10 误收 G-10a。 */
function belongsToGate(path, gateId) {
  const normalized = path.toLowerCase().replaceAll('\\', '/');
  const compact = gatePrefix(gateId);
  const dashed = gateId.toLowerCase();
  const compactPattern = new RegExp(`(?:^|[/_-])${compact}(?:[/_.-]|$)`);
  const dashedPattern = new RegExp(`(?:^|[/_-])${dashed}(?:[/_.-]|$)`);
  return compactPattern.test(normalized) || dashedPattern.test(normalized);
}

/** 计算非空文件的 SHA-256；缺失或空文件不构成真实捕获。 */
function captureDigest(path) {
  if (!existsSync(path) || statSync(path).size === 0) {
    return null;
  }
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

/** 以稳定缩进写入 JSON，保证重复生成可比较。 */
function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

/** 读取当前 Git HEAD，作为本地证据合同的源码锁。 */
function currentHead() {
  return execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: ROOT,
    encoding: 'utf8',
  }).trim();
}

/** 收集 Gate 对应的 Helix producer、fixture 与交互测试源码。 */
function helixSources(gateId) {
  return walkFiles(HELIX_TESTS)
    .filter((path) => belongsToGate(path, gateId))
    .filter((path) => path.endsWith('.rs'))
    .filter((path) => !/(interaction[_-]?frame|surface)/i.test(path))
    .map((path) => relative(ROOT, path))
    .sort();
}

/** 收集 Gate 对应的 WDIO mock、single-real 与 dual-real 源码入口。 */
function wdioSources(gateId) {
  return walkFiles(WDIO_SPECS)
    .filter((path) => belongsToGate(path, gateId))
    .filter((path) => /-(mock|single-real|dual-real)/.test(path))
    .sort();
}

/** 索引历史 Spec evidence 文件但不复制内容或继承其 Green 结论。 */
function historicalEvidence(gateId) {
  return readdirSync(join(ROOT, 'specs'), { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && belongsToGate(entry.name, gateId))
    .flatMap((entry) => walkFiles(join(ROOT, 'specs', entry.name, 'evidence')))
    .filter((path) => !/(interaction[_-]?frame|surface|\.frame-v\d)/i.test(path))
    .map((path) => relative(ROOT, path))
    .sort();
}

/** 将当前源码中的 event 名称转成可审核的 Effect::Emit 合同条目。 */
function emitRecords(gateId) {
  const definition = GATE_EMITS[gateId];
  if (!definition) {
    throw new Error(`Emit contract missing for Gate: ${gateId}`);
  }
  return {
    channels: definition.current.map((event) => ({
      event,
      core_output: 'Effect::Emit',
      envelope: '{event,data}',
      source: EVENT_SOURCES[event] ?? 'MISSING_SOURCE_INDEX',
      source_status: EVENT_SOURCES[event] ? 'CURRENT_SOURCE' : 'MISSING',
    })),
    required_semantics: definition.required,
    inferred_gaps: definition.inferred ?? [],
    coverage_status: definition.inferred?.length ? 'PARTIAL' : 'CURRENT_SOURCE',
  };
}

/** 将 Matrix UC 条目转换为 Tick、内部 Effect 与业务 Emit 三层合同。 */
function contractRecord(uc, sourceLock) {
  const effects = [...new Set(uc.commands.flatMap((command) => command.effect ?? []))];
  const emits = emitRecords(uc.gate_id);
  const tickKinds = ['Tick::Command'];
  if ((uc.ws?.events ?? 'N/A') !== 'N/A' && !String(uc.ws?.events).startsWith('无')) {
    tickKinds.push('Tick::Inbound');
  }
  if ((uc.http?.response ?? 'N/A') !== 'N/A') {
    tickKinds.push('Tick::PortReply');
  }
  if (effects.includes('timer')) {
    tickKinds.push('Tick::Timer');
  }
  return {
    schema_version: 'helix-im-gate-evidence/v1',
    gate_id: uc.gate_id,
    title: uc.title,
    group: gateGroup(uc.gate_id),
    evidence_level: 'DECLARED_CONTRACT',
    direction_basis: 'Helix module boundary',
    source_lock: sourceLock,
    inbound: {
      tick_kinds: tickKinds,
      ui_commands: uc.commands.map((command) => ({
        name: command.name,
        input: command.input,
      })),
      websocket_authority: {
        status: 'DECLARED',
        events: uc.ws?.events ?? 'N/A',
      },
      port_responses: {
        http:
          (uc.http?.response ?? 'N/A') === 'N/A'
            ? 'N/A'
            : 'Tick::PortReply(HttpOk/HttpErr)；原始 body 写入 inbound capture，本目录不继承 Matrix 中的旧 UI 投影描述',
        persistence: 'PersistOk / PersistErr；具体完成屏障以该 Gate producer 测试为准',
      },
    },
    internal_effects: {
      matrix_effect_tags: effects,
      http_request: uc.http?.request ?? 'N/A',
      persistence: uc.persistence ?? [],
    },
    outbound: {
      contract: 'Effect::Emit { event: DomainEventBytes({event,data}) }',
      ...emits,
      adapter_delivery: {
        native: 'EventSink broadcast；Tauri adapter 令 channel=event',
        ffi: '同一步的 DomainEventBytes 通过 on_domain_events_batch 批量交付',
      },
    },
    negative_assertions: (uc.negative_assertions ?? []).filter(
      (assertion) => !/(interaction[_-]?frame|surface|\bframe\b|\bdom\b)/i.test(assertion),
    ),
    omitted_legacy_ui_assertions: (uc.negative_assertions ?? []).filter((assertion) =>
      /(interaction[_-]?frame|surface|\bframe\b|\bdom\b)/i.test(assertion),
    ).length,
  };
}

/** 建立 Tick、Effect 与 Emit 三个原始捕获槽位。 */
function captureStatus(gateId, sourceIndexes) {
  const captureDir = join(OUTPUT_DIR, 'gates', gateId, 'captures');
  mkdirSync(captureDir, { recursive: true });
  const slots = Object.fromEntries(
    ['inbound', 'effects', 'emit'].map((name) => {
      const path = join(captureDir, `${name}.ndjson`);
      const sha256 = captureDigest(path);
      return [
        name,
        {
          path: relative(join(OUTPUT_DIR, 'gates', gateId), path),
          status: sha256 ? 'CAPTURE_PRESENT_UNVERIFIED' : 'NOT_CAPTURED',
          sha256,
        },
      ];
    }),
  );
  const captured = Object.values(slots).filter(
    (slot) => slot.status === 'CAPTURE_PRESENT_UNVERIFIED',
  ).length;
  return {
    schema_version: 'helix-im-gate-capture-status/v1',
    gate_id: gateId,
    slots,
    source_indexes: sourceIndexes,
    captured_slots: captured,
    required_slots: 3,
    verdict: captured === 3 ? 'CAPTURE_SET_PRESENT_UNVERIFIED' : 'INCOMPLETE',
    warning:
      '源码、fixture、旧 evidence 或测试 PASS 不能替代同一 run 的 inbound/effects/emit 原始捕获。',
  };
}

/** 渲染本地总览，优先展示捕获缺口而不是旧报告的 Green 标签。 */
function renderReadme(records, manifest) {
  const rows = records
    .map(({ contract, status }) => {
      const commands = contract.inbound.ui_commands.map((item) => item.name).join('<br>') || 'N/A';
      const channels =
        contract.outbound.channels.map((item) => item.event).join('<br>') || 'MISSING';
      return `| ${contract.gate_id} | ${contract.group} | ${commands} | ${channels} | ${contract.outbound.coverage_status} | ${status.captured_slots}/3 | ${status.verdict} |`;
    })
    .join('\n');
  return `# IM Gate 本地证据索引 v1

> 本目录由 \`scripts/generate-v1-evidence.mjs\` 生成并被 Git 忽略。它首先盘点合同与
> 源码入口，不把旧日志或静态测试冒充本轮真实网络/Port 捕获。

## 方向定义

- \`inbound\`：进入模块的 \`Tick::Command/Inbound/PortReply/Timer\`。
- \`effects\`：模块交给 host 执行的完整 \`Effect\` 流，用于核对 HTTP、Persist、Timer。
- \`emit\`：对端消费的业务出站，只认 \`Effect::Emit\` 中的 \`{event,data}\`。

每个 Gate 的 \`contract.json\` 是 Matrix 声明；\`capture-status.json\` 是原始捕获状态。
Native/Tauri 的 \`channel=event\` 是适配器路由；FFI 按 step 批量交付 event bytes。
真实采集文件只能写入 \`captures/inbound.ndjson\`、\`effects.ndjson\`、
\`emit.ndjson\`。空文件仍按 \`NOT_CAPTURED\` 处理。

## 汇总

- Gate：${manifest.summary.gates}
- 必需捕获槽位：${manifest.summary.required_capture_slots}
- 已存在但未验证槽位：${manifest.summary.present_unverified_capture_slots}
- 缺失槽位：${manifest.summary.missing_capture_slots}
- 结论：${manifest.summary.verdict}

| Gate | 分组 | 入站 command | Emit channel | 合同覆盖 | 捕获 | 状态 |
|---|---|---|---|---|---:|---|
${rows}
`;
}

/** 生成 30 Gate 合同、源码索引、捕获状态和本地总 manifest。 */
function main() {
  const matrix = JSON.parse(readFileSync(MATRIX_PATH, 'utf8'));
  const sourceLock = {
    helix_head: currentHead(),
    matrix_epoch: matrix.contract_epoch,
    matrix_path: relative(ROOT, MATRIX_PATH),
  };
  const ucByGate = new Map(matrix.uc_cases.map((uc) => [uc.gate_id, uc]));
  const records = GATE_IDS.map((gateId) => {
    const uc = ucByGate.get(gateId);
    if (!uc) {
      throw new Error(`Gate missing from Strict Matrix: ${gateId}`);
    }
    const sourceIndexes = {
      helix_tests: helixSources(gateId),
      wdio_sources: wdioSources(gateId),
      historical_evidence: historicalEvidence(gateId),
    };
    const contract = contractRecord(uc, sourceLock);
    const status = captureStatus(gateId, sourceIndexes);
    const gateDir = join(OUTPUT_DIR, 'gates', gateId);
    writeJson(join(gateDir, 'contract.json'), contract);
    writeJson(join(gateDir, 'inbound-contract.json'), {
      schema_version: 'helix-im-gate-inbound-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.inbound,
    });
    writeJson(join(gateDir, 'effects-contract.json'), {
      schema_version: 'helix-im-gate-effects-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.internal_effects,
    });
    writeJson(join(gateDir, 'outbound-contract.json'), {
      schema_version: 'helix-im-gate-outbound-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.outbound,
    });
    rmSync(join(gateDir, 'response-contract.json'), { force: true });
    writeJson(join(gateDir, 'capture-status.json'), status);
    return { contract, status };
  });
  const present = records.reduce((sum, record) => sum + record.status.captured_slots, 0);
  const manifest = {
    schema_version: 'helix-im-gate-evidence-manifest/v1',
    generated_from: sourceLock,
    direction_basis: 'Helix boundary',
    gate_ids: GATE_IDS,
    summary: {
      gates: records.length,
      required_capture_slots: records.length * 3,
      present_unverified_capture_slots: present,
      missing_capture_slots: records.length * 3 - present,
      verdict: present === records.length * 3 ? 'CAPTURE_SET_PRESENT_UNVERIFIED' : 'INCOMPLETE',
    },
  };
  writeJson(join(OUTPUT_DIR, 'manifest.json'), manifest);
  writeFileSync(join(OUTPUT_DIR, 'README.md'), renderReadme(records, manifest));
  process.stdout.write(`${JSON.stringify(manifest.summary)}\n`);
}

main();
