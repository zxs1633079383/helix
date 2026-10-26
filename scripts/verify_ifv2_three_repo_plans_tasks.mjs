#!/usr/bin/env node

import { existsSync, readFileSync } from 'node:fs';
import { dirname, isAbsolute, join, resolve } from 'node:path';

const ROOT = new URL('..', import.meta.url).pathname.replace(/\/$/, '');
const MASTER = 'specs/020-ifv2-three-repo-uc-control';
const MANIFEST = `${MASTER}/gate-spec-manifest.json`;
const INDEX = `${MASTER}/plan-task-index.json`;
const REQUIRED = [
  'spec.md',
  'discovery.md',
  'plan.md',
  'research.md',
  'data-model.md',
  'quickstart.md',
  'verification.md',
  'tasks.md',
  'review-ledger.json',
  'contracts/gate-acceptance.md',
  'contracts/canonical-fixture.schema.json',
  'contracts/cross-repo-report.schema.json',
];
const MASTER_REQUIRED = [
  'spec.md',
  'discovery.md',
  'plan.md',
  'research.md',
  'data-model.md',
  'quickstart.md',
  'verification.md',
  'tasks.md',
  'review-ledger.json',
  'contracts/execution-order.md',
  'contracts/gate-status.schema.json',
  'plan-task-index.json',
  'plan-task-index.md',
];
const TASK_RE = /^- \[[ xX]\] (T\d{3})\b/gm;
const CONTRACT_RE = /^\|\s*(T\d{3})\s*\|/gm;
const LINK_RE = /\[[^\]]+\]\(([^)]+)\)/g;
const PLACEHOLDER_RE = /\[(?:FEATURE|DATE|NEEDS CLARIFICATION|Command and expected falsifiable result)/i;

// 读取 JSON 并把语法错误提升为校验失败。
function readJson(path) {
  return JSON.parse(readFileSync(join(ROOT, path), 'utf8'));
}

// 收集任务和 Loop contract 的 ID，要求顺序与数量一一对应。
function validateTasks(feature, expectedCount, errors) {
  const text = readFileSync(join(ROOT, feature, 'tasks.md'), 'utf8');
  const taskIds = [...text.matchAll(TASK_RE)].map((match) => match[1]);
  const contractIds = [...text.matchAll(CONTRACT_RE)].map((match) => match[1]);
  if (taskIds.length !== expectedCount) {
    errors.push(`${feature} task count expected=${expectedCount} actual=${taskIds.length}`);
  }
  if (taskIds.join(',') !== contractIds.join(',')) {
    errors.push(`${feature} task IDs and Loop Task Contracts differ`);
  }
  if (new Set(taskIds).size !== taskIds.length) {
    errors.push(`${feature} has duplicate task IDs`);
  }
}

// 验证 Feature 设计资产、R1-R3 ledger 和模板占位符清零。
function validateFeature(feature, required, expectedTasks, errors) {
  for (const path of required) {
    if (!existsSync(join(ROOT, feature, path))) errors.push(`${feature} missing ${path}`);
  }
  validateTasks(feature, expectedTasks, errors);
  const ledger = readJson(`${feature}/review-ledger.json`);
  for (const gate of ['r1_spec', 'r2_plan', 'r3_tasks']) {
    if (ledger.gates?.[gate]?.status !== 'pass') errors.push(`${feature} ${gate} is not pass`);
  }
  const controlledStates = new Set([
    'ready:admit_implementation',
    'implementing:implement_tasks',
  ]);
  if (!controlledStates.has(`${ledger.phase}:${ledger.next_action}`)) {
    errors.push(`${feature} is not ready for controlled implementation`);
  }
  const combined = required
    .filter((path) => path.endsWith('.md'))
    .map((path) => readFileSync(join(ROOT, feature, path), 'utf8'))
    .join('\n');
  if (PLACEHOLDER_RE.test(combined)) errors.push(`${feature} has unresolved placeholder`);
}

// 验证新增索引范围内的相对 Markdown 链接，绝对路径和 Web URL 不参与。
function validateLinks(paths, errors) {
  for (const path of paths) {
    const absolute = join(ROOT, path);
    const text = readFileSync(absolute, 'utf8');
    for (const match of text.matchAll(LINK_RE)) {
      const target = match[1].split('#', 1)[0];
      if (!target || ['http://', 'https://', 'mailto:'].some((prefix) => target.startsWith(prefix)) || isAbsolute(target)) continue;
      if (!existsSync(resolve(dirname(absolute), target))) errors.push(`${path} broken link ${target}`);
    }
  }
}

// 执行 26 Feature 的 Plan/Task、索引、JSON 和 ledger 总校验。
function main() {
  const errors = [];
  const manifest = readJson(MANIFEST);
  const index = readJson(INDEX);
  const records = manifest.scope.children;
  if (manifest.scope.activeCount !== 26 || records.length !== 25 || manifest.scope.gap.length !== 20) {
    errors.push('manifest count mismatch');
  }
  if (index.gates.length !== 25) errors.push('plan-task index child count mismatch');
  if (records.map((record) => record.gateId).join(',') !== index.gates.map((record) => record.gateId).join(',')) {
    errors.push('plan-task index order differs from gate manifest');
  }

  validateFeature(MASTER, MASTER_REQUIRED, 32, errors);
  for (const record of records) validateFeature(record.featureDirectory, REQUIRED, 38, errors);

  const linkPaths = [
    `${MASTER}/plan.md`,
    `${MASTER}/tasks.md`,
    `${MASTER}/plan-task-index.md`,
    'docs/interaction-frame-v2-全量替换-按UC/08-26Gate-Plan-Task实施索引.md',
    ...records.flatMap((record) => [
      `${record.featureDirectory}/plan.md`,
      `${record.featureDirectory}/tasks.md`,
      `${record.featureDirectory}/quickstart.md`,
      `${record.featureDirectory}/contracts/gate-acceptance.md`,
    ]),
  ];
  validateLinks(linkPaths, errors);

  for (const record of records) {
    readJson(`${record.featureDirectory}/contracts/canonical-fixture.schema.json`);
    readJson(`${record.featureDirectory}/contracts/cross-repo-report.schema.json`);
  }
  readJson(`${MASTER}/contracts/gate-status.schema.json`);

  if (errors.length) {
    for (const error of errors) process.stderr.write(`FAIL: ${error}\n`);
    process.exit(1);
  }
  process.stdout.write('PASS: 26 Feature R1-R3；25 x 38 子任务 + 32 总控任务；Plan/Task/contract/index/link/JSON 全部有效\n');
}

main();
