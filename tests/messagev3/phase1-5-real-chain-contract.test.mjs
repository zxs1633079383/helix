import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const RUNNER = path.join(ROOT, 'scripts/messagev3/phase1-5-real-chain.mjs');

/** 运行只读列表模式并返回结构化行。 */
function list(...args) {
    const result = spawnSync(process.execPath, [RUNNER, '--list', ...args], {
        cwd: ROOT,
        encoding: 'utf8'
    });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.trim().split(/\r?\n/).filter(Boolean).map(line => line.split('\t'));
}

/** 断言目录发现结果与当前 Phase1-5 机器分母完全一致。 */
test('全量分母固定为 Phase1-5 共 92 Gate', () => {
    const rows = list();
    assert.equal(rows.length, 92);
    assert.deepEqual(
        [1, 2, 3, 4, 5].map(phase => rows.filter(row => row[0] === `Phase${phase}`).length),
        [33, 20, 15, 12, 12]
    );
    assert.equal(rows.some(row => row[1] === 'todo'), false);
});

/** 断言 runner 的三种选择器不会扩大或缩小精确目标。 */
test('支持按 Phase、Domain 与 Gate 精确筛选', () => {
    assert.equal(list('--phase', '5').length, 12);
    assert.equal(list('--domain', 'channel_member').length, 5);
    const gate = list('--gate', 'MV3-G01A');
    assert.equal(gate.length, 1);
    assert.deepEqual(gate[0].slice(0, 3), ['Phase1', 'post', 'MV3-G01A']);
});
