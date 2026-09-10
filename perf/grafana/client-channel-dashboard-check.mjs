#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { buildDashboard, contract, outputPath, scopeLink, technicalSelector, syncDisplayProjection } from './generate-client-channel-dashboard.mjs';

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
  return query.replace(/\$\{([^}:]+):(doublequote|percentencode|regex)\}/g, (_, name, format) => {
    const value = vars[name] ?? '';
    if (format === 'regex') return JSON.stringify(value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).slice(1, -1);
    return format === 'doublequote' ? JSON.stringify(value) : encodeURIComponent(value);
  }).replaceAll('$__rate_interval', '5m').replaceAll('$__interval', '1m').replaceAll('$__range', '30m');
}

const panels = flatten(dashboard.panels);
const targets = panels.flatMap((panel) => (panel.targets ?? []).map((target) => ({ panel, target })));
assert.deepEqual(dashboard, buildDashboard(), '生成物必须与generator/contract逐字结构一致');
assert.equal(dashboard.uid, 'helix-client-channel-v1');
assert.equal(snapshot.families.length, 71);
assert.equal(contract.review, 'APPROVED_ARTIFACT_ONLY');
assert.equal(contract.diagnosticsStatus, 'PRE_GO_NATIVE_VERIFIED_CLIENT_TERMINAL_PENDING');
const correctedTypes = new Map(contract.typeCorrections.map(item => [item.name, item]));
assert.deepEqual(contract.metrics.map(({name,type})=>({name,type})), snapshot.families.map(({name,type})=>{
  const correction=correctedTypes.get(name);
  if(correction) assert.equal(type,correction.from);
  return {name,type:correction?.to??type};
}));
assert.equal(contract.baselinePanels.total, 111);
for(let id=1;id<=111;id++) assert.ok(panels.some(panel=>panel.id===id), `原面板 ${id} 不得删除`);
for (const service of ['', 'cses-client-dev', 'client.v1+(demo)']) {
  const query=interpolate(technicalSelector(),{service});
  const pattern=JSON.parse(query.match(/service_name=~("(?:[^"\\]|\\.)*")/)[1]);
  const matcher=new RegExp(pattern);
  for (const actual of ['cses-client-dev','cses-client-dev-extra','cses-client-pre','client.v1+(demo)','clientXv1demo']) {
    assert.equal(matcher.test(actual), service===''||actual===service, '服务空值全选、非空精确且转义');
  }
}
const syncDetail=panels.find(panel=>panel.id===4);
assert.equal(syncDetail.targets.length,3,'分离服务端/客户端/整轮，避免共享1000行截断');
assert.ok(syncDetail.targets.every(target=>target.maxLines===1000));
assert.ok(syncDetail.targets.some(target=>target.expr.includes('sync_recovery_terminal')));
assert.ok(syncDetail.targets.some(target=>target.expr.includes('sync_server_page_terminal')));
assert.ok(syncDisplayProjection(['count','from_seq','elapsed_seconds']).includes('.events_returned'));
assert.ok(!syncDisplayProjection(['count']).includes('default "0"'));
assert.equal(panels.find(panel=>panel.id===112).targets[0].queryType,'instant');
assert.equal(new Set(panels.map((panel) => panel.id)).size, panels.length, 'panel ID唯一');
for(const panel of panels.filter(panel=>panel.type==='table')) assert.equal(panel.options.enablePagination,true,'明细必须分页，避免大量行遮挡其他面板');
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
    assert.ok(panel.description, '业务面板必须说明事实与口径');
    assert.ok(query.includes(`service_name="${contract.diagnosticService}"`));
    assert.ok(!query.includes('$__rate_interval'), 'Loki不能使用Prometheus专有rate_interval');
    for (const field of contract.scopeFields) {
      if (['登录后群聊是否完整','整轮同步开始和真正完成','清单各页开始和落库完成'].includes(panel.title) && field === 'channel_id') {
        assert.ok(!query.includes('selected_channel_id'), '登录总览不受单群筛选误伤');
        continue;
      }
      assert.ok(query.includes(`selected_${field}="" or ${field}=\${${field}:doublequote}`));
      assert.ok(query.includes(`label_format selected_${field}=\${${field}:doublequote}`));
    }
    assert.ok(!/^\{[^}]*user_id/.test(query), '用户ID不得进入Loki stream索引');
    const selected = interpolate(query, { user_id: 'user"A' });
    assert.ok(selected.includes('user_id="user\\"A"'), 'ID按字符串安全转义');
  }
}
// 双端指标必须分别使用各自真实资源标签；详情时间不成为Prom标签。
for (const metric of contract.syncMetrics) {
  const entry=targets.find(({target})=>target.expr.includes(metric.name));
  assert.ok(entry,`缺少同步指标 ${metric.name}`);
  if(metric.owner==='helix') assert.ok(metric.name.startsWith('otel_helix_'),'匹配实测Prometheus暴露名');
  if(metric.owner==='go') {
    assert.ok(entry.target.expr.includes('environment=${environment:doublequote}'));
    assert.ok(entry.target.expr.includes('job="cses-im-server"'));
    assert.ok(!entry.target.expr.includes('service_name='));
  }
  assert.ok(!/page_index=|sync_session_id=/.test(entry.target.expr));
}
const ratio = targets.find(({ target }) => target.expr.includes('unwrap committed')).target.expr;
assert.ok(ratio.includes('evidence_complete="true"'));
assert.ok(ratio.includes('matured > 0'));
assert.ok(ratio.includes('by (tenant_id,user_id,channel_id,device_session_id,login_attempt_id,cohort_id)'));
assert.ok(ratio.includes('unwrap committed') && ratio.includes('unwrap matured'));
assert.ok(ratio.includes('unless') && ratio.includes('evidence_complete!="true"') && ratio.includes('diagnostic_coverage_gap') && ratio.includes('dropped_records > 0'), '不能只对幸存完整样本显示百分比');
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
    if (process.argv.includes('--loki-only') && entry.target.datasource.type !== 'loki') continue;
    requests.push({ ...entry, scope: 'global', selected: {} });
    requests.push({ ...entry, scope: 'user', selected: { user_id: 'scope-check-nonexistent' } });
  }
  // 用一个代表性technical query分别验证其他身份过滤维度，不构造真实账号数据。
  const representative = targets.find(({ target }) => target.datasource.type === 'prometheus');
  if (!process.argv.includes('--loki-only')) for (const field of contract.scopeFields.filter((value) => value !== 'user_id')) requests.push({ ...representative, scope: field, selected: { [field]: 'scope-check-nonexistent' } });
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

// 同一Span被两个Collector connector接收，面板必须选择唯一主链。
for (const {target} of targets.filter(({target}) => target.expr.includes('otel_traces_span_metrics_'))) assert.ok(target.expr.includes('collector_instance_id!=""'));
// 短启动过程保留首个累计样本，最近时间按所选时段取实际最大值。
for (const id of [104,105]) assert.ok(panels.find(panel=>panel.id===id).targets[0].expr.includes('max_over_time('));
for (const id of [106,107]) {
  const panel=panels.find(panel=>panel.id===id);
  assert.ok(panel.title.includes('进程累计'));
  assert.ok(!panel.targets[0].expr.includes('rate('));
}
if (process.argv.includes('--live')) await checkLive();
