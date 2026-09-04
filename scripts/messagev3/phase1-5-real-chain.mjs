#!/usr/bin/env node

import { spawn } from 'node:child_process';
import { access, mkdir, readFile, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
    resolveMessagev3CredentialEnvironment,
    resolveMessagev3Environment
} from './real-chain-environment.mjs';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const HELIX_ROOT = path.resolve(SCRIPT_DIR, '../..');
const TEST_ROOT = path.join(HELIX_ROOT, 'crates/helix-driver-native/tests/messageV3-domain');
const TEST_ENTRY = path.join(HELIX_ROOT, 'crates/helix-driver-native/tests/message_v3_domain.rs');
const DEFAULT_ARTIFACT_ROOT = path.join(HELIX_ROOT, 'artifacts/messagev3-phase1-5-real-chain');
const EXPECTED_PHASE_COUNTS = new Map([[1, 32], [2, 16], [3, 15], [4, 12], [5, 8]]);
const EXPECTED_GATE_COUNT = 83;
const EXCLUDED_GATE_KEYS = new Set([
    'Phase1/post/MV3-G05L',
    'Phase2/read/MV3-G06C',
    'Phase2/read/MV3-G06E',
    'Phase2/timeline/MV3-G11F',
    'Phase2/sync/MV3-G13E',
    'Phase5/acceptance/CHAIN-ACC-G01',
    'Phase5/category_chain/CHAIN-CAT-G01',
    'Phase5/category_chain/CHAIN-CAT-G02',
    'Phase5/category_chain/CHAIN-CAT-G03'
]);
// 目录过滤之外还要传给 libtest，确保 --all 不执行被排除 Gate 的副作用。
const EXCLUDED_TEST_FILTERS = [
    'phase1_post_g05l',
    'phase2_read_g06c',
    'phase2_read_g06e',
    'phase2_timeline_g11f',
    'phase2_sync_g13e',
    'phase5_chain_acceptance_acc_g01',
    'phase5_chain_category_cat_g01',
    'phase5_chain_category_cat_g02',
    'phase5_chain_category_cat_g03'
];

/** 解析环境与 Phase/Domain/Gate 选择器，并保持默认全量 83 个可执行 Gate。 */
function parseArgs(argv) {
    const options = {
        phases: [],
        domains: [],
        gates: [],
        environment: undefined,
        list: false,
        dryRun: false,
        artifactRoot: DEFAULT_ARTIFACT_ROOT
    };
    for (let index = 0; index < argv.length; index += 1) {
        const argument = argv[index];
        if (argument === '--all') continue;
        if (argument === '--list') {
            options.list = true;
            continue;
        }
        if (argument === '--dry-run') {
            options.dryRun = true;
            continue;
        }
        const [name, inlineValue] = argument.split('=', 2);
        const value = inlineValue ?? argv[++index];
        if (!value) throw new Error(`缺少 ${name} 的值`);
        if (name === '--phase') options.phases.push(normalizePhase(value));
        else if (name === '--domain') options.domains.push(normalizeDomain(value));
        else if (name === '--gate') options.gates.push(value.toUpperCase());
        else if (name === '--env' || name === '--environment') options.environment = value;
        else if (name === '--artifact-root') options.artifactRoot = path.resolve(value);
        else throw new Error(`不支持的参数: ${argument}`);
    }
    return options;
}

/** 将 Phase1/1 两种输入收敛为 1..5。 */
function normalizePhase(value) {
    const phase = Number(String(value).replace(/^phase/i, ''));
    if (!Number.isInteger(phase) || phase < 1 || phase > 5) {
        throw new Error(`无效 Phase: ${value}`);
    }
    return phase;
}

/** 将目录名与 CLI 输入统一成 docs/mmd 的 snake_case Domain。 */
function normalizeDomain(value) {
    return String(value)
        .replace(/([a-z0-9])([A-Z])/g, '$1_$2')
        .replace(/[\s/-]+/g, '_')
        .toLowerCase();
}

/** 递归发现测试类文件；仅接受 Phase→Domain→Gate/main.rs 叶子。 */
async function discoverMainFiles(directory) {
    const entries = await readdir(directory, { withFileTypes: true });
    const files = [];
    for (const entry of entries) {
        const candidate = path.join(directory, entry.name);
        if (entry.isDirectory()) files.push(...await discoverMainFiles(candidate));
        else if (entry.isFile() && entry.name === 'main.rs') files.push(candidate);
    }
    return files;
}

/** 从集成测试入口建立 path→module 映射，保证 Cargo filter 精确命中一个类。 */
async function loadModuleRegistry() {
    const source = await readFile(TEST_ENTRY, 'utf8');
    const registry = new Map();
    const matcher = /#\[path = "([^"]+)"\]\s*mod\s+([a-zA-Z0-9_]+);/g;
    for (const match of source.matchAll(matcher)) {
        registry.set(path.resolve(path.dirname(TEST_ENTRY), match[1]), match[2]);
    }
    return registry;
}

/** 把物理目录映射为可执行 Gate 的 canonical Phase/Domain/Gate 身份。 */
async function describeTest(file, registry) {
    const relative = path.relative(TEST_ROOT, file).split(path.sep);
    const phase = normalizePhase(relative[0]);
    if (phase === 4 && relative[1] === 'Todo') return null;
    const gate = phase === 5 ? relative.at(-2) : `MV3-${relative.at(-2)}`;
    const phase5Domains = {
        Common: 'chain',
        Text: 'text_chain',
        Category: 'category_chain',
        Acceptance: 'acceptance'
    };
    const domain = phase === 5 ? phase5Domains[relative[2]] : normalizeDomain(relative[1]);
    if (!domain) throw new Error(`未知 Phase5 Domain: ${relative.slice(0, -2).join('/')}`);
    const gateName = gate.toUpperCase();
    const key = `Phase${phase}/${domain}/${gateName}`;
    if (EXCLUDED_GATE_KEYS.has(key)) return null;
    const source = await readFile(file, 'utf8');
    const functionName = source.match(/(?:async\s+)?fn\s+([a-zA-Z0-9_]+_real_chain)\s*\(/)?.[1];
    const moduleName = registry.get(file);
    if (!functionName || !moduleName) {
        throw new Error(`测试类未注册或缺少 *_real_chain: ${path.relative(HELIX_ROOT, file)}`);
    }
    return {
        phase,
        phaseName: `Phase${phase}`,
        domain,
        gate: gateName,
        file: path.relative(HELIX_ROOT, file),
        filter: `${moduleName}::${functionName}`
    };
}

/** 校验真实目录分母，拒绝 Todo 或漏注册被静默纳入“全量”。 */
function validateDenominator(tests) {
    const seen = new Set();
    for (const test of tests) {
        const key = `${test.phaseName}/${test.domain}/${test.gate}`;
        if (seen.has(key)) throw new Error(`重复 Gate: ${key}`);
        seen.add(key);
    }
    for (const [phase, expected] of EXPECTED_PHASE_COUNTS) {
        const actual = tests.filter(test => test.phase === phase).length;
        if (actual !== expected) throw new Error(`Phase${phase} 分母漂移: expected=${expected} actual=${actual}`);
    }
    if (tests.length !== EXPECTED_GATE_COUNT) {
        throw new Error(`总分母漂移: expected=${EXPECTED_GATE_COUNT} actual=${tests.length}`);
    }
}

/** 只验证 Go 前置在线；health 不会进入任何 Gate PASS 判断。 */
async function assertGoReady(healthUrl) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5_000);
    try {
        const response = await fetch(healthUrl, { signal: controller.signal });
        const body = await response.json().catch(() => ({}));
        if (!response.ok || body?.status !== 'OK') {
            throw new Error(`Go health 未就绪 (${healthUrl}): HTTP ${response.status} status=${body?.status ?? 'UNKNOWN'}`);
        }
    } finally {
        clearTimeout(timer);
    }
}

/** 执行一次 Cargo 测试批次，并将完整输出落到本轮 run-owned artifact。 */
function cargoTimeoutMs() {
    const configured = Number.parseInt(process.env.MESSAGEV3_CARGO_TIMEOUT_MS ?? '', 10);
    return Number.isInteger(configured) && configured >= 30_000
        ? configured
        : 180_000;
}

/**
 * 执行一次 Cargo 测试批次，并把超时收敛为可审计的 TEST_FAILED，避免 CI 永久等待。
 *
 * 超时不能升级为 PASS：只负责终止当前 Cargo 子进程、保留已收集日志并让调用方继续写
 * summary。默认三分钟，CI/本地可用 MESSAGEV3_CARGO_TIMEOUT_MS 调大，但不能关闭 watchdog。
 */
async function runCargo(filter, exact, environment, logPath) {
    await mkdir(path.dirname(logPath), { recursive: true });
    const startedAt = Date.now();
    return new Promise((resolve, reject) => {
        const args = ['test', '-p', 'helix-driver-native', '--test', 'message_v3_domain'];
        if (filter) args.push(filter);
        args.push('--', '--nocapture', '--test-threads=1');
        if (exact) args.push('--exact');
        // Phase4/Todo is deliberately outside the executable Gate denominator and can hang on obsolete flows.
        args.push('--skip', 'phase4_todo');
        for (const filter of EXCLUDED_TEST_FILTERS) args.push('--skip', filter);
        const child = spawn('cargo', args, {
            cwd: HELIX_ROOT,
            env: environment,
            stdio: ['ignore', 'pipe', 'pipe']
        });
        let output = '';
        let settled = false;
        let timedOut = false;
        let forceKillTimer;
        let timeoutTimer;
        const append = chunk => { output += chunk.toString(); };
        const finish = async code => {
            if (settled) return;
            settled = true;
            clearTimeout(timeoutTimer);
            clearTimeout(forceKillTimer);
            await writeFile(logPath, output, 'utf8');
            resolve({
                code: timedOut ? 124 : (code ?? 1),
                output,
                durationMs: Date.now() - startedAt,
                timedOut,
                timeoutMs: cargoTimeoutMs()
            });
        };
        child.stdout.on('data', append);
        child.stderr.on('data', append);
        child.once('error', error => {
            if (settled) return;
            clearTimeout(timeoutTimer);
            clearTimeout(forceKillTimer);
            settled = true;
            reject(error);
        });
        child.once('close', code => { void finish(code); });
        timeoutTimer = setTimeout(() => {
            if (settled) return;
            timedOut = true;
            output += `\nMESSAGEV3_CARGO_TIMEOUT_MS=${cargoTimeoutMs()} exceeded; terminating cargo.\n`;
            child.kill('SIGTERM');
            // cargo may leave the libtest child alive after SIGTERM; force the bounded fallback.
            forceKillTimer = setTimeout(() => {
                if (!settled) child.kill('SIGKILL');
            }, 5_000);
        }, cargoTimeoutMs());
    });
}

/** 从 libtest 输出读取一个精确测试函数的结果，避免全量批次一败全体失真。 */
function testExitCode(output, filter) {
    const escaped = filter.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    if (new RegExp(`test ${escaped} \\.\\.\\. ok`).test(output)) return 0;
    if (new RegExp(`test ${escaped} \\.\\.\\. FAILED`).test(output)) return 101;
    return 102;
}

/** 判断路径存在性，不把缺少 evidence 转换成无关堆栈。 */
async function exists(file) {
    return access(file).then(() => true).catch(() => false);
}

/** 对单 Gate 证据执行 fail-closed 验证，Cargo 绿本身不构成 PASS。 */
async function validateEvidence(file, test, suiteRunId) {
    if (!await exists(file)) return { status: 'NOT_VERIFIED', reason: '缺少同轮 evidence.json' };
    const evidence = JSON.parse(await readFile(file, 'utf8'));
    const identityMatches = evidence.suite_run_id === suiteRunId
        && evidence.phase === test.phaseName
        && normalizeDomain(evidence.domain) === test.domain
        && String(evidence.gate).toUpperCase() === test.gate;
    if (!identityMatches) return { status: 'NOT_VERIFIED', reason: '证据身份与当前 Gate 不一致' };
    if (evidence.status !== 'PASS') return { status: evidence.status ?? 'NOT_VERIFIED', reason: evidence.details?.reason ?? '非 PASS 证据' };
    const details = evidence.details ?? {};
    const required = ['http_business', 'projection', 'cleanup'];
    const missing = required.filter(field => details[field] === undefined || details[field] === null);
    for (const [flag, field] of [
        ['ws_required', 'ws_action'],
        ['sqlite_required', 'sqlite_readback'],
        ['event_sink_required', 'event_sink'],
        ['persist_required', 'persist_after']
    ]) {
        if (details[flag] === true && (details[field] === undefined || details[field] === null)) missing.push(field);
    }
    if (evidence.corr_key === 'unbound' || missing.length > 0) {
        return { status: 'NOT_VERIFIED', reason: `PASS 证据不可关联或缺字段: ${missing.join(',')}` };
    }
    return { status: 'PASS', reason: null };
}

/** 生成不含凭据的稳定 suite id，串起 runner、Cargo 与 Rust evidence。 */
function suiteId() {
    return `messagev3-${Date.now()}-${process.pid}`;
}

/** 执行筛选后的 Gate，并汇总真实业务状态。 */
async function main() {
    const options = parseArgs(process.argv.slice(2));
    const runtimeEnvironment = resolveMessagev3Environment(options.environment);
    const registry = await loadModuleRegistry();
    const files = await discoverMainFiles(TEST_ROOT);
    const tests = (await Promise.all(files.map(file => describeTest(file, registry))))
        .filter(Boolean)
        .sort((left, right) => left.phase - right.phase || left.domain.localeCompare(right.domain) || left.gate.localeCompare(right.gate));
    validateDenominator(tests);
    const selected = tests.filter(test =>
        (!options.phases.length || options.phases.includes(test.phase))
        && (!options.domains.length || options.domains.includes(test.domain))
        && (!options.gates.length || options.gates.includes(test.gate))
    );
    if (!selected.length) throw new Error('筛选结果为空');
    if (options.list || options.dryRun) {
        for (const test of selected) console.log(`${test.phaseName}\t${test.domain}\t${test.gate}\t${test.filter}`);
        if (options.list || options.dryRun) return;
    }
    await assertGoReady(runtimeEnvironment.goHealthUrl);
    const suiteRunId = suiteId();
    const suiteRoot = path.join(options.artifactRoot, suiteRunId);
    await mkdir(suiteRoot, { recursive: true });
    const suiteStartedAt = Date.now();
    const phasePoolRequested = !['0', 'false', 'no'].includes(
        String(process.env.MESSAGEV3_PHASE_POOL ?? '1').trim().toLowerCase()
    );
    const environment = {
        ...process.env,
        ...resolveMessagev3CredentialEnvironment(runtimeEnvironment.name),
        MESSAGEV3_SUITE_RUN_ID: suiteRunId,
        MESSAGEV3_GATE_RUN_ID: suiteRunId,
        HELIX_MESSAGEV3_ARTIFACT_ROOT: options.artifactRoot,
        MESSAGEV3_ENV: runtimeEnvironment.name,
        MESSAGEV3_GO_HTTP_BASE: runtimeEnvironment.goHttpBase,
        MESSAGEV3_GO_WS_URL: runtimeEnvironment.goWsUrl,
        MESSAGEV3_GO_HEALTH_URL: runtimeEnvironment.goHealthUrl,
        MESSAGEV3_JAVA_API_BASE: runtimeEnvironment.javaApiBase,
        HELIX_MAIN_API_BASE: runtimeEnvironment.javaApiBase,
        MESSAGEV3_PHASE_POOL: phasePoolRequested ? '1' : '0'
    };
    const executions = new Map();
    const selectedByPhase = new Map();
    for (const test of selected) {
        const phaseTests = selectedByPhase.get(test.phase) ?? [];
        phaseTests.push(test);
        selectedByPhase.set(test.phase, phaseTests);
    }
    // --all 以及完整 --phase 批次各启动一次 Cargo；Rust cleanup test 会在同一进程末回收池。
    const batchPhases = [...selectedByPhase.keys()]
        .filter(phase => selectedByPhase.get(phase).length === EXPECTED_PHASE_COUNTS.get(phase))
        .sort((left, right) => left - right);
    const batchPhaseSet = new Set(batchPhases);
    const executionRecords = [];
    for (const phase of batchPhases) {
        const execution = await runCargo(
            `phase${phase}_`,
            false,
            { ...environment, MESSAGEV3_PHASE: String(phase) },
            path.join(suiteRoot, `cargo-phase${phase}.log`)
        );
        executionRecords.push({
            phase,
            filter: `phase${phase}_`,
            durationMs: execution.durationMs,
            code: execution.code,
            timed_out: execution.timedOut,
            timeout_ms: execution.timeoutMs
        });
        for (const test of selectedByPhase.get(phase)) {
            executions.set(test.filter, {
                code: testExitCode(execution.output, test.filter),
                durationMs: execution.durationMs,
                batch: true,
                timedOut: execution.timedOut
            });
        }
    }
    for (const test of selected.filter(candidate => !batchPhaseSet.has(candidate.phase))) {
        const gateRoot = path.join(suiteRoot, test.phaseName, test.domain, test.gate);
        const execution = await runCargo(
            test.filter,
            true,
            { ...environment, MESSAGEV3_PHASE_POOL: '0' },
            path.join(gateRoot, 'cargo.log')
        );
        executionRecords.push({
            phase: test.phase,
            filter: test.filter,
            durationMs: execution.durationMs,
            code: execution.code,
            timed_out: execution.timedOut,
            timeout_ms: execution.timeoutMs
        });
        executions.set(test.filter, {
            code: execution.code,
            durationMs: execution.durationMs,
            batch: false,
            timedOut: execution.timedOut
        });
    }
    const results = [];
    for (const test of selected) {
        const gateRoot = path.join(suiteRoot, test.phaseName, test.domain, test.gate);
        const execution = executions.get(test.filter) ?? { code: 102, durationMs: null, batch: false };
        const exitCode = execution.code;
        const evidence = await validateEvidence(path.join(gateRoot, 'evidence.json'), test, suiteRunId);
        const status = exitCode === 0 ? evidence.status : 'TEST_FAILED';
        results.push({
            ...test,
            exitCode,
            status,
            duration_ms: execution.durationMs,
            batch: execution.batch,
            timed_out: execution.timedOut ?? false,
            reason: exitCode === 0 ? evidence.reason : 'Cargo test 未通过、超时或未执行'
        });
        console.log(`${status}\t${test.phaseName}/${test.domain}/${test.gate}`);
    }
    const summary = {
        schema_version: 'messagev3-phase1-5-suite/v1',
        suite_run_id: suiteRunId,
        environment: runtimeEnvironment.name,
        selected: results.length,
        passed: results.filter(result => result.status === 'PASS').length,
        execution: {
            cargo_batches: executionRecords.length,
            batched_phases: batchPhases,
            per_gate_processes: executionRecords.filter(record => !batchPhases.includes(record.phase)).length,
            phase_pool_enabled: phasePoolRequested && batchPhases.length > 0,
            phase_pool_max_channels: 5,
            phase_pool_ws_reuse: phasePoolRequested,
            cargo: executionRecords,
            elapsed_ms: Date.now() - suiteStartedAt
        },
        results
    };
    await writeFile(path.join(suiteRoot, 'summary.json'), `${JSON.stringify(summary, null, 2)}\n`, 'utf8');
    if (summary.passed !== summary.selected) process.exitCode = 1;
}

main().catch(error => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
});
