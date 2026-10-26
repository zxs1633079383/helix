#!/usr/bin/env node

import { spawn, execFileSync } from 'node:child_process';
import { access, mkdir, mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const HELIX_ROOT = path.resolve(SCRIPT_DIR, '../..');
const MATRIX_PATH = path.join(SCRIPT_DIR, 'phase1-real-chain/matrix.json');
const DEFAULT_CLIENT_ROOT = path.resolve(HELIX_ROOT, '../../../angular/visual/cses-client');
const DEFAULT_GO_ROOT = path.resolve(
    HELIX_ROOT,
    '../../../golang/cses-im-server-MessageV3-helix'
);
const DEFAULT_ARTIFACT_ROOT = path.join(
    HELIX_ROOT,
    'artifacts/messagev3-phase1-real-chain'
);
const GO_BASE_URL = 'http://127.0.0.1:8066';
const PHASE1_GAPS = new Set(['MV3-G01h', 'MV3-G01i', 'MV3-G01j']);
const WDIO_PORTS = [4544, 4545, 4546];

/** 解析命令行选项；默认跑完整 Phase1，路径覆盖只作为调试逃生口。 */
function parseArgs(argv) {
    const options = {
        all: true,
        domains: [],
        gates: [],
        dryRun: false,
        skipBuild: false,
        clientRoot: DEFAULT_CLIENT_ROOT,
        goRoot: DEFAULT_GO_ROOT,
        helixRoot: HELIX_ROOT,
        artifactRoot: DEFAULT_ARTIFACT_ROOT
    };
    for (let index = 0; index < argv.length; index += 1) {
        const argument = argv[index];
        if (argument === '--all') {
            options.all = true;
            continue;
        }
        if (argument === '--dry-run') {
            options.dryRun = true;
            continue;
        }
        if (argument === '--skip-build') {
            options.skipBuild = true;
            continue;
        }
        const [name, inlineValue] = argument.split('=', 2);
        const value = inlineValue ?? argv[++index];
        if (!value) throw new Error(`缺少 ${name} 的值`);
        if (name === '--domain') options.domains.push(value);
        else if (name === '--gate') options.gates.push(value.toUpperCase());
        else if (name === '--client-root') options.clientRoot = path.resolve(value);
        else if (name === '--go-root') options.goRoot = path.resolve(value);
        else if (name === '--helix-root') options.helixRoot = path.resolve(value);
        else if (name === '--artifact-root') options.artifactRoot = path.resolve(value);
        else throw new Error(`不支持的参数: ${argument}`);
    }
    if (options.domains.length || options.gates.length) options.all = false;
    return options;
}

/** 读取 JSON 文件并保留原始错误上下文，供契约报告定位真源。 */
async function readJson(filePath) {
    return JSON.parse(await readFile(filePath, 'utf8'));
}

/** 判断文件或目录是否存在，不把缺失前置转化为异常堆栈。 */
async function exists(filePath) {
    return access(filePath).then(() => true).catch(() => false);
}

/** 读取一个仓库的 branch、HEAD 与脏状态，不输出 diff 内容或凭据。 */
function sourcePin(repoRoot) {
    // 只读取一个 Git 标量字段，禁止把工作区 diff 写入测试报告。
    const git = args => execFileSync('git', args, {
        cwd: repoRoot,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe']
    }).trim();
    return {
        repoRoot,
        branch: git(['branch', '--show-current']),
        head: git(['rev-parse', 'HEAD']),
        dirty: git(['status', '--porcelain']).length > 0
    };
}

/** 返回本机监听端口的占用摘要，只保留端口号以避免泄露用户进程路径。 */
function listeningPorts() {
    return WDIO_PORTS.filter(port => {
        try {
            const output = execFileSync('lsof', ['-nP', `-iTCP:${port}`, '-sTCP:LISTEN'], {
                encoding: 'utf8',
                stdio: ['ignore', 'pipe', 'ignore']
            });
            return output.trim().split(/\r?\n/).length > 1;
        } catch {
            return false;
        }
    });
}

/** 只用健康端点确认用户指定的 Go 进程在线；健康通过不升级为业务通过。 */
async function probeGoHealth() {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5_000);
    try {
        const response = await fetch(`${GO_BASE_URL}/api/cses/health`, { signal: controller.signal });
        const body = await response.json().catch(() => ({}));
        return {
            ok: response.ok && body?.status === 'OK',
            httpStatus: response.status,
            status: typeof body?.status === 'string' ? body.status : 'UNKNOWN'
        };
    } catch (error) {
        return {
            ok: false,
            httpStatus: null,
            status: 'UNREACHABLE',
            error: error instanceof Error ? error.message : String(error)
        };
    } finally {
        clearTimeout(timer);
    }
}

/** 创建只含非生产网络配置的临时 release profile，始终把 IM 指向 8066。 */
async function createReleaseConfig(clientRoot) {
    const sourcePath = path.join(clientRoot, 'release.local.json');
    const document = await readJson(sourcePath);
    const section = document?.cses?.dev && typeof document.cses.dev === 'object'
        ? document.cses.dev
        : document?.cses && typeof document.cses === 'object'
            ? document.cses
            : document;
    if (!section || typeof section !== 'object') {
        throw new Error('release.local.json 缺少 cses.dev 配置');
    }
    const envType = String(section.envType ?? '').toLowerCase();
    if (!envType || ['prod', 'production', 'online'].includes(envType)) {
        throw new Error('release.local.json 的 dev envType 不能是生产环境');
    }
    const patched = structuredClone(document);
    const patchedSection = patched?.cses?.dev && typeof patched.cses.dev === 'object'
        ? patched.cses.dev
        : patched?.cses && typeof patched.cses === 'object'
            ? patched.cses
            : patched;
    patchedSection.imApiHost = GO_BASE_URL;
    const directory = await mkdtemp(path.join(os.tmpdir(), 'helix-phase1-release-'));
    const target = path.join(directory, 'release.local.json');
    await writeFile(target, `${JSON.stringify(patched, null, 2)}\n`, { mode: 0o600 });
    return { directory, target };
}

/** 构造子进程环境；调用方无需设置任何 CSES_WDIO_* 变量。 */
function runtimeEnvironment({ options, releaseConfig }) {
    const inherited = Object.fromEntries(
        Object.entries(process.env).filter(([name]) => !name.startsWith('CSES_WDIO_'))
    );
    return {
        ...inherited,
        CSES_WDIO_PROVIDER: 'release-local',
        CSES_WDIO_RELEASE_ENV: 'dev',
        CSES_WDIO_RELEASE_CONFIG: releaseConfig,
        CSES_WDIO_REPO_ROOT: options.clientRoot,
        CSES_WDIO_HELIX_ROOT: options.helixRoot,
        CSES_WDIO_GO_ROOT: options.goRoot,
        CSES_WDIO_REFERENCE_ROOT: options.clientRoot,
        CSES_WDIO_ARTIFACT_ROOT: options.artifactRoot,
        CSES_WDIO_CREDENTIALS_FILE: path.join(options.clientRoot, '.wdio/credentials.local.json')
    };
}

/** 执行 Node 子进程并把完整输出保存到 artifact，终端只回显退出摘要。 */
async function runNode({ args, cwd, env, logPath }) {
    await mkdir(path.dirname(logPath), { recursive: true });
    return new Promise((resolve, reject) => {
        const child = spawn(process.execPath, args, {
            cwd,
            env,
            stdio: ['ignore', 'pipe', 'pipe']
        });
        let output = '';
        // 限制子进程日志内存窗口，同时完整写入本轮 artifact。
        const append = chunk => {
            output += chunk.toString();
            if (output.length > 2_000_000) output = output.slice(-2_000_000);
        };
        child.stdout.on('data', append);
        child.stderr.on('data', append);
        child.once('error', reject);
        child.once('close', async (code, signal) => {
            await writeFile(logPath, output, 'utf8');
            resolve({ code: code ?? 1, signal, logPath });
        });
    });
}

/** 从 artifact 根中发现最新的 WDIO run，不借用旧运行目录。 */
async function findLatestRun(artifactRoot, startedAtMs) {
    const entries = await readdir(artifactRoot, { withFileTypes: true }).catch(() => []);
    const candidates = [];
    for (const entry of entries.filter(item => item.isDirectory())) {
        const runRoot = path.join(artifactRoot, entry.name);
        const runFile = path.join(runRoot, 'run.json');
        const fileStat = await stat(runFile).catch(() => null);
        if (fileStat && fileStat.mtimeMs >= startedAtMs) candidates.push({ runRoot, mtimeMs: fileStat.mtimeMs });
    }
    candidates.sort((left, right) => right.mtimeMs - left.mtimeMs);
    if (!candidates[0]) return null;
    const runRoot = candidates[0].runRoot;
    const run = await readJson(path.join(runRoot, 'run.json'));
    const report = await readJson(path.join(runRoot, 'report.json')).catch(() => null);
    const pins = await readJson(path.join(runRoot, 'source-pins.json')).catch(() => null);
    return { runRoot, run, report, pins };
}

/** 从领域证据中提取 Gate 级结果，保留原始相对路径便于人工复核。 */
async function gateEvidence(runRoot, gateId) {
    const domainRoot = path.join(runRoot, 'domains');
    const domains = await readdir(domainRoot, { withFileTypes: true }).catch(() => []);
    for (const domain of domains.filter(item => item.isDirectory())) {
        const evidence = await readJson(path.join(domainRoot, domain.name, 'evidence.json')).catch(() => null);
        const gate = evidence?.gates?.find(item => item.gateId === gateId);
        if (gate) return { domain: domain.name, gate };
    }
    return { domain: null, gate: null };
}

/** 将 WDIO 的观察字段收敛成可审计状态，拒绝把 HTTP 200/health 当成完成。 */
function classifyGate({ gateId, evidence, run }) {
    const gap = PHASE1_GAPS.has(gateId);
    if (!evidence) return gap ? 'GAP_NOT_VERIFIED' : 'NOT_VERIFIED';
    if (gap) {
        if (evidence.result === 'BLOCKED') return 'GAP_BLOCKED';
        if (evidence.result === 'GAP_BOUNDARY_OBSERVED') return 'GAP_BOUNDARY_OBSERVED';
        return 'GAP_NOT_VERIFIED';
    }
    const required = [
        evidence.commandEvidence,
        evidence.httpEvidence,
        evidence.wsEvidence,
        evidence.projectionEvidence,
        evidence.domEvidence
    ];
    const observed = required.every(value => typeof value === 'string' && (
        value.startsWith('OBSERVED') || value.startsWith('NOT_APPLICABLE')
    ));
    if (!observed) return run?.status === 'BLOCKED_SETUP' ? 'BLOCKED' : 'NOT_VERIFIED';
    const storageNotRequired = gateId === 'MV3-G02d' || gateId === 'MV3-G05l';
    if (!storageNotRequired && !String(evidence.storageEvidence ?? '').startsWith('OBSERVED')) {
        return 'PARTIAL_STORAGE_READBACK';
    }
    return evidence.result === 'PARTIAL' ? 'PARTIAL_CANONICAL' : 'PARTIAL';
}

/** 校验 Phase1 三仓前置，返回阻塞项而不是在用户机器上隐式改服务或端口。 */
async function preflight(options, matrix) {
    const required = [
        [options.helixRoot, 'Helix worktree'],
        [options.clientRoot, 'Angular client'],
        [options.goRoot, 'Go server source'],
        [path.join(options.clientRoot, 'package.json'), 'Angular package.json'],
        [path.join(options.clientRoot, 'scripts/message-wdio/phase1/run-lane.mjs'), 'Angular Phase1 runner'],
        [path.join(options.clientRoot, 'release.local.json'), 'Angular release.local.json'],
        [path.join(options.clientRoot, '.wdio/credentials.local.json'), 'Angular credentials']
    ];
    const missing = [];
    for (const [filePath, label] of required) {
        if (!(await exists(filePath))) missing.push(`${label}: ${filePath}`);
    }
    const candidate = path.join(options.clientRoot, 'src-tauri/target/debug/cses-client');
    if (!(await exists(candidate))) missing.push(`Angular candidate: ${candidate}`);
    const ports = listeningPorts();
    const health = await probeGoHealth();
    const pins = {
        helix: sourcePin(options.helixRoot),
        client: sourcePin(options.clientRoot),
        go: sourcePin(options.goRoot)
    };
    const result = {
        ok: missing.length === 0 && ports.length === 0 && health.ok,
        missing,
        wdioPortsInUse: ports,
        goHealth: health,
        pins,
        gateDenominator: matrix.gates.length,
        gapDenominator: [...PHASE1_GAPS].length,
        goTarget: GO_BASE_URL
    };
    return result;
}

/** 根据 domain/gate 筛选运行单元，保持 Domain→Gate 分包且不重复执行无关 suite。 */
function selectRuns(matrix, options) {
    const selectedDomains = options.domains.length
        ? matrix.domains.filter(domain => options.domains.includes(domain.id))
        : matrix.domains;
    const unknownDomains = options.domains.filter(id => !matrix.domains.some(domain => domain.id === id));
    if (unknownDomains.length) throw new Error(`未知 Phase1 Domain: ${unknownDomains.join(', ')}`);
    const runs = [];
    for (const domain of selectedDomains) {
        for (const run of domain.runs) {
            const requested = options.gates.length === 0
                || run.gates.some(gate => options.gates.includes(gate));
            if (requested) runs.push({ domain, run });
        }
    }
    const unknownGates = options.gates.filter(gate => !matrix.gates.includes(gate));
    if (unknownGates.length) throw new Error(`未知 Phase1 Gate: ${unknownGates.join(', ')}`);
    if (!runs.length) throw new Error('筛选条件没有对应的 Phase1 Domain/Gate 运行单元');
    return runs;
}

/** 为完整运行写出不含凭据/正文的汇总证据，Gate 结果来自同轮 WDIO artifact。 */
async function writeSummary({ options, matrix, preflightResult, selectedRuns, results, status }) {
    await mkdir(options.artifactRoot, { recursive: true });
    const summary = {
        schemaVersion: 'messagev3-phase1-real-chain-summary/v1',
        phase: 1,
        status,
        gateDenominator: matrix.gates.length,
        gapDenominator: [...PHASE1_GAPS].length,
        target: GO_BASE_URL,
        worktree: sourcePin(options.helixRoot),
        preflight: preflightResult,
        selected: selectedRuns.map(({ domain, run }) => ({
            domain: domain.id,
            name: domain.name,
            lane: run.lane,
            suite: run.suite,
            gates: run.gates,
            spec: run.spec
        })),
        results
    };
    const target = path.join(options.artifactRoot, 'summary.json');
    await writeFile(target, `${JSON.stringify(summary, null, 2)}\n`, 'utf8');
    return target;
}

/** 打印稳定的中文执行计划，供 dry-run 和 CI 日志直接审阅。 */
function printPlan(matrix, selectedRuns, preflightResult) {
    console.log(`Phase1 Gate=${matrix.gates.length}，Gap=${PHASE1_GAPS.size}，Go=${GO_BASE_URL}`);
    console.log(`预检: ${preflightResult.ok ? 'PASS' : 'BLOCKED'}；端口占用=${preflightResult.wdioPortsInUse.join(',') || '无'}`);
    for (const { domain, run } of selectedRuns) {
        console.log(`- ${domain.id} -> ${run.lane}/${run.suite} -> ${run.gates.join(', ')}`);
        console.log(`  Angular class: ${run.spec}`);
    }
}

/** 主流程：预检、构建一次、逐 Domain 执行真实 runner，并为每个 Gate 记录证据等级。 */
async function main() {
    const options = parseArgs(process.argv.slice(2));
    const matrix = await readJson(MATRIX_PATH);
    const registry = await readJson(path.join(options.helixRoot, matrix.gateRegistry));
    const clientManifest = await readJson(path.join(options.clientRoot, matrix.clientManifest));
    if (registry.denominator?.total !== 33 || clientManifest.expectedGateCount !== 33) {
        throw new Error('Phase1 Gate 分母漂移：Helix registry 与 Angular manifest 必须都是 33');
    }
    const matrixGates = matrix.domains.flatMap(domain => domain.runs.flatMap(run => run.gates));
    if (new Set(matrixGates).size !== 33 || matrixGates.some(gate => !clientManifest.gates.includes(gate))) {
        throw new Error('Phase1 Domain→Gate 矩阵未覆盖 Angular manifest 的 33 个 Gate');
    }
    const selectedRuns = selectRuns({ ...matrix, gates: clientManifest.gates }, options);
    const preflightResult = await preflight(options, { ...matrix, gates: clientManifest.gates });
    printPlan({ ...matrix, gates: clientManifest.gates }, selectedRuns, preflightResult);
    if (options.dryRun) {
        const summaryPath = await writeSummary({
            options,
            matrix: { ...matrix, gates: clientManifest.gates },
            preflightResult,
            selectedRuns,
            results: [],
            status: preflightResult.ok ? 'DRY_RUN_READY' : 'DRY_RUN_BLOCKED'
        });
        console.log(`dry-run artifact: ${summaryPath}`);
        return preflightResult.ok ? 0 : 2;
    }
    if (!preflightResult.ok) {
        const summaryPath = await writeSummary({
            options,
            matrix: { ...matrix, gates: clientManifest.gates },
            preflightResult,
            selectedRuns,
            results: [],
            status: 'BLOCKED_PRECONDITION'
        });
        console.error(`BLOCKED: 真实链路前置不满足；详情见 ${summaryPath}`);
        return 2;
    }

    const release = await createReleaseConfig(options.clientRoot);
    const env = runtimeEnvironment({ options, releaseConfig: release.target });
    const results = [];
    try {
        if (!options.skipBuild) {
            const buildStartedAt = Date.now();
            const build = await runNode({
                args: ['scripts/message-wdio/phase1/build-candidate.mjs'],
                cwd: options.clientRoot,
                env,
                logPath: path.join(options.artifactRoot, 'build.log')
            });
            results.push({ type: 'build', code: build.code, log: path.relative(options.artifactRoot, build.logPath) });
            if (build.code !== 0) {
                const summaryPath = await writeSummary({
                    options,
                    matrix: { ...matrix, gates: clientManifest.gates },
                    preflightResult,
                    selectedRuns,
                    results,
                    status: 'BLOCKED_BUILD'
                });
                console.error(`BLOCKED: Angular candidate 构建失败；详情见 ${summaryPath}`);
                return 2;
            }
            console.log(`Angular candidate build PASS (${Date.now() - buildStartedAt} ms)`);
        }
        for (const [index, selected] of selectedRuns.entries()) {
            const { domain, run } = selected;
            const startedAt = Date.now();
            const logPath = path.join(options.artifactRoot, 'child-logs', `${String(index + 1).padStart(2, '0')}-${domain.id}-${run.suite}.log`);
            const child = await runNode({
                args: ['scripts/message-wdio/phase1/run-lane.mjs', run.lane, run.suite],
                cwd: options.clientRoot,
                env,
                logPath
            });
            const runtime = await findLatestRun(options.artifactRoot, startedAt - 1_000);
            const gateResults = [];
            for (const gateId of run.gates) {
                const evidence = runtime ? await gateEvidence(runtime.runRoot, gateId) : { domain: null, gate: null };
                gateResults.push({
                    gateId,
                    status: classifyGate({ gateId, evidence: evidence.gate, run: runtime?.run }),
                    domain: evidence.domain,
                    result: evidence.gate?.result ?? 'UNVERIFIED',
                    evidence: evidence.gate
                        ? {
                            command: evidence.gate.commandEvidence,
                            storage: evidence.gate.storageEvidence,
                            http: evidence.gate.httpEvidence,
                            ws: evidence.gate.wsEvidence,
                            projection: evidence.gate.projectionEvidence,
                            dom: evidence.gate.domEvidence
                        }
                        : null
                });
            }
            results.push({
                type: 'domain-run',
                domain: domain.id,
                lane: run.lane,
                suite: run.suite,
                childExitCode: child.code,
                runId: runtime?.run?.runId ?? null,
                runStatus: runtime?.run?.status ?? 'NOT_FOUND',
                runRoot: runtime ? path.relative(options.artifactRoot, runtime.runRoot) : null,
                log: path.relative(options.artifactRoot, logPath),
                gates: gateResults
            });
            console.log(`${domain.id}/${run.suite}: child=${child.code}, ${gateResults.map(item => `${item.gateId}=${item.status}`).join(' ')}`);
        }
    } finally {
        await rm(release.directory, { recursive: true, force: true });
    }
    const statuses = results.flatMap(result => result.type === 'domain-run' ? result.gates.map(gate => gate.status) : []);
    const status = statuses.some(item => ['BLOCKED', 'NOT_VERIFIED', 'GAP_NOT_VERIFIED'].includes(item))
        ? 'PARTIAL_OR_BLOCKED'
        : statuses.some(item => item.startsWith('PARTIAL'))
            ? 'PARTIAL'
            : 'OBSERVED';
    const summaryPath = await writeSummary({
        options,
        matrix: { ...matrix, gates: clientManifest.gates },
        preflightResult,
        selectedRuns,
        results,
        status
    });
    console.log(`Phase1 summary: ${summaryPath}`);
    return status === 'OBSERVED' ? 0 : 1;
}

main().then(code => {
    process.exitCode = code;
}).catch(error => {
    console.error(`ERROR: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
});
