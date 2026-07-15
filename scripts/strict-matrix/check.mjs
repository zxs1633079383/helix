#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '../..');
const crateRoot = path.join(repoRoot, 'crates/helix-im');
const matrixPath = path.join(crateRoot, 'contracts/strict-matrix.v1.json');
const schemaPath = path.join(crateRoot, 'contracts/strict-matrix.schema.json');
const markdownPath = path.join(crateRoot, 'docs/Strict Matrix/UC全链路发布矩阵-v1.md');

const args = new Set(process.argv.slice(2));
const consumerIndex = process.argv.indexOf('--consumer-ts');
const consumerPath = consumerIndex >= 0 ? path.resolve(process.argv[consumerIndex + 1] ?? '') : undefined;

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

function validateMatrix(matrix, schema) {
    if (!isObject(schema) || schema.$id !== 'https://helix.im/contracts/strict-matrix/v1/schema.json') {
        fail('JSON Schema 缺失或版本错误');
    }
    if (!isObject(matrix) || matrix.schema_version !== 'helix-im-strict-matrix/v1') fail('schema_version 错误');
    if (!Number.isInteger(matrix.contract_epoch) || matrix.contract_epoch < 1) fail('contract_epoch 错误');
    const sourceDocuments = requireArray(matrix.source_documents, 'source_documents');
    for (const source of sourceDocuments) {
        requireString(source, 'source_documents[]');
        if (!existsSync(path.join(crateRoot, source))) fail(`source document 不存在: ${source}`);
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

function contractHash(matrix) {
    return createHash('sha256').update(JSON.stringify(matrix)).digest('hex');
}

function renderMarkdown(matrix, cases, digest) {
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

> 由 \`contracts/strict-matrix.v1.json\` 生成；禁止手工修改。\n> contract epoch: ${matrix.contract_epoch}; sha256: \`${digest}\`。\n> \`declared\` 与 \`blocked\` 均不可被运行证据晋升为 Green；只有实现 RenderNode/v1、直接 core command 与八面证据均完整的 \`implemented\` UC 才可晋升。

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

function renderConsumerTypeScript(matrix, cases, digest) {
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
        contractSha256: digest,
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

const matrix = JSON.parse(readFileSync(matrixPath, 'utf8'));
const schema = JSON.parse(readFileSync(schemaPath, 'utf8'));
const cases = validateMatrix(matrix, schema);
const digest = contractHash(matrix);
const markdown = renderMarkdown(matrix, cases, digest);
const consumer = consumerPath ? renderConsumerTypeScript(matrix, cases, digest) : undefined;

if (args.has('--write')) {
    writeFileSync(markdownPath, markdown);
    if (consumerPath) writeFileSync(consumerPath, consumer);
}
if (args.has('--check-generated')) {
    assertGenerated(markdownPath, markdown);
    if (consumerPath) assertGenerated(consumerPath, consumer);
}
if (!args.has('--write') && !args.has('--check-generated')) {
    process.stdout.write(`${JSON.stringify({ gates: cases.length, contractEpoch: matrix.contract_epoch, contractSha256: digest }, null, 2)}\n`);
}
