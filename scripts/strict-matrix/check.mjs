#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '../..');
const crateRoot = path.join(repoRoot, 'crates/helix-im');
const matrixPath = path.join(crateRoot, 'contracts/strict-matrix.v1.json');
const schemaPath = path.join(crateRoot, 'contracts/strict-matrix.schema.json');
const markdownPath = path.join(crateRoot, 'docs/Strict Matrix/UC全链路发布矩阵-v1.md');
const commandApiPath = path.join(crateRoot, 'contracts/command-api.v1.json');
const commandApiMarkdownPath = path.join(crateRoot, 'docs/Strict Matrix/Command接口-v1.md');

const argv = process.argv.slice(2);
const args = new Set(argv);

function optionValue(name) {
    const index = argv.indexOf(name);
    if (index < 0) return undefined;
    const value = argv[index + 1];
    if (!value || value.startsWith('--')) fail(`${name} 缺少值`);
    return value;
}

const legacyConsumerPath = optionValue('--consumer-ts');
const writeConsumerPath = optionValue('--write-consumer-ts');
const checkConsumerPath = optionValue('--check-consumer-ts');
const revisionArg = optionValue('--revision');
if (writeConsumerPath && checkConsumerPath && path.resolve(writeConsumerPath) !== path.resolve(checkConsumerPath)) {
    fail('--write-consumer-ts 与 --check-consumer-ts 必须指向同一文件');
}
if ((writeConsumerPath || checkConsumerPath) && !revisionArg) {
    fail('--write-consumer-ts/--check-consumer-ts 必须提供正式 --revision');
}
const consumerPath = writeConsumerPath ?? checkConsumerPath ?? legacyConsumerPath;

function fail(message) {
    throw new Error(`strict-matrix: ${message}`);
}

function isObject(value) {
    return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function requireArray(value, field) {
    if (!Array.isArray(value)) fail(`${field} 必须是数组`);
    return value;
}

function requireString(value, field) {
    if (typeof value !== 'string' || value.length === 0) fail(`${field} 必须是非空字符串`);
    return value;
}

function unique(values, field) {
    if (new Set(values).size !== values.length) fail(`${field} 不允许重复`);
}

function resolvedCase(matrix, entry) {
    return {
        ...entry,
        owner: entry.owner ?? matrix.case_defaults.owner,
        evidence: entry.evidence ?? matrix.case_defaults.evidence
    };
}

function validateStrictInput(input, label) {
    if (!isObject(input) || input.forbid_unknown !== true) fail(`${label}.input 必须拒绝未知字段`);
    const required = requireArray(input.required, `${label}.input.required`);
    const optional = requireArray(input.optional, `${label}.input.optional`);
    unique(required, `${label}.input.required`);
    unique(optional, `${label}.input.optional`);
    for (const field of [...required, ...optional]) {
        if (!/^[a-z][a-z0-9_]*$/.test(field)) fail(`${label}.input 含非 canonical snake_case 字段 ${field}`);
    }
    const overlap = required.find(field => optional.includes(field));
    if (overlap) fail(`${label}.input required/optional 重复 ${overlap}`);
}

function validateMatrix(matrix, schema, sourceDocumentExists) {
    if (!isObject(schema) || schema.$id !== 'https://helix.im/contracts/strict-matrix/v1/schema.json') {
        fail('JSON Schema 缺失或版本错误');
    }
    if (!isObject(matrix) || matrix.schema_version !== 'helix-im-strict-matrix/v1') fail('schema_version 错误');
    if (!Number.isInteger(matrix.contract_epoch) || matrix.contract_epoch < 1) fail('contract_epoch 错误');
    const sourceDocuments = requireArray(matrix.source_documents, 'source_documents');
    for (const source of sourceDocuments) {
        requireString(source, 'source_documents[]');
        if (!sourceDocumentExists(source)) fail(`source document 不存在: ${source}`);
    }
    const facets = requireArray(matrix.evidence_facets, 'evidence_facets');
    const expectedFacets = ['command', 'effect', 'transport', 'local_store', 'remote_store', 'projection', 'dom', 'trace'];
    if (JSON.stringify(facets) !== JSON.stringify(expectedFacets)) fail('evidence_facets 必须是完整八面且顺序固定');
    const aliases = requireArray(matrix.forbidden_aliases, 'forbidden_aliases');
    unique(aliases, 'forbidden_aliases');
    const forbiddenAliases = new Set(aliases);
    const renderProtocol = matrix.render_protocol;
    if (!isObject(renderProtocol) || renderProtocol.name !== 'RenderNode' || renderProtocol.version !== 1) fail('RenderNode/v1 未冻结');
    const envelopeFields = requireArray(renderProtocol.envelope_fields, 'render_protocol.envelope_fields');
    for (const field of ['entity_id', 'entity_revision', 'render_kind', 'props', 'allowed_actions', 'correlation']) {
        if (!envelopeFields.includes(field)) fail(`RenderNode 缺字段 ${field}`);
    }
    const renderKinds = requireArray(renderProtocol.render_kinds, 'render_protocol.render_kinds');
    const renderKindSet = new Set();
    for (const kind of renderKinds) {
        if (!isObject(kind) || !/^[a-z]+(?:[._][a-z]+)+\.v1$/.test(kind.kind ?? '')) fail('render_kind 格式错误');
        if (renderKindSet.has(kind.kind)) fail(`重复 render_kind: ${kind.kind}`);
        renderKindSet.add(kind.kind);
        if (requireArray(kind.required_props, `${kind.kind}.required_props`).length === 0) fail(`${kind.kind} 无 render props`);
    }
    const defaults = matrix.case_defaults;
    if (!isObject(defaults) || !isObject(defaults.owner) || !isObject(defaults.evidence)) fail('case_defaults 缺失');
    if (defaults.owner.business_state !== 'helix' || defaults.owner.host !== 'transport-storage-only' || defaults.owner.ui !== 'intent-and-render-only') {
        fail('默认所有权必须是 Helix/纯宿主/纯 UI');
    }
    if (JSON.stringify(defaults.evidence.required_facets) !== JSON.stringify(expectedFacets)) fail('默认 e2e 证据面不完整');
    for (const key of ['run_id', 'gate_id', 'operation_id', 'req_id', 'channel_id']) {
        if (!defaults.evidence.correlation_keys.includes(key)) fail(`默认关联键缺 ${key}`);
    }
    const coverage = matrix.coverage;
    if (!isObject(coverage) || !Number.isInteger(coverage.minimum_gate_count)) fail('coverage 缺失');
    const requiredGates = requireArray(coverage.required_gate_ids, 'coverage.required_gate_ids');
    unique(requiredGates, 'coverage.required_gate_ids');
    const cases = requireArray(matrix.uc_cases, 'uc_cases').map(entry => resolvedCase(matrix, entry));
    if (cases.length < coverage.minimum_gate_count) fail(`Gate 数 ${cases.length} 小于 baseline ${coverage.minimum_gate_count}`);
    const seenGates = new Set();
    for (const entry of cases) {
        const label = entry.gate_id ?? '<unknown-gate>';
        if (!/^G-(?:S[1-4]|[0-9]{2}[a-z]?)$/.test(label)) fail(`非法 gate_id: ${label}`);
        if (seenGates.has(label)) fail(`重复 Gate: ${label}`);
        seenGates.add(label);
        requireString(entry.uc_id, `${label}.uc_id`);
        requireString(entry.title, `${label}.title`);
        if (!['P0', 'P1', 'P2', 'P3', 'P4', 'P5', 'P6', 'P7', 'L2'].includes(entry.phase)) fail(`${label}.phase 错误`);
        if (!['active', 'deferred-by-user', 'excluded'].includes(entry.lifecycle)) fail(`${label}.lifecycle 错误`);
        if (!['declared', 'implemented', 'blocked'].includes(entry.contract_state)) fail(`${label}.contract_state 错误`);
        if (entry.lifecycle === 'active' && entry.contract_state === 'blocked') fail(`${label}: active 不可为 blocked`);
        if (!isObject(entry.owner) || entry.owner.business_state !== 'helix' || entry.owner.host !== 'transport-storage-only' || entry.owner.ui !== 'intent-and-render-only') fail(`${label}: 所有权越界`);
        const commands = requireArray(entry.commands, `${label}.commands`);
        if (commands.length === 0) fail(`${label}: 无直接 core command`);
        for (const step of commands) {
            if (!isObject(step) || !/^im_[a-z0-9_]+$/.test(step.name ?? '')) fail(`${label}: command 名错误`);
            if (forbiddenAliases.has(step.name)) fail(`${label}: 使用禁止 alias ${step.name}`);
            validateStrictInput(step.input, `${label}.${step.name}`);
            if (requireArray(step.effect, `${label}.${step.name}.effect`).length === 0) fail(`${label}.${step.name} 无 Effect`);
        }
        if (!isObject(entry.http)) fail(`${label}: HTTP request/response 合同缺失`);
        requireString(entry.http.request, `${label}.http.request`);
        requireString(entry.http.response, `${label}.http.response`);
        if (!isObject(entry.ws)) fail(`${label}: WS 合同缺失`);
        requireString(entry.ws.events, `${label}.ws.events`);
        if (!isObject(entry.remote_store)) fail(`${label}: 远端存储合同缺失`);
        if (!['mattermost-imv2', 'N/A', 'external-unverified'].includes(entry.remote_store.database)) fail(`${label}: 远端库名非法`);
        requireArray(entry.remote_store.tables, `${label}.remote_store.tables`);
        requireString(entry.remote_store.contract, `${label}.remote_store.contract`);
        if (entry.lifecycle === 'active' && entry.remote_store.database === 'external-unverified') fail(`${label}: active UC 不得使用未核验远端库`);
        if (requireArray(entry.persistence, `${label}.persistence`).length === 0) fail(`${label}: 未声明持久化`);
        const projections = requireArray(entry.projections, `${label}.projections`);
        if (projections.length === 0) fail(`${label}: 未声明投影`);
        for (const projection of projections) {
            if (!isObject(projection) || !/^im:[a-z0-9:._-]+$/.test(projection.event ?? '')) fail(`${label}: projection event 错误`);
            if (projection.schema !== 'RenderNode/v1') fail(`${label}: projection 未使用 RenderNode/v1`);
            if (!renderKindSet.has(projection.render_kind)) fail(`${label}: 未登记 render_kind ${projection.render_kind}`);
            if (projection.required_data_fields !== undefined) {
                unique(requireArray(projection.required_data_fields, `${label}.required_data_fields`), `${label}.required_data_fields`);
            }
            if (projection.merge_contract !== undefined) requireString(projection.merge_contract, `${label}.merge_contract`);
        }
        if (label === 'G-12') {
            const replies = projections.find(projection => projection.event === 'im:channel:replies');
            const required = ['reqId', 'channelId', 'rootMessageId', 'projectionMode', 'revision', 'replyIds', 'nodes', 'replyCount', 'cursor', 'hasMore'];
            if (!replies || JSON.stringify(replies.required_data_fields) !== JSON.stringify(required)) {
                fail('G-12: reply projection 字段或顺序未冻结');
            }
            if (!replies.merge_contract?.includes('旧 revision') || !replies.merge_contract?.includes('append')) {
                fail('G-12: 乱序/增量合并合同缺失');
            }
        }
        requireArray(entry.allowed_actions, `${label}.allowed_actions`);
        if (JSON.stringify(entry.evidence.required_facets) !== JSON.stringify(expectedFacets)) fail(`${label}: 证据面不完整`);
        for (const key of ['run_id', 'gate_id', 'operation_id', 'req_id', 'channel_id']) {
            if (!entry.evidence.correlation_keys.includes(key)) fail(`${label}: 关联键缺 ${key}`);
        }
        if (requireArray(entry.negative_assertions, `${label}.negative_assertions`).length === 0) fail(`${label}: 缺反向断言`);
    }
    for (const gateId of requiredGates) if (!seenGates.has(gateId)) fail(`缺 baseline Gate: ${gateId}`);
    return cases;
}

function stableJson(value) {
    if (Array.isArray(value)) return `[${value.map(stableJson).join(',')}]`;
    if (isObject(value)) {
        return `{${Object.keys(value).sort().map(key => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(',')}}`;
    }
    return JSON.stringify(value);
}

function matrixHashes(matrixBytes, matrix) {
    return {
        raw: createHash('sha256').update(matrixBytes).digest('hex'),
        semantic: createHash('sha256').update(stableJson(matrix)).digest('hex')
    };
}

function cloneJson(value) {
    return JSON.parse(JSON.stringify(value));
}

function resolveRevision(revision) {
    if (!revision) return undefined;
    if (!/^[0-9a-f]{7,64}$/i.test(revision)) fail('--revision 必须是 Git commit SHA');
    try {
        return execFileSync('git', ['rev-parse', '--verify', `${revision}^{commit}`], {
            cwd: repoRoot,
            encoding: 'utf8'
        }).trim();
    } catch {
        fail(`无法解析正式 Helix commit: ${revision}`);
    }
}

function readRepositoryBlob(relativePath, revision) {
    if (!revision) return readFileSync(path.join(repoRoot, relativePath));
    try {
        return execFileSync('git', ['show', `${revision}:${relativePath}`], {
            cwd: repoRoot,
            encoding: 'buffer'
        });
    } catch {
        fail(`正式 commit 缺少文件: ${relativePath}@${revision}`);
    }
}

function sourceDocumentExists(source, revision) {
    if (!revision) return existsSync(path.join(crateRoot, source));
    try {
        execFileSync('git', ['cat-file', '-e', `${revision}:crates/helix-im/${source}`], {
            cwd: repoRoot,
            stdio: 'ignore'
        });
        return true;
    } catch {
        return false;
    }
}

function renderMarkdown(matrix, cases, hashes) {
    const rows = cases.map(entry => {
        const commands = entry.commands.map(step => `${step.name}(${[...step.input.required, ...step.input.optional].join(', ') || '∅'})`).join('<br>');
        const effects = [...new Set(entry.commands.flatMap(step => step.effect))].join(', ');
        const projections = entry.projections.map(item => {
            const fields = item.required_data_fields ? `<br>fields: {${item.required_data_fields.join(',')}}` : '';
            const merge = item.merge_contract ? `<br>${item.merge_contract}` : '';
            return `${item.event} → ${item.render_kind}${fields}${merge}`;
        }).join('<br>');
        const renderKinds = entry.projections.map(item => item.render_kind).join('<br>');
        const persistence = entry.persistence.join(', ');
        const remoteTables = entry.remote_store.tables.length > 0 ? entry.remote_store.tables.join(', ') : '∅';
        return `| ${entry.gate_id} | ${entry.uc_id} ${entry.title} | ${renderKinds}<br>DOM | ${commands}<br>${effects}<br>${projections} | request: ${entry.http.request}<br>response: ${entry.http.response} | ${entry.ws.events} | local: ${persistence}<br>remote DB: ${entry.remote_store.database}<br>tables: ${remoteTables}<br>${entry.remote_store.contract} | real Tauri/FFI + trace | ${entry.lifecycle}/${entry.contract_state} |`;
    });
    const stateCounts = cases.reduce((counts, entry) => {
        counts[entry.contract_state] = (counts[entry.contract_state] ?? 0) + 1;
        return counts;
    }, {});
    return `# Helix IM Strict Matrix v1

> 由 \`contracts/strict-matrix.v1.json\` 生成；禁止手工修改。\n> contract epoch: ${matrix.contract_epoch}; raw matrix sha256: \`${hashes.raw}\`; semantic matrix sha256: \`${hashes.semantic}\`。\n> raw hash 绑定正式 commit blob 与下游 pin；semantic hash 仅用于内容诊断，二者不可混用。\n> \`declared\` 与 \`blocked\` 均不可被运行证据晋升为 Green；只有实现 RenderNode/v1、直接 core command 与八面证据均完整的 \`implemented\` UC 才可晋升。

> 这张表不是“功能清单”，而是 \`UC × UI × Helix × HTTP × WS × Storage × Native\` 发布闸门。任一证据平面缺失、过期、无法关联或仅由 mock 产生，该 UC 整行即为 Red。

## 边界

- 业务状态、同步/补偿、read receipt、回复树、可见性和 allowed actions 均归 Helix。
- Tauri/FFI 只负责 transport、storage port 和透明 bus 转发；不改 command 名、payload 或业务投影。
- UI 只提交用户 intent 并按 \`render_kind\` 静态选择组件；不生成业务 id、不推导展示语义。
- 八面证据：${matrix.evidence_facets.map(item => `\`${item}\``).join('、')}。关联键：\`${matrix.case_defaults.evidence.correlation_keys.join('`、`')}\`。

## Fail-closed 发布判定

- **UI**：必须有真实用户操作、RenderNode 消费结果、稳定 DOM 断言和必要的双视角截图；可点击 mock 不代表通过。
- **Helix**：必须证明直接 core command 、Effect、权威 projection 及反向断言；Tauri alias 或 UI 推导不可替代。
- **HTTP**：必须保留 request/response 与 \`req_id\`。若该 UC 不应发 HTTP，必须给出“未发生”的反向证明，不允许留空。
- **WS**：必须保留发送/接收 frame、事件顺序和观察者结果。不应产生 WS 的 UC 同样必须有反向证明。
- **Storage**：必须分开证明本地投影与远端权威事实；只有内存 Store 或只有远端 200 都不足以通过。
- **Native**：必须由真实 Tauri/FFI 进程执行，并将本行所有证据绑定到同一组关联键；browser-only 或旧 run 证据不可顶替。
- **N/A 不是空值**：只允许“带理由、带反向证据的 not-applicable”。任何空单元格、未知状态或无法关联的证据均按 Red 处理。

## RenderNode/v1

| render_kind | 必须 props |
| --- | --- |
${matrix.render_protocol.render_kinds.map(kind => `| ${kind.kind} | ${kind.required_props.join(', ')} |`).join('\n')}

## UC × 七平面发布矩阵

| Gate | UC | UI | Helix | HTTP | WS | Storage | Native | 生命周期/合同 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
${rows.join('\n')}

## 当前统计

- Gate 总数：${cases.length}（baseline ≥ ${matrix.coverage.minimum_gate_count}）。
- 合同状态：${Object.entries(stateCounts).map(([state, count]) => `${state}=${count}`).join('，')}。
- 禁止 alias：${matrix.forbidden_aliases.map(alias => `\`${alias}\``).join('、')}。
`;
}

function buildCommandApi(matrix, cases, hashes) {
    const commands = new Map();
    for (const entry of cases) {
        for (const step of entry.commands) {
            if (!commands.has(step.name)) commands.set(step.name, { name: step.name, uses: [] });
            commands.get(step.name).uses.push({
                gate_id: entry.gate_id,
                uc_id: entry.uc_id,
                title: entry.title,
                phase: entry.phase,
                lifecycle: entry.lifecycle,
                contract_state: entry.contract_state,
                input: {
                    required: [...step.input.required],
                    optional: [...step.input.optional],
                    forbid_unknown: step.input.forbid_unknown
                },
                effects: [...step.effect],
                projections: cloneJson(entry.projections),
                persistence: [...entry.persistence],
                http: cloneJson(entry.http),
                ws: cloneJson(entry.ws),
                remote_store: cloneJson(entry.remote_store)
            });
        }
    }
    return {
        schema_version: 'helix-im-command-api/v1',
        source: {
            matrix_path: 'crates/helix-im/contracts/strict-matrix.v1.json',
            contract_epoch: matrix.contract_epoch,
            matrix_sha256: hashes.raw,
            matrix_semantic_sha256: hashes.semantic,
            gate_count: cases.length,
            command_count: commands.size
        },
        ownership: cloneJson(matrix.case_defaults.owner),
        evidence_facets: [...matrix.evidence_facets],
        forbidden_aliases: [...matrix.forbidden_aliases],
        commands: [...commands.values()]
    };
}

function renderCommandApiMarkdown(api) {
    const rows = api.commands.flatMap(command => command.uses.map(use => {
        const required = use.input.required.join(', ') || '∅';
        const optional = use.input.optional.join(', ') || '∅';
        const effects = use.effects.join(', ') || '∅';
        const projections = use.projections.map(projection => projection.event).join(', ') || '∅';
        return `| ${command.name} | ${use.gate_id} / ${use.uc_id} ${use.title} | ${required} | ${optional} | ${effects} | ${projections} | ${use.lifecycle}/${use.contract_state} |`;
    }));
    return `# Helix IM Command API v1

> 由 \`contracts/strict-matrix.v1.json\` 生成；禁止手工修改。
> contract epoch: ${api.source.contract_epoch}; raw matrix sha256: \`${api.source.matrix_sha256}\`; semantic matrix sha256: \`${api.source.matrix_semantic_sha256}\`。
> raw hash 是正式 Helix commit blob、PC/Mobile pin 与生成物的唯一绑定值；semantic hash 仅用于诊断。

## 边界

- 本文件仅声明 Helix canonical command 与各 Gate 的输入、Effect、投影和合同状态；不表示 HTTP/WS/投影真实链已 Green。
- 同一 command 的多行 use 不能合并：每个 Gate 的 required/optional 字段和投影语义都必须独立保留。
- UI 只发 canonical command；host 只传输原始 JSON payload；alias、业务字段注入与 UI temporary id 均不属于 host。

## Canonical command uses

| Command | Gate / UC | Required | Optional | Effects | Projections | Lifecycle / Contract |
| --- | --- | --- | --- | --- | --- | --- |
${rows.join('\n')}

## 禁止 alias

${api.forbidden_aliases.map(alias => `- \`${alias}\``).join('\n')}
`;
}

function renderConsumerTypeScript(matrix, cases, hashes, revision) {
    const payload = cases.map(entry => ({
        gateId: entry.gate_id,
        ucId: entry.uc_id,
        title: entry.title,
        lifecycle: entry.lifecycle,
        contractState: entry.contract_state,
        commands: entry.commands.map(step => ({
            name: step.name,
            required: step.input.required,
            optional: step.input.optional,
            effects: step.effect
        })),
        projections: entry.projections.map(projection => ({
            event: projection.event,
            renderKind: projection.render_kind
        })),
        allowedActions: entry.allowed_actions,
        requiredEvidence: entry.evidence.required_facets,
        correlationKeys: entry.evidence.correlation_keys
    }));
    const meta = {
        schemaVersion: matrix.schema_version,
        contractEpoch: matrix.contract_epoch,
        matrixSha256: hashes.raw,
        matrixSemanticSha256: hashes.semantic,
        helixRevision: revision ?? null,
        renderProtocol: 'RenderNode/v1',
        forbiddenAliases: matrix.forbidden_aliases,
        source: 'helix-im/contracts/strict-matrix.v1.json'
    };
    return `/* Generated from Helix contracts/strict-matrix.v1.json. Do not edit. */
export const HELIX_STRICT_MATRIX_META = ${JSON.stringify(meta, null, 4)} as const;

export const HELIX_STRICT_UC_ROWS = ${JSON.stringify(payload, null, 4)} as const;

export type HelixStrictUcRow = typeof HELIX_STRICT_UC_ROWS[number];
`;
}

function assertGenerated(pathname, expected) {
    if (!existsSync(pathname)) fail(`缺生成产物: ${path.relative(repoRoot, pathname)}`);
    if (readFileSync(pathname, 'utf8') !== expected) fail(`生成产物漂移: ${path.relative(repoRoot, pathname)}；请运行 --write`);
}

const revision = resolveRevision(revisionArg);
const matrixBytes = readRepositoryBlob('crates/helix-im/contracts/strict-matrix.v1.json', revision);
const schemaBytes = readRepositoryBlob('crates/helix-im/contracts/strict-matrix.schema.json', revision);
const matrix = JSON.parse(matrixBytes.toString('utf8'));
const schema = JSON.parse(schemaBytes.toString('utf8'));
const cases = validateMatrix(matrix, schema, source => sourceDocumentExists(source, revision));
const hashes = matrixHashes(matrixBytes, matrix);
const markdown = renderMarkdown(matrix, cases, hashes);
const commandApi = buildCommandApi(matrix, cases, hashes);
const commandApiJson = `${JSON.stringify(commandApi, null, 2)}\n`;
const commandApiMarkdown = renderCommandApiMarkdown(commandApi);
const consumer = consumerPath ? renderConsumerTypeScript(matrix, cases, hashes, revision) : undefined;
const writeDocs = args.has('--write') || args.has('--write-docs');
const checkDocs = args.has('--check-generated') || args.has('--check-docs');
const writeConsumer = Boolean(writeConsumerPath) || (args.has('--write') && Boolean(legacyConsumerPath));
const checkConsumer = Boolean(checkConsumerPath) || (args.has('--check-generated') && Boolean(legacyConsumerPath));

if (revision && writeDocs) fail('--revision 只允许生成下游 consumer，不能重写 Helix 工作树生成物');
if (writeDocs) {
    writeFileSync(markdownPath, markdown);
    writeFileSync(commandApiPath, commandApiJson);
    writeFileSync(commandApiMarkdownPath, commandApiMarkdown);
}
if (writeConsumer) writeFileSync(path.resolve(consumerPath), consumer);
if (checkDocs) {
    assertGenerated(markdownPath, markdown);
    assertGenerated(commandApiPath, commandApiJson);
    assertGenerated(commandApiMarkdownPath, commandApiMarkdown);
}
if (checkConsumer) assertGenerated(path.resolve(consumerPath), consumer);
if (!writeDocs && !checkDocs && !writeConsumer && !checkConsumer) {
    process.stdout.write(`${JSON.stringify({
        gates: cases.length,
        commands: commandApi.source.command_count,
        contractEpoch: matrix.contract_epoch,
        matrixSha256: hashes.raw,
        matrixSemanticSha256: hashes.semantic,
        helixRevision: revision ?? null
    }, null, 2)}\n`);
}
