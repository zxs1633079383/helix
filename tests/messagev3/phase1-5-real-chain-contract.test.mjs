import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
    DEFAULT_MESSAGEV3_ENVIRONMENT,
    MESSAGEV3_ENVIRONMENTS,
    resolveMessagev3Environment
} from '../../scripts/messagev3/real-chain-environment.mjs';

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

/** 断言目录发现结果与当前 Phase1-5 可执行分母完全一致。 */
test('全量分母固定为 Phase1-5 共 83 个可执行 Gate', () => {
    const rows = list();
    assert.equal(rows.length, 83);
    assert.deepEqual(
        [1, 2, 3, 4, 5].map(phase => rows.filter(row => row[0] === `Phase${phase}`).length),
        [32, 16, 15, 12, 8]
    );
    assert.equal(rows.some(row => row[1] === 'todo'), false);
    for (const excluded of [
        ['Phase1', 'post', 'MV3-G05L'],
        ['Phase2', 'read', 'MV3-G06C'],
        ['Phase2', 'read', 'MV3-G06E'],
        ['Phase2', 'timeline', 'MV3-G11F'],
        ['Phase2', 'sync', 'MV3-G13E'],
        ['Phase5', 'acceptance', 'CHAIN-ACC-G01'],
        ['Phase5', 'category_chain', 'CHAIN-CAT-G01'],
        ['Phase5', 'category_chain', 'CHAIN-CAT-G02'],
        ['Phase5', 'category_chain', 'CHAIN-CAT-G03']
    ]) {
        assert.equal(rows.some(row => row.slice(0, 3).every((value, index) => value === excluded[index])), false);
    }
});

/** 断言 runner 的三种选择器不会扩大或缩小精确目标。 */
test('支持按 Phase、Domain 与 Gate 精确筛选', () => {
    assert.equal(list('--phase', '5').length, 8);
    assert.equal(list('--domain', 'channel_member').length, 5);
    const gate = list('--gate', 'MV3-G01A');
    assert.equal(gate.length, 1);
    assert.deepEqual(gate[0].slice(0, 3), ['Phase1', 'post', 'MV3-G01A']);
});

/** 断言 pre/local profile 与逐端点覆盖都由同一配置解析。 */
test('real-chain 环境默认 pre 且支持 local 与端点覆盖', () => {
    const keys = [
        'MESSAGEV3_ENV',
        'MESSAGEV3_GO_HTTP_BASE',
        'MESSAGEV3_GO_WS_URL',
        'MESSAGEV3_GO_HEALTH_URL',
        'MESSAGEV3_JAVA_API_BASE',
        'HELIX_MAIN_API_BASE'
    ];
    const previous = new Map(keys.map(key => [key, process.env[key]]));
    try {
        for (const key of keys) delete process.env[key];
        assert.equal(DEFAULT_MESSAGEV3_ENVIRONMENT, 'pre');
        assert.deepEqual(resolveMessagev3Environment(), {
            name: 'pre',
            goHttpBase: MESSAGEV3_ENVIRONMENTS.pre.goHttpBase,
            goWsUrl: MESSAGEV3_ENVIRONMENTS.pre.goWsUrl,
            goHealthUrl: `${MESSAGEV3_ENVIRONMENTS.pre.goHttpBase}/health`,
            javaApiBase: MESSAGEV3_ENVIRONMENTS.pre.javaApiBase
        });
        const local = resolveMessagev3Environment('local');
        assert.equal(local.name, 'local');
        assert.equal(local.goHttpBase, MESSAGEV3_ENVIRONMENTS.local.goHttpBase);
        assert.throws(() => resolveMessagev3Environment('dev'), /不支持的 MessageV3 环境/);
        process.env.MESSAGEV3_GO_HTTP_BASE = 'http://dev.example/api/cses';
        process.env.MESSAGEV3_GO_WS_URL = 'ws://dev.example/api/v4/websocket';
        process.env.MESSAGEV3_JAVA_API_BASE = 'https://dev.example';
        const overridden = resolveMessagev3Environment('local');
        assert.deepEqual(overridden, {
            name: 'local',
            goHttpBase: 'http://dev.example/api/cses',
            goWsUrl: 'ws://dev.example/api/v4/websocket',
            goHealthUrl: 'http://dev.example/api/cses/health',
            javaApiBase: 'https://dev.example'
        });
        process.env.MESSAGEV3_ENV = 'dev';
        const futureProfile = resolveMessagev3Environment();
        assert.equal(futureProfile.name, 'dev');
        assert.equal(futureProfile.javaApiBase, 'https://dev.example');
    } finally {
        for (const [key, value] of previous) {
            if (value === undefined) delete process.env[key];
            else process.env[key] = value;
        }
    }
});
