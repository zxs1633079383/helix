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
import { spawnSync } from 'node:child_process';

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const FEATURE_DIR = resolve(SCRIPT_DIR, '..');
const ROOT = resolve(FEATURE_DIR, '..', '..');
const OUTPUT_DIR = join(FEATURE_DIR, '.v1');
const MIGRATION_PATH = join(FEATURE_DIR, 'contracts', 'gate-migration.v1.json');
const REQUIRED_FILES = [
  'inbound.json',
  'outbound.sender.json',
  'outbound.receiver.json',
  'provenance.json',
];

/** 读取并解析 UTF-8 JSON，所有业务内容必须已由 Rust producer 写入。 */
function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'));
}

/** 对文件原始 bytes 计算小写 SHA-256。 */
function fileHash(path) {
  return createHash('sha256').update(readFileSync(path)).digest('hex');
}

/** 运行一个 Rust test target，并把唯一输出目录通过环境变量交给测试。 */
function runProducer(entry) {
  const output = join(OUTPUT_DIR, 'gates', entry.gate_id, 'generated');
  mkdirSync(output, { recursive: true });
  const result = spawnSync(
    'cargo',
    ['test', '-p', 'helix-im', '--test', entry.test_target, '--', '--nocapture'],
    {
      cwd: ROOT,
      env: { ...process.env, HELIX_GATE_ARTIFACT_DIR: output },
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    throw new Error(`${entry.gate_id} producer failed with status ${result.status}`);
  }
}

/** 让 Rust schema contract 一次性校验全部 Gate 的真实 generated 树。 */
function runSchemaValidation() {
  const result = spawnSync(
    'cargo',
    ['test', '-p', 'helix-im', '--test', 'message_v3_artifact_schema', '--', '--nocapture'],
    {
      cwd: ROOT,
      env: {
        ...process.env,
        HELIX_GATE_ARTIFACT_ROOT: join(OUTPUT_DIR, 'gates'),
      },
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  );
  if (result.status !== 0) {
    process.stderr.write(result.stdout);
    process.stderr.write(result.stderr);
    throw new Error(`generated artifact schema validation failed with status ${result.status}`);
  }
}

/** 校验 Rust producer 已写齐四份文件，并返回 byte-exact hash 索引。 */
function inspectGenerated(entry) {
  const output = join(OUTPUT_DIR, 'gates', entry.gate_id, 'generated');
  const actual = existsSync(output)
    ? readdirSync(output).filter((name) => statSync(join(output, name)).isFile()).sort()
    : [];
  const missing = REQUIRED_FILES.filter((name) => !actual.includes(name));
  const extra = actual.filter((name) => !REQUIRED_FILES.includes(name));
  if (missing.length || extra.length) {
    throw new Error(
      `${entry.gate_id} generated set drift: missing=${missing.join(',')} extra=${extra.join(',')}`,
    );
  }
  return Object.fromEntries(
    REQUIRED_FILES.map((name) => [name, fileHash(join(output, name))]),
  );
}

/** 对所有生成文件计算排序后的全树 hash，证明重复生成稳定性。 */
function treeHash(gates) {
  const hash = createHash('sha256');
  for (const gate of [...gates].sort((left, right) => left.gate_id.localeCompare(right.gate_id))) {
    for (const name of REQUIRED_FILES) {
      const path = join(OUTPUT_DIR, 'gates', gate.gate_id, 'generated', name);
      hash.update(`${relative(OUTPUT_DIR, path)}\0`);
      hash.update(readFileSync(path));
    }
  }
  return hash.digest('hex');
}

/** 构造只含索引、hash 和同步边界的顶层 manifest，不重建任何业务 payload。 */
function buildManifest(entries, artifacts) {
  return {
    schema_version: 'helix-im-gate-evidence-index/v1',
    gate_ids: entries.map((entry) => entry.gate_id),
    gate_count: entries.length,
    generated_files_per_gate: REQUIRED_FILES.length,
    artifacts,
    tree_sha256: treeHash(entries),
    consumer_sync: 'NOT_SYNCED_BY_SCOPE',
  };
}

/** 执行精确 30 Gate 的 Rust producer，然后校验并索引唯一生成物。 */
function main() {
  const migration = readJson(MIGRATION_PATH);
  const entries = migration.entries;
  if (!Array.isArray(entries) || entries.length !== 30) {
    throw new Error(`migration manifest must contain exactly 30 gates`);
  }
  const ids = new Set(entries.map((entry) => entry.gate_id));
  if (ids.size !== entries.length) {
    throw new Error(`migration manifest contains duplicate gate ids`);
  }

  for (const entry of entries) {
    runProducer(entry);
  }
  const artifacts = Object.fromEntries(
    entries.map((entry) => [entry.gate_id, inspectGenerated(entry)]),
  );
  runSchemaValidation();
  const manifest = buildManifest(entries, artifacts);
  mkdirSync(OUTPUT_DIR, { recursive: true });
  writeFileSync(join(OUTPUT_DIR, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  process.stdout.write(
    `${JSON.stringify({
      verdict: 'PASS',
      gates: entries.length,
      files: entries.length * REQUIRED_FILES.length,
      tree_sha256: manifest.tree_sha256,
      consumer_sync: manifest.consumer_sync,
    })}\n`,
  );
}

main();
