#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = path.dirname(fileURLToPath(import.meta.url));
export const contract = JSON.parse(fs.readFileSync(path.join(dir, 'client-channel-dashboard.contract.json'), 'utf8'));
export const outputPath = path.join(dir, 'client-channel-observability.json');
const prom = { type: 'prometheus', uid: '${DS_PROMETHEUS}' };
const loki = { type: 'loki', uid: '${DS_LOKI}' };

// 只对当前无身份标签的技术family开放全局查询；身份选择非空时不借全局值补齐。
export function technicalSelector(job = 'opentelemetry-collector') {
  const identity = contract.scopeFields.map((field) => `${field}=\${${field}:doublequote}`).join(',');
  return `service_name=\${service:doublequote},deployment_environment=\${environment:doublequote},job="${job}",${identity}`;
}

// 在解析后的JSON字段上精确筛选；空字符串才代表该维度全部，ID不进入stream索引。
export function diagnosticPipeline() {
  let query = `{service_name="${contract.diagnosticService}",deployment_environment=\${environment:doublequote}} | json | __error__=""`;
  for (const field of contract.scopeFields) {
    query += ` | label_format selected_${field}=\${${field}:doublequote} | selected_${field}="" or ${field}=\${${field}:doublequote}`;
  }
  return query;
}

// 每个ID单元格可选择该主体，保留其他scope与时间；对URL值显式编码。
export function scopeLink(selectedField) {
  const values = [];
  for (const field of contract.scopeFields) {
    values.push(`var-${field}=${field === selectedField ? '${__value.raw:percentencode}' : `\${${field}:percentencode}`}`);
  }
  for (const field of ['environment', 'service', 'DS_PROMETHEUS', 'DS_LOKI']) values.push(`var-${field}=\${${field}:percentencode}`);
  return `/d/${contract.uid}?${values.join('&')}&\${__url_time_range}`;
}

// 以确定性布局生成独立看板，不读写任何已有Dashboard。
export function buildDashboard() {
  let id = 1;
  let y = 0;
  const panels = [];
  const dq = diagnosticPipeline();

  // 面板基线统一保留No data语义，不给未接入业务染成成功。
  function base(title, type, datasource, height = 8) {
    const panel = { id: id++, title, type, gridPos: { x: 0, y, w: 24, h: height },
      fieldConfig: { defaults: { noValue: '无数据 / 待接入', color: { mode: 'palette-classic' } }, overrides: [] }, targets: [] };
    if (datasource) panel.datasource = datasource;
    y += height;
    return panel;
  }

  // 文本说明随Dashboard导入，清楚标出当前可用范围和待实现前置。
  function note(title, content, height = 5) {
    const panel = base(title, 'text', undefined, height);
    panel.options = { mode: 'markdown', content };
    panels.push(panel);
  }

  // 对日志表附加可点击的用户/群/登录/设备精确下钻，不执行授权逻辑。
  function addLinks(panel) {
    for (const field of contract.scopeFields) {
      panel.fieldConfig.overrides.push({ matcher: { id: 'byName', options: field }, properties: [
        { id: 'links', value: [{ title: `只看此 ${field}`, url: scopeLink(field), targetBlank: false }] },
        { id: 'unit', value: 'string' },
      ] });
    }
  }

  // 表格从JSON保留精确字符串，不把seq经float转换；所有表均受1000行窗口限制。
  function logTable(title, events, fields, description, checkpoint = false) {
    const panel = base(`[待接入] ${title}`, 'table', loki, 10);
    panel.description = `${description} 来源为拟新增专用诊断JSON；最多${contract.maxLogLines}行，空数据不表示零故障。`;
    panel.targets = [{ refId: 'A', datasource: loki, expr: `${dq} | event=~"${events.join('|')}"`, queryType: 'range', maxLines: contract.maxLogLines }];
    panel.transformations = [{ id: 'extractFields', options: { source: 'Line', format: 'json', replace: true, keepTime: true } }];
    if (checkpoint) {
      // 多设备/登录/seq域/epoch单独分组，仅取窗口中最后观察，禁止max(seq)。
      const groupFields = {};
      for (const field of [...contract.scopeFields, 'seq_domain', 'stream_epoch']) groupFields[field] = { aggregations: [], operation: 'groupby' };
      for (const field of fields) if (!groupFields[field]) groupFields[field] = { aggregations: ['lastNotNull'], operation: 'aggregate' };
      panel.transformations.push({ id: 'sortBy', options: { sort: [{ field: 'Time', desc: false }] } }, { id: 'groupBy', options: { fields: groupFields } });
    } else {
      panel.transformations.push({ id: 'filterFieldsByName', options: { include: { names: ['Time', ...contract.scopeFields, ...fields] } } });
      panel.transformations.push({ id: 'sortBy', options: { sort: [{ field: 'Time', desc: true }] } });
    }
    panel.options = { showHeader: true, cellHeight: 'sm', footer: { show: false } };
    addLinks(panel);
    panel.fieldConfig.overrides.push({ matcher: { id: 'byRegexp', options: '.*(seq|ranges|checkpoint_version).*' }, properties: [{ id: 'unit', value: 'string' }] });
    panels.push(panel);
  }

  // 诊断记录数只作为观测量，不把日志重试次数称为唯一消息或gap episode数量。
  function logCount(title, event, group) {
    const panel = base(`[待接入] ${title}`, 'timeseries', loki);
    panel.description = '诊断记录/秒；需去重才能解释为唯一业务事件数。当前producer未接入，不填零。';
    panel.targets = [{ refId: 'A', datasource: loki, expr: `sum by (${group}) (rate(${dq} | event="${event}" [$__interval]))`, queryType: 'range', legendFormat: `{{${group}}}` }];
    panel.fieldConfig.defaults.unit = 'ops';
    panel.interval = '1m';
    panels.push(panel);
  }

  note('范围与数据可用性', `默认身份留空=当前授权范围全局；输入准确用户/群聊/登录/设备ID，或点击表格ID下钻。\n\n技术指标来源：2026-09-07 PRE快照71族；无身份标签，选择身份后技术面板为空。业务诊断依赖尚未接入的 \`${contract.diagnosticService}\` JSON流，不能将空值解释为0%/100%。\n\n现有Grafana 12.2.1、Loki 2.9.3；原生OTLP Logs接入须处理Loki升级/兼容适配。变量不是租户授权，数据源必须限制可读范围。`, 7);
  logTable('每次登录与少群定位（点击 user_id 选择用户）', ['login_started', 'login_sync_terminal'],
    ['event', 'result', 'reason', 'expected_known', 'membership_revision', 'expected_channels', 'returned_channels', 'persisted_channels', 'projected_channels', 'applied_channels', 'filtered_channels', 'missing_count', 'elapsed_seconds', 'evidence_complete', 'dropped_records'],
    '同一membership快照下逐阶段集合差；expected_known=false时完整性未知，认证前user_id可能为空。');
  logTable('每轮离线/重连同步结果', ['sync_started', 'sync_terminal'],
    ['event', 'sync_session_id', 'trigger', 'result', 'reason', 'pages', 'events_received', 'events_committed', 'deduplicated', 'redacted', 'from_seq', 'next_seq', 'horizon_seq', 'elapsed_seconds', 'evidence_complete'],
    '同轮、同设备的分页和终态；成功HTTP不等于同步完成。');
  logCount('接收→落库→应用的诊断记录速率', 'delivery_stage', 'stage');

  // 同一完整关闭cohort的最后一份不可变摘要去重后相除；不存在/零分母不补值。
  const cohortFilter = `${dq} | event="delivery_cohort_closed" | evidence_complete="true" | matured > 0`;
  const cohortKey = 'tenant_id,user_id,channel_id,device_session_id,login_attempt_id,cohort_id';
  const numerator = `sum(last_over_time(${cohortFilter} | unwrap committed | __error__="" [$__range]) by (${cohortKey}))`;
  const denominator = `sum(last_over_time(${cohortFilter} | unwrap matured | __error__="" [$__range]) by (${cohortKey}))`;
  const ratio = base('[待接入] 已闭合且证据完整 cohort 的接收后落库率', 'stat', loki, 5);
  ratio.description = '按关闭时间窗统计 committed/(committed+failed+timeout)，同cohort去重。pending/unknown/partial和观测丢弃单列；不代表服务端audience端到端送达率。';
  ratio.targets = [{ refId: 'A', datasource: loki, expr: `(${numerator} / ${denominator}) and (${denominator} > 0)`, queryType: 'instant' }];
  ratio.fieldConfig.defaults.unit = 'percentunit';
  ratio.options = { reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false }, colorMode: 'none', graphMode: 'none' };
  panels.push(ratio);
  logTable('落库 cohort：成功、失败、pending 与 unknown', ['delivery_cohort_closed'],
    ['cohort_id', 'path', 'matured', 'committed', 'failed', 'timeout', 'pending', 'unknown', 'evidence_complete', 'dropped_records'],
    '接受前取消排除；接受后取消保留pending；中断不能恢复为unknown。unknown/partial不混进成功率。');
  const gapCount = base('[待接入] 窗口内Gap episode数（按gap_id去重）', 'stat', loki, 5);
  gapCount.description = '仅统计窗口内有gap_opened记录且gap_id非空的episode；不等于真实丢失数，也不等于全部仍未关闭Gap。观测丢弃时是不完整证据。';
  gapCount.targets = [{ refId: 'A', datasource: loki, expr: `count(count by (tenant_id,user_id,channel_id,device_session_id,seq_domain,stream_epoch,gap_id) (count_over_time(${dq} | event="gap_opened" | gap_id!="" [$__range])))`, queryType: 'instant' }];
  gapCount.options = { reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false }, colorMode: 'none', graphMode: 'none' };
  panels.push(gapCount);
  logCount('Gap建立与终态的诊断记录速率', 'gap_opened', 'seq_domain');
  logTable('Gap具体区间与补拉结论', ['gap_opened', 'gap_updated', 'gap_repair_result', 'gap_terminal'],
    ['event', 'gap_id', 'seq_domain', 'stream_epoch', 'expected_seq', 'received_seq', 'missing_ranges', 'local_contiguous_seq', 'observed_high_seq', 'server_horizon_seq', 'result', 'reason', 'elapsed_seconds', 'evidence_complete'],
    '跳号仅是疑似缺口；分清重排/重复/合法不可见/裁剪/提交失败/证据不足。精确seq保留字符串。');
  logTable('各群最后观测seq（窗口内，不保证全量当前态）', ['channel_checkpoint'],
    ['observed_at', 'checkpoint_version', 'local_contiguous_seq', 'local_persisted_high_seq', 'observed_high_seq', 'server_horizon_seq', 'requested_from_seq', 'response_next_seq', 'pending_ranges', 'evidence_complete'],
    '按设备/登录/群/seq域/epoch分组取窗口内最后观察。时钟偏差、迟到版本及窗口外频道需latest-state服务权威对账；不能用max(seq)。', true);
  note('全局技术指标（以下分组可展开）', '下方71个族来自实测快照，Counter使用rate，Histogram使用rate(bucket)分位数，Gauge保持当前值。所有身份过滤非空时不回退全局。技术指标不能单独证明当前用户送达或SQLite业务完整性。');

  const groups = new Map();
  for (const metric of contract.metrics) {
    if (!groups.has(metric.group)) groups.set(metric.group, []);
    groups.get(metric.group).push(metric);
  }
  for (const [name, metrics] of groups) {
    const row = { id: id++, type: 'row', title: `${name}（${metrics.length}个实测族）`, collapsed: true, gridPos: { x: 0, y: y++, w: 24, h: 1 }, panels: [] };
    for (const metric of metrics) {
      const panel = base(`全局 · ${metric.title}`, 'timeseries', prom);
      panel.description = `实测族：${metric.name}；类型：${metric.type}；范围仅全局。身份筛选非空时无身份维度。快照不证明本轮业务被触发。`;
      panel.fieldConfig.defaults.noValue = '无数据 / 所选身份无技术指标维度';
      panel.options = { legend: { displayMode: 'table', placement: 'bottom', calcs: ['lastNotNull', 'max'] }, tooltip: { mode: 'multi', sort: 'desc' } };
      const selector = technicalSelector();
      if (metric.type === 'histogram') {
        for (const [index, quantile] of [0.5, 0.95, 0.99].entries()) panel.targets.push({ refId: String.fromCharCode(65 + index), datasource: prom, expr: `histogram_quantile(${quantile}, sum by (le) (rate(${metric.name}_bucket{${selector}}[$__rate_interval])))`, legendFormat: `P${quantile * 100}` });
        panel.fieldConfig.defaults.unit = metric.name.endsWith('_seconds') ? 's' : metric.name.endsWith('_bytes') ? 'bytes' : 'short';
      } else if (metric.type === 'counter') {
        panel.targets = [{ refId: 'A', datasource: prom, expr: `sum(rate(${metric.name}{${selector}}[$__rate_interval]))`, legendFormat: '每秒增量' }];
        panel.fieldConfig.defaults.unit = 'ops';
      } else {
        panel.targets = [{ refId: 'A', datasource: prom, expr: `${metric.name}{${selector}}`, legendFormat: '{{stage}} {{operation}} {{pool}} {{tick_kind}}' }];
        panel.fieldConfig.defaults.unit = metric.name.endsWith('_ratio') ? 'percentunit' : 'short';
      }
      row.panels.push(panel);
    }
    panels.push(row);
  }
  note('Trace派生指标边界', 'Collector和Jaeger各有调用Counter与耗时Histogram。这是Span观测，受采样/重试/配置影响，两个管道不能相加作为业务消息数量。');
  for (const metric of contract.traceMetrics) {
    const panel = base(`全局 · ${metric.job} · ${metric.name}`, 'timeseries', prom);
    panel.description = 'Span派生，非业务送达。当前4个族缺deployment_environment；严格环境筛选下为空，不能撤掉过滤后把跨环境Span当本环境。选择身份时也不回退全局。';
    panel.fieldConfig.defaults.noValue = '缺环境/身份维度，或无数据';
    panel.targets = [{ refId: 'A', datasource: prom, expr: metric.type === 'counter'
      ? `sum(rate(${metric.name}{${technicalSelector(metric.job)}}[$__rate_interval]))`
      : `histogram_quantile(0.95, sum by (le) (rate(${metric.name}_bucket{${technicalSelector(metric.job)}}[$__rate_interval])))` }];
    panel.fieldConfig.defaults.unit = metric.type === 'counter' ? 'ops' : 'ms';
    panels.push(panel);
  }
  const variables = [
    { name: 'DS_PROMETHEUS', label: 'Prometheus', type: 'datasource', query: 'prometheus', current: { text: contract.defaults.prometheusUid, value: contract.defaults.prometheusUid }, refresh: 1 },
    { name: 'DS_LOKI', label: 'Loki诊断数据源', type: 'datasource', query: 'loki', current: { text: contract.defaults.lokiUid, value: contract.defaults.lokiUid }, refresh: 1 },
    { name: 'environment', label: '环境', type: 'custom', query: 'pre,dev,prod,local,test', current: { text: 'pre', value: 'pre' } },
    { name: 'service', label: '技术指标服务', type: 'textbox', query: contract.defaults.service, current: { text: contract.defaults.service, value: contract.defaults.service } },
  ];
  const labels = { tenant_id: '租户ID', user_id: '用户ID', channel_id: '群聊ID', login_attempt_id: '登录ID', device_session_id: '设备会话ID' };
  for (const field of contract.scopeFields) variables.push({ name: field, label: `${labels[field]}（空=全部）`, type: 'textbox', query: '', current: { text: '', value: '' }, skipUrlSync: false });
  return { id: null, uid: contract.uid, title: contract.title, description: '实测技术指标与拟接入的用户/群聊诊断分开；无数据不伪装成功。', tags: ['Helix', 'MessageV3', 'Metrics', '用户诊断'],
    editable: true, schemaVersion: 39, version: 1, timezone: 'Asia/Shanghai', time: { from: 'now-30m', to: 'now' }, refresh: '30s',
    templating: { list: variables }, annotations: { list: [] }, panels };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  fs.writeFileSync(outputPath, JSON.stringify(buildDashboard(), null, 2) + '\n');
  console.log(`生成 ${path.basename(outputPath)}：${contract.metrics.length} 个实测family，诊断producer状态=${contract.diagnosticsStatus}`);
}
