#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { buildDashboard, contract, outputPath, scopeLink } from './generate-client-channel-dashboard.mjs';

const dir = path.dirname(fileURLToPath(import.meta.url));
const snapshot = JSON.parse(fs.readFileSync(path.resolve(dir, contract.snapshot), 'utf8'));
const dashboard = JSON.parse(fs.readFileSync(outputPath, 'utf8'));

// 扁平化row仅用于验证，不改变Grafana保存的折叠布局。
function flatten(panels) {
  return panels.flatMap((panel) => [panel, ...flatten(panel.panels ?? [])]);
}

// 模拟Grafana明确使用的格式化变量，保留ID中的引号和URL特殊字符。
function interpolate(query, selected = {}) {
  const vars = { service: 'cses-client-pre', environment: 'pre', DS_PROMETHEUS: 'prometheus', DS_LOKI: 'loki',
    tenant_id: '', user_id: '', channel_id: '', login_attempt_id: '', device_session_id: '', ...selected };
  return query.replace(/\$\{([^}:]+):(doublequote|percentencode)\}/g, (_, name, format) => {
    const value = vars[name] ?? '';
    return format === 'doublequote' ? JSON.stringify(value) : encodeURIComponent(value);
  }).replaceAll('$__rate_interval', '5m').replaceAll('$__interval', '1m').replaceAll('$__range', '30m');
}

const panels = flatten(dashboard.panels);
const targets = panels.flatMap((panel) => (panel.targets ?? []).map((target) => ({ panel, target })));
assert.deepEqual(dashboard, buildDashboard(), '生成物必须与generator/contract逐字结构一致');
assert.equal(dashboard.uid, 'helix-client-channel-v1');
assert.equal(snapshot.families.length, 71);
assert.equal(contract.review, 'APPROVED_ARTIFACT_ONLY');
assert.equal(contract.diagnosticsStatus, 'PLANNED_NO_PRODUCER');
assert.deepEqual(contract.metrics.map(({ name, type }) => ({ name, type })), snapshot.families);
assert.equal(new Set(panels.map((panel) => panel.id)).size, panels.length, 'panel ID唯一');
assert.equal(new Set(contract.plannedLabels.reason).size, contract.plannedLabels.reason.length);
assert.ok(contract.plannedLabels.reason.includes('other'));
for (const field of contract.scopeFields) {
  const variable = dashboard.templating.list.find((item) => item.name === field);
  assert.equal(variable.type, 'textbox');
  assert.equal(variable.current.value, '');
  const link = scopeLink(field);
  assert.ok(link.includes(`var-${field}=\${__value.raw:percentencode}`));
  for (const other of contract.scopeFields.filter((item) => item !== field)) assert.ok(link.includes(`var-${other}=\${${other}:percentencode}`));
  const value = 'user/&?="中';
  const rendered = interpolate(link.replaceAll('${__value.raw:percentencode}', encodeURIComponent(value)), { user_id: 'existing' });
  assert.equal(new URL(rendered, 'http://grafana.invalid').searchParams.get(`var-${field}`), value);
}
for (const panel of panels) {
  assert.ok(!/[A-Za-z_]/.test(panel.title), `面板标题必须使用中文行为描述: ${panel.title}`);
  const { x, y, w, h } = panel.gridPos;
  assert.ok(x >= 0 && y >= 0 && w > 0 && h > 0 && x + w <= 24);
}
for (const { panel, target } of targets) {
  const query = target.expr;
  assert.ok(!/\bor\s+(?:on\([^)]*\)\s+)?vector\(|\babsent(?:_over_time)?\(|\bclamp_min\(/.test(query), '禁止缺数据填0/1或伪造分母');
  assert.ok(!/unwrap\s+\w*seq\b|max\([^)]*seq\b/i.test(query), 'seq不得float化或max伪装最新');
  if (target.datasource.type === 'prometheus') {
    for (const field of contract.scopeFields) assert.ok(query.includes(`${field}=\${${field}:doublequote}`));
    assert.ok(interpolate(query).includes('user_id=""'));
    assert.ok(interpolate(query, { user_id: 'scope-check-nonexistent' }).includes('user_id="scope-check-nonexistent"'));
  } else {
    assert.ok(panel.title.startsWith('[待接入]'));
    assert.ok(query.includes(`service_name="${contract.diagnosticService}"`));
    assert.ok(!query.includes('$__rate_interval'), 'Loki不能使用Prometheus专有rate_interval');
    for (const field of contract.scopeFields) {
      assert.ok(query.includes(`selected_${field}="" or ${field}=\${${field}:doublequote}`));
      assert.ok(query.includes(`label_format selected_${field}=\${${field}:doublequote}`));
    }
    assert.ok(!/^\{[^}]*user_id/.test(query), '用户ID不得进入Loki stream索引');
    const selected = interpolate(query, { user_id: 'user"A' });
    assert.ok(selected.includes('user_id="user\\"A"'), 'ID按字符串安全转义');
  }
}
const ratio = targets.find(({ target }) => target.expr.includes('unwrap committed')).target.expr;
assert.ok(ratio.includes('evidence_complete="true"'));
assert.ok(ratio.includes('matured > 0'));
assert.ok(ratio.includes('by (tenant_id,user_id,channel_id,device_session_id,login_attempt_id,cohort_id)'));
assert.ok(ratio.includes('unwrap committed') && ratio.includes('unwrap matured'));
const checkpoint = panels.find((panel) => panel.targets?.some(target => target.expr.includes('event=~"channel_checkpoint"')));
const groupBy = checkpoint.transformations.find((item) => item.id === 'groupBy');
for (const field of [...contract.scopeFields, 'seq_domain', 'stream_epoch']) assert.equal(groupBy.options.fields[field].operation, 'groupby');
assert.deepEqual(groupBy.options.fields.local_contiguous_seq.aggregations, ['lastNotNull']);
assert.equal(checkpoint.targets[0].maxLines, 1000);
assert.ok(checkpoint.transformations.find((item) => item.id === 'sortBy').options.sort[0].desc === false);
// 精确seq字符串例明确越过float安全边界，不能在前端处理时丢精度。
assert.equal(JSON.parse('{"local_contiguous_seq":"9007199254740993"}').local_contiguous_seq, '9007199254740993');
console.log(`STATIC PASS: ${snapshot.families.length} families, ${panels.length} panels/rows, ${targets.length} queries, scope/URL/cohort/seq guards`);

// 同一时刻只发1个只读请求，避免看板校验压垮共享采集服务。
async function mapBounded(items, handler) {
  let cursor = 0;
  const results = [];
  // 顺序领取请求，服务端限流后也不扩大并发。
  async function worker() {
    while (cursor < items.length) {
      const index = cursor++;
      results[index] = await handler(items[index]);
    }
  }
  await Promise.all(Array.from({ length: 1 }, () => worker()));
  return results;
}

// 验证API接受查询与无身份标签时不回退全局；非空/空序列分别记账。
async function checkLive() {
  const promUrl = process.env.HELIX_PRE_PROMETHEUS_URL ?? 'http://192.168.6.66:30090';
  const lokiUrl = process.env.HELIX_DIAGNOSTIC_LOKI_URL ?? 'http://192.168.6.66:31000';
  const requests = [];
  for (const entry of targets) {
    requests.push({ ...entry, scope: 'global', selected: {} });
    requests.push({ ...entry, scope: 'user', selected: { user_id: 'scope-check-nonexistent' } });
  }
  // 用一个代表性technical query分别验证其他身份过滤维度，不构造真实账号数据。
  const representative = targets.find(({ target }) => target.datasource.type === 'prometheus');
  for (const field of contract.scopeFields.filter((value) => value !== 'user_id')) requests.push({ ...representative, scope: field, selected: { [field]: 'scope-check-nonexistent' } });
  const results = await mapBounded(requests, async ({ panel, target, scope, selected }) => {
    const isProm = target.datasource.type === 'prometheus';
    const endpoint = isProm ? '/api/v1/query' : target.queryType === 'instant' ? '/loki/api/v1/query' : '/loki/api/v1/query_range';
    const url = new URL(endpoint, isProm ? promUrl : lokiUrl);
    url.searchParams.set('query', interpolate(target.expr, selected));
    if (!isProm && target.queryType !== 'instant') {
      url.searchParams.set('limit', '1');
      url.searchParams.set('start', String(BigInt(Date.now() - 30 * 60_000) * 1_000_000n));
      url.searchParams.set('end', String(BigInt(Date.now()) * 1_000_000n));
      url.searchParams.set('step', '60');
    }
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(12_000) });
      const raw = await response.text();
      assert.ok(response.ok, `HTTP ${response.status}: ${raw.slice(0, 150)}`);
      const data = JSON.parse(raw);
      assert.equal(data.status, 'success');
      const count = data.data?.result?.length ?? 0;
      if (isProm && scope !== 'global') assert.equal(count, 0, '身份选择不能回退全局');
      return { panelId: panel.id, refId: target.refId, source: target.datasource.type, scope, status: 'PASS', series: count };
    } catch (error) {
      return { panelId: panel.id, refId: target.refId, source: target.datasource.type, scope, status: 'FAIL', error: String(error.message).slice(0, 300) };
    }
  });
  const evidence = { observedAt: new Date().toISOString(), evidenceLevel: 'QUERY_API_ACCEPTANCE', promUrl, lokiUrl,
    snapshot: contract.snapshot, sourceRevision: snapshot.helix_source_revision, clientSourceRevision: snapshot.client_source_revision,
    selector: snapshot.selector, realBusinessTriggered: false, runtimeRevisionVerified: false, diagnosticsStatus: contract.diagnosticsStatus,
    passed: results.filter((item) => item.status === 'PASS').length, failed: results.filter((item) => item.status === 'FAIL').length, results };
  const output = process.env.HELIX_DASHBOARD_CHECK_OUTPUT ?? '/tmp/helix-client-channel-dashboard-check.json';
  fs.writeFileSync(output, JSON.stringify(evidence, null, 2) + '\n');
  console.log(`LIVE ${evidence.failed ? 'FAIL' : 'PASS'}: ${evidence.passed}/${results.length} query checks; evidence=${output}`);
  for (const result of results.filter((item) => item.status === 'FAIL')) console.error(JSON.stringify(result));
  assert.equal(evidence.failed, 0, '查询校验失败，不可标记看板通过');
}

if (process.argv.includes('--live')) await checkLive();
