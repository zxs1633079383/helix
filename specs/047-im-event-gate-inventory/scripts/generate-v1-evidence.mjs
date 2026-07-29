import { createHash } from 'node:crypto';
import {
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
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
    .map((path) => relative(ROOT, path))
    .sort();
}

/** 将 Matrix UC 条目转换为以 Helix 为观察点的入站/出站合同。 */
function contractRecord(uc, sourceLock) {
  const effects = [...new Set(uc.commands.flatMap((command) => command.effect ?? []))];
  return {
    schema_version: 'helix-im-gate-evidence/v1',
    gate_id: uc.gate_id,
    title: uc.title,
    group: gateGroup(uc.gate_id),
    evidence_level: 'DECLARED_CONTRACT',
    direction_basis: 'Helix boundary',
    source_lock: sourceLock,
    ingress: {
      ui_commands: uc.commands.map((command) => ({
        name: command.name,
        input: command.input,
      })),
      websocket_authority: {
        status: 'DECLARED',
        events: uc.ws?.events ?? 'N/A',
      },
      port_responses: {
        http: uc.http?.response ?? 'N/A',
        persistence: 'PersistOk / PersistErr；具体完成屏障以该 Gate producer 测试为准',
      },
    },
    egress: {
      effects,
      http_request: uc.http?.request ?? 'N/A',
      persistence: uc.persistence ?? [],
      projections: uc.projections ?? [],
    },
    negative_assertions: uc.negative_assertions ?? [],
  };
}

/** 建立三个原始捕获槽位；只有非空文件才可从 NOT_CAPTURED 晋级。 */
function captureStatus(gateId, sourceIndexes) {
  const captureDir = join(OUTPUT_DIR, 'gates', gateId, 'captures');
  mkdirSync(captureDir, { recursive: true });
  const slots = Object.fromEntries(
    ['inbound', 'outbound', 'response'].map((name) => {
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
      '源码、fixture、旧 evidence 或测试 PASS 不能替代同一 run 的 inbound/outbound/response 原始捕获。',
  };
}

/** 渲染本地总览，优先展示捕获缺口而不是旧报告的 Green 标签。 */
function renderReadme(records, manifest) {
  const rows = records
    .map(({ contract, status }) => {
      const commands = contract.ingress.ui_commands.map((item) => item.name).join('<br>') || 'N/A';
      const surfaces =
        contract.egress.projections.map((item) => item.render_kind).join('<br>') || 'N/A';
      return `| ${contract.gate_id} | ${contract.group} | ${commands} | ${surfaces} | ${status.captured_slots}/3 | ${status.verdict} |`;
    })
    .join('\n');
  return `# IM Gate 本地证据索引 v1

> 本目录由 \`scripts/generate-v1-evidence.mjs\` 生成并被 Git 忽略。它首先盘点合同与
> 源码入口，不把旧日志或静态测试冒充本轮真实网络/Port 捕获。

## 方向定义

- \`ingress\`：进入 Helix 的 UI command、WS authority、HTTP/Persist PortReply。
- \`egress\`：Helix 产出的 HTTP、Persist、Timer 与 InteractionFrame Effect。
- \`response\`：把请求或权威事件闭合为业务终态的响应证据。

每个 Gate 的 \`contract.json\` 是 Matrix 声明；\`capture-status.json\` 是原始捕获状态。
真实采集文件只能写入 \`captures/inbound.ndjson\`、\`outbound.ndjson\`、
\`response.ndjson\`。空文件仍按 \`NOT_CAPTURED\` 处理。

## 汇总

- Gate：${manifest.summary.gates}
- 必需捕获槽位：${manifest.summary.required_capture_slots}
- 已存在但未验证槽位：${manifest.summary.present_unverified_capture_slots}
- 缺失槽位：${manifest.summary.missing_capture_slots}
- 结论：${manifest.summary.verdict}

| Gate | 分组 | 入站 command | 出站 Surface | 捕获 | 状态 |
|---|---|---|---|---:|---|
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
      ...contract.ingress,
    });
    writeJson(join(gateDir, 'outbound-contract.json'), {
      schema_version: 'helix-im-gate-outbound-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      ...contract.egress,
    });
    writeJson(join(gateDir, 'response-contract.json'), {
      schema_version: 'helix-im-gate-response-contract/v1',
      gate_id: gateId,
      direction_basis: contract.direction_basis,
      http_response: contract.ingress.port_responses.http,
      websocket_authority: contract.ingress.websocket_authority,
      persistence_response: contract.ingress.port_responses.persistence,
      terminal_projections: contract.egress.projections,
      negative_assertions: contract.negative_assertions,
    });
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
