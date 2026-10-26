import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { test } from 'node:test';

const TEST_DIR = path.dirname(fileURLToPath(import.meta.url));
const HELIX_ROOT = path.resolve(TEST_DIR, '../..');
const CLIENT_ROOT = path.resolve(HELIX_ROOT, '../../../angular/visual/cses-client');
const GO_ROOT = path.resolve(HELIX_ROOT, '../../../golang/cses-im-server-MessageV3-helix');
const MATRIX_PATH = path.join(HELIX_ROOT, 'scripts/messagev3/phase1-real-chain/matrix.json');

/** 读取 JSON 真源，契约测试失败时保留文件路径上下文。 */
async function readJson(filePath) {
    return JSON.parse(await readFile(filePath, 'utf8'));
}

/** 把 Domain→run→Gate 展平，供分母、重复与漏项断言复用。 */
function matrixGates(matrix) {
    return matrix.domains.flatMap(domain => domain.runs.flatMap(run => run.gates));
}

/** 在读取 Angular 权威矩阵时隔离外部 CSES_WDIO_*，保证契约测试无需用户环境变量。 */
async function loadClientDomainMatrix(filePath) {
    const saved = Object.fromEntries(
        Object.entries(process.env).filter(([name]) => name.startsWith('CSES_WDIO_'))
    );
    for (const name of Object.keys(saved)) delete process.env[name];
    try {
        return await import(pathToFileURL(filePath).href);
    } finally {
        Object.assign(process.env, saved);
    }
}

test('Phase1 Domain→Gate 矩阵与 Helix/Angular 两份 manifest 完全一致', async () => {
    const matrix = await readJson(MATRIX_PATH);
    const registry = await readJson(path.join(HELIX_ROOT, matrix.gateRegistry));
    const clientManifest = await readJson(path.join(CLIENT_ROOT, matrix.clientManifest));
    const clientMatrix = await loadClientDomainMatrix(
        path.join(CLIENT_ROOT, matrix.clientDomainMatrix)
    );
    const matrixIds = matrixGates(matrix);
    const registryIds = registry.entries.map(entry => entry.gate_id);
    const clientDomainIds = clientMatrix.PHASE1_DOMAINS.flatMap(domain => domain.gates);
    assert.equal(registry.denominator.total, 33);
    assert.equal(clientManifest.expectedGateCount, 33);
    assert.deepEqual([...new Set(matrixIds)].sort(), [...new Set(registryIds)].sort());
    assert.deepEqual([...new Set(matrixIds)].sort(), [...new Set(clientManifest.gates)].sort());
    assert.deepEqual([...new Set(matrixIds)].sort(), [...new Set(clientDomainIds)].sort());
    assert.equal(matrixIds.length, 33);
    assert.equal(new Set(matrixIds).size, 33);
});

test('每个 Gate 都指向 Angular 真实 WDIO 类而不是猜测的占位文件', async () => {
    const matrix = await readJson(MATRIX_PATH);
    const runLaneSource = await readFile(
        path.join(CLIENT_ROOT, 'scripts/message-wdio/phase1/run-lane.mjs'),
        'utf8'
    );
    for (const domain of matrix.domains) {
        assert.match(domain.id, /^[a-z0-9-]+$/);
        assert.ok(domain.runs.length > 0, `${domain.id} must have at least one real run`);
        for (const run of domain.runs) {
            assert.match(run.suite, /^(single|dual|multi)-real-[a-z-]+$/);
            assert.match(runLaneSource, new RegExp(`['"]${run.suite}['"]`));
            const specPath = path.join(CLIENT_ROOT, run.spec);
            const source = await readFile(specPath, 'utf8');
            for (const gateId of run.gates) {
                assert.match(source, new RegExp(gateId), `${gateId} missing from ${run.spec}`);
            }
        }
    }
});

test('三仓入口与 Helix Rust Gate target、Go HTTP/WS 路由都存在', async () => {
    const matrix = await readJson(MATRIX_PATH);
    const registry = await readJson(path.join(HELIX_ROOT, matrix.gateRegistry));
    const router = await readFile(path.join(GO_ROOT, 'internal/api/router.go'), 'utf8');
    const websocketAuth = await readFile(path.join(GO_ROOT, 'internal/api/websocket_auth.go'), 'utf8');
    assert.match(router, /\/api\/cses/);
    assert.match(router, /\/api\/v4\/websocket/);
    assert.match(websocketAuth, /cookieId/);
    for (const entry of registry.entries) {
        const target = path.join(HELIX_ROOT, entry.test_path);
        assert.equal(await readFile(target, 'utf8').then(() => true).catch(() => false), true, entry.gate_id);
    }
});

test('三仓 runner 的默认目标固定为当前 Go localhost:8066，禁止把 health 当业务绿', async () => {
    const runner = await readFile(path.join(HELIX_ROOT, 'scripts/messagev3/phase1-real-chain.mjs'), 'utf8');
    assert.match(runner, /127\.0\.0\.1:8066/);
    assert.match(runner, /PARTIAL_STORAGE_READBACK/);
    assert.match(runner, /BLOCKED_PRECONDITION/);
    assert.match(runner, /commandEvidence/);
    assert.match(runner, /wsEvidence/);
    assert.match(runner, /projectionEvidence/);
});
