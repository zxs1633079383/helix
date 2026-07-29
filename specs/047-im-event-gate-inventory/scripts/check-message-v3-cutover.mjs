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
const FORBIDDEN_RUNTIME_TERMS =
  /\b(?:InteractionFrame|SurfaceMutation|ActionBinding|UiDirective|FrameApplied)\b/;
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
  expectFailure('shared and gate both active', () => validateSingleActive(ledger, twoActive));

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
      selfTest: process.argv.includes('--self-test') ? 'PASS' : 'NOT_RUN',
    })}\n`,
  );
}

main();
