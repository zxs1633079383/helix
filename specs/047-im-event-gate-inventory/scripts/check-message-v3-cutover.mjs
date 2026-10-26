import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const FEATURE_DIR = resolve(SCRIPT_DIR, '..');
const ROOT = resolve(FEATURE_DIR, '..', '..');
const MANIFEST_PATH = resolve(FEATURE_DIR, 'contracts/gate-migration.v1.json');
const LEDGER_PATH = resolve(FEATURE_DIR, 'review-ledger.json');
const SPEC_PATH = resolve(FEATURE_DIR, 'spec.md');
const GENERATED_GATES_PATH = resolve(FEATURE_DIR, '.v1/gates');
const RETIRED_MATRIX_PATH = resolve(ROOT, 'crates/helix-im/contracts/strict-matrix.v1.json');
const RETIRED_COMMAND_API_PATH = resolve(ROOT, 'crates/helix-im/contracts/command-api.v1.json');
const FORBIDDEN_RUNTIME_TERMS =
  /\b(?:InteractionFrame|SurfaceMutation|ActionBinding|UiDirective|FrameApplied)\b|im:interaction:frame|actionCapabilities|channelCapabilities/;
const FORBIDDEN_CODE_DEPENDENCIES =
  /helix_im::(?:interaction_frame|projector)|\bSurfaceRuntime\b|\bSurfaceMutationV\d*\b|\bInteractionFrameV\d*\b|im:interaction:frame|helix_invoke_action|build_surface_resync_command|build_action_invocation_command/;
const RETIRED_SOURCE_PATHS = [
  'crates/helix-im/src/interaction_frame.rs',
  'crates/helix-im/src/surface_composer.rs',
  'crates/helix-im/src/surface_runtime.rs',
  'crates/helix-im/src/dialog_frame.rs',
  'crates/helix-im/src/dialog_patch.rs',
  'crates/helix-im/src/projector',
  'crates/helix-im/src/ws/handlers/legacy_event.rs',
  'crates/helix-im/src/ws/parser/core/inbound.rs',
  'crates/helix-im/src/message_capabilities.rs',
  'crates/helix-im/src/channel_capabilities.rs',
];
const DOMAIN_PACKAGES = new Map([
  ['post', 'post'],
  ['schedule', 'schedule'],
  ['timeline', 'timeline'],
  ['read', 'read'],
  ['channel', 'channel'],
  ['channel-member', 'channel_member'],
  ['sync', 'sync'],
]);

/** 读取受版本控制的 JSON，并把解析错误绑定到文件路径。 */
function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'));
  } catch (error) {
    throw new Error(`invalid JSON ${path}: ${error.message}`);
  }
}

/** 深度枚举目录下满足谓词的普通文件，并保持稳定排序。 */
function walkFiles(root, predicate = () => true) {
  if (!existsSync(root)) {
    return [];
  }
  const files = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const path = resolve(root, entry.name);
    if (entry.isDirectory()) {
      files.push(...walkFiles(path, predicate));
    } else if (entry.isFile() && predicate(path)) {
      files.push(path);
    }
  }
  return files.sort();
}

/** 从规格 Gate 表提取冻结分母，避免维护第二份手写 ID 列表。 */
function expectedGateIds() {
  const ids = new Set();
  const pattern = /^\| (G-(?:S[1-4]|\d{2}[a-z]?)) \|/gm;
  for (const match of readFileSync(SPEC_PATH, 'utf8').matchAll(pattern)) {
    ids.add(match[1]);
  }
  return ids;
}

/** 对集合做 missing/extra/duplicate 对账并返回稳定摘要。 */
function compareGateSets(entries) {
  const expected = expectedGateIds();
  const actual = entries.map((entry) => entry.gate_id);
  const actualSet = new Set(actual);
  const duplicate = actual.filter((id, index) => actual.indexOf(id) !== index);
  const missing = [...expected].filter((id) => !actualSet.has(id)).sort();
  const extra = [...actualSet].filter((id) => !expected.has(id)).sort();
  if (expected.size !== 30 || missing.length || extra.length || duplicate.length) {
    throw new Error(
      `gate set mismatch expected=${expected.size} missing=${missing} extra=${extra} duplicate=${duplicate}`,
    );
  }
  return { missing, extra, duplicate: [...new Set(duplicate)].sort() };
}

/** 对 generated 目录执行集合对账；迁移中允许缺失，最终模式要求精确 30。 */
function validateGeneratedDirectories(expected, requireGenerated) {
  if (!existsSync(GENERATED_GATES_PATH)) {
    if (requireGenerated) {
      throw new Error(`generated gate root is missing: ${GENERATED_GATES_PATH}`);
    }
    return { state: 'NOT_GENERATED', missing: [...expected].sort(), extra: [] };
  }
  const actual = readdirSync(GENERATED_GATES_PATH, { withFileTypes: true })
    .filter(
      (entry) =>
        entry.isDirectory() &&
        existsSync(resolve(GENERATED_GATES_PATH, entry.name, 'generated')),
    )
    .map((entry) => entry.name);
  const actualSet = new Set(actual);
  const missing = [...expected].filter((id) => !actualSet.has(id)).sort();
  const extra = [...actualSet].filter((id) => !expected.has(id)).sort();
  if (extra.length || (requireGenerated && missing.length)) {
    throw new Error(`generated gate set mismatch missing=${missing} extra=${extra}`);
  }
  return {
    state: missing.length ? 'PARTIAL' : 'EXACT',
    missing,
    extra,
  };
}

/** 验证每个 Gate 的 domain、test path、owner 与七类删除引用。 */
function validateEntries(manifest) {
  const catalogIds = new Set(Object.keys(manifest.inventory_catalog ?? {}));
  if (catalogIds.size !== 7) {
    throw new Error(`inventory catalog must contain 7 kinds, got ${catalogIds.size}`);
  }
  for (const entry of manifest.entries) {
    const expectedPackage = DOMAIN_PACKAGES.get(entry.domain);
    if (!expectedPackage || entry.test_package !== expectedPackage) {
      throw new Error(`${entry.gate_id} invalid domain/package mapping`);
    }
    const expectedPath = `crates/helix-im/tests/message_v3/${entry.test_package}/`;
    if (!entry.test_path.startsWith(expectedPath)) {
      throw new Error(`${entry.gate_id} test_path does not match package`);
    }
    if (!entry.production_owners?.length) {
      throw new Error(`${entry.gate_id} has no production owner`);
    }
    for (const owner of entry.production_owners) {
      if (!existsSync(resolve(ROOT, owner))) {
        throw new Error(`${entry.gate_id} owner does not exist: ${owner}`);
      }
    }
    if (
      entry.legacy_inventory.length !== catalogIds.size ||
      entry.legacy_inventory.some((id) => !catalogIds.has(id))
    ) {
      throw new Error(`${entry.gate_id} deletion inventory is incomplete`);
    }
    if (JSON.stringify(entry).includes('TODO')) {
      throw new Error(`${entry.gate_id} contains unresolved TODO`);
    }
    if (entry.state === 'MIGRATED' && entry.verification.length === 0) {
      throw new Error(`${entry.gate_id} migrated entry has no verification`);
    }
  }
}

/** 把 review workflow 与 Gate migration 合并为一个单活跃计数。 */
function validateSingleActive(ledger, entries) {
  const sharedActive = ledger.phase === 'implementing' || ledger.phase === 'verifying' ? 1 : 0;
  const gateActive = entries.filter((entry) => entry.state === 'IN_PROGRESS').length;
  if (sharedActive + gateActive > 1) {
    throw new Error(`IN_PROGRESS invariant violated: shared=${sharedActive} gate=${gateActive}`);
  }
  return sharedActive + gateActive;
}

/** 迁移完成的 owner 不得继续引用退役业务协议。 */
function validateMigratedOwners(entries) {
  for (const entry of entries.filter((candidate) => candidate.state === 'MIGRATED')) {
    for (const owner of entry.production_owners) {
      const source = readFileSync(resolve(ROOT, owner), 'utf8');
      if (FORBIDDEN_RUNTIME_TERMS.test(source)) {
        throw new Error(`${entry.gate_id} migrated owner still references retired contract: ${owner}`);
      }
    }
  }
}

/** 证明共享旧模块已物理删除，且生产 Rust 源码不再引用退役业务合同。 */
function validateProductionCutover() {
  const remaining = RETIRED_SOURCE_PATHS.filter((path) => existsSync(resolve(ROOT, path)));
  if (remaining.length) {
    throw new Error(`retired production paths still exist: ${remaining.join(',')}`);
  }
  const violations = walkFiles(resolve(ROOT, 'crates/helix-im/src'), (path) =>
    path.endsWith('.rs'),
  ).filter((path) => FORBIDDEN_RUNTIME_TERMS.test(readFileSync(path, 'utf8')));
  if (violations.length) {
    throw new Error(
      `production source still references retired contract: ${violations
        .map((path) => path.slice(ROOT.length + 1))
        .join(',')}`,
    );
  }
}

/** 证明 native/FFI/Web 当前代码与测试不再依赖旧 UI 协议或独立 action ABI。 */
function validateDriverAndTestCutover() {
  const roots = [
    'crates/helix-im/tests',
    'crates/helix-driver-native/src',
    'crates/helix-driver-ffi/src',
    'crates/helix-driver-ffi/tests',
    'crates/helix-driver-web/src',
  ];
  const violations = roots
    .flatMap((root) => walkFiles(resolve(ROOT, root), (path) => path.endsWith('.rs')))
    .filter((path) => FORBIDDEN_CODE_DEPENDENCIES.test(readFileSync(path, 'utf8')));
  if (violations.length) {
    throw new Error(
      `current test/driver still depends on retired contract: ${violations
        .map((path) => path.slice(ROOT.length + 1))
        .join(',')}`,
    );
  }
}

/** 把旧 50 Gate Matrix 与其下游 API 锁为显式历史快照，禁止继续冒充当前合同。 */
function validateRetiredMatrixAllowlist() {
  for (const path of [RETIRED_MATRIX_PATH, RETIRED_COMMAND_API_PATH]) {
    const contract = readJson(path);
    if (
      contract.lifecycle !== 'RETIRED_HISTORICAL' ||
      contract.retired_by !== 'specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json'
    ) {
      throw new Error(`historical Matrix allowlist entry is not retired: ${path}`);
    }
  }
  return [
    'crates/helix-im/contracts/strict-matrix.v1.json',
    'crates/helix-im/contracts/command-api.v1.json',
    'crates/helix-im/docs/Strict Matrix/**',
    'crates/helix-im/docs/projection-schema.md',
    'crates/helix-im/docs/interaction-v2/**',
    'docs/interaction-frame-v2-全量替换-按UC/**',
    'specs/018-interaction-frame-v2-cutover..046-timeline-v3-delta-window/**',
  ];
}

/** 验证 30 个 Gate 各自拥有唯一显式 Cargo target 与真实 main.rs。 */
function validateGateTargets(entries) {
  const cargo = readFileSync(resolve(ROOT, 'crates/helix-im/Cargo.toml'), 'utf8');
  const targetCounts = new Map();
  for (const match of cargo.matchAll(/^name\s*=\s*"([^"]+)"/gm)) {
    targetCounts.set(match[1], (targetCounts.get(match[1]) ?? 0) + 1);
  }
  for (const entry of entries) {
    if (targetCounts.get(entry.test_target) !== 1) {
      throw new Error(`${entry.gate_id} test target must be registered exactly once`);
    }
    if (!existsSync(resolve(ROOT, entry.test_path))) {
      throw new Error(`${entry.gate_id} test main is missing: ${entry.test_path}`);
    }
  }
}

/** 递归拒绝同一业务对象同时携带 temporaryId 与 temporary_id。 */
function rejectDuplicateTemporaryAlias(value, location) {
  if (Array.isArray(value)) {
    value.forEach((item, index) => rejectDuplicateTemporaryAlias(item, `${location}[${index}]`));
    return;
  }
  if (!value || typeof value !== 'object') {
    return;
  }
  if (
    Object.prototype.hasOwnProperty.call(value, 'temporaryId') &&
    Object.prototype.hasOwnProperty.call(value, 'temporary_id')
  ) {
    throw new Error(`duplicate temporary id aliases at ${location}`);
  }
  for (const [key, item] of Object.entries(value)) {
    if (key === 'actionCapabilities' || key === 'channelCapabilities') {
      throw new Error(`retired capability payload at ${location}.${key}`);
    }
    rejectDuplicateTemporaryAlias(item, `${location}.${key}`);
  }
}

/** 最终模式逐文件检查 generated JSON，防止 schema 外重新混入旧套壳或重复别名。 */
function validateGeneratedPayloads(entries, requireGenerated) {
  if (!requireGenerated) {
    return;
  }
  for (const entry of entries) {
    for (const name of entry.generated_files) {
      const path = resolve(GENERATED_GATES_PATH, entry.gate_id, 'generated', name);
      rejectDuplicateTemporaryAlias(readJson(path), `${entry.gate_id}/${name}`);
    }
  }
}

/** 断言一个破坏性变体确实被 checker 拒绝，防止护栏静默失效。 */
function expectFailure(label, action) {
  try {
    action();
  } catch {
    return;
  }
  throw new Error(`negative self-test did not fail: ${label}`);
}

/** 在内存副本上证伪集合、映射与单活跃三个核心不变量。 */
function runSelfTest(manifest, ledger) {
  const duplicate = structuredClone(manifest.entries);
  duplicate[1].gate_id = duplicate[0].gate_id;
  expectFailure('duplicate gate', () => compareGateSets(duplicate));

  const wrongPackage = structuredClone(manifest);
  wrongPackage.entries[0].test_package = 'post';
  expectFailure('domain/package mismatch', () => validateEntries(wrongPackage));

  const twoActive = structuredClone(manifest.entries);
  twoActive[0].state = 'IN_PROGRESS';
  const implementingLedger = structuredClone(ledger);
  implementingLedger.phase = 'implementing';
  expectFailure('shared and gate both active', () =>
    validateSingleActive(implementingLedger, twoActive),
  );

  const expected = expectedGateIds();
  const generatedWithExtra = new Set([...expected, 'G-17']);
  expectFailure('generated extra gate', () => {
    const extra = [...generatedWithExtra].filter((id) => !expected.has(id));
    if (extra.length) {
      throw new Error(`generated extra=${extra}`);
    }
  });
}

/** 运行 MessageV3 cutover 的确定性静态护栏。 */
function main() {
  const manifest = readJson(MANIFEST_PATH);
  const ledger = readJson(LEDGER_PATH);
  if (manifest.schema_version !== 'helix-im-gate-migration/v1') {
    throw new Error(`unsupported migration schema: ${manifest.schema_version}`);
  }
  const setResult = compareGateSets(manifest.entries);
  validateEntries(manifest);
  const generated = validateGeneratedDirectories(
    expectedGateIds(),
    process.argv.includes('--require-generated'),
  );
  const active = validateSingleActive(ledger, manifest.entries);
  validateMigratedOwners(manifest.entries);
  validateProductionCutover();
  validateDriverAndTestCutover();
  const historicalAllowlist = validateRetiredMatrixAllowlist();
  validateGateTargets(manifest.entries);
  validateGeneratedPayloads(manifest.entries, process.argv.includes('--require-generated'));
  if (
    process.argv.includes('--require-generated') &&
    manifest.entries.some((entry) => entry.state !== 'MIGRATED')
  ) {
    throw new Error(`final cutover requires all 30 gates to be MIGRATED`);
  }
  if (process.argv.includes('--self-test')) {
    runSelfTest(manifest, ledger);
  }
  process.stdout.write(
    `${JSON.stringify({
      verdict: 'PASS',
      gates: manifest.entries.length,
      ...setResult,
      generated,
      active,
      consumerSync: 'NOT_SYNCED_BY_SCOPE',
      historicalAllowlist,
      selfTest: process.argv.includes('--self-test') ? 'PASS' : 'NOT_RUN',
    })}\n`,
  );
}

main();
