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
export function diagnosticPipeline(scopeFields = contract.scopeFields, includeRuntimeGaps = false) {
  let query = `{service_name="${contract.diagnosticService}",deployment_environment=\${environment:doublequote}} | json | __error__=""`;
  for (const field of scopeFields) {
    query += ` | label_format selected_${field}=\${${field}:doublequote} | selected_${field}="" or ${field}=\${${field}:doublequote}`;
    if (includeRuntimeGaps) query += ' or coverage_scope="producer_runtime"';
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
  const fieldLabels = {producer_service:'产出服务',batch_id:'服务端推送批次',inventory_snapshot_id:'服务端清单记录',inventory_scope:'清单范围',inventory_filter:'清单筛选方式',inventory_since:'查询起始时间',selected_channels:'本次查询选中群数',selected_set_hash:'查询群集合摘要',returned_set_hash:'返回群集合摘要',snapshot_complete:'清单记录是否完整',filter_reason:'筛除原因',omitted_records:'未逐条记录数量',Time:'记录时间',tenant_id:'租户',user_id:'用户',channel_id:'群聊',login_attempt_id:'本次登录',device_session_id:'设备会话',event:'发生的事情',stage:'进行到哪一步',result:'处理结果',reason:'原因',count:'本次数量',expected_known:'是否知道应有群数',membership_revision:'成员清单版本',expected_channels:'应有群数',returned_channels:'服务端返回群数',persisted_channels:'本机保存群数',projected_channels:'准备展示群数',applied_channels:'页面接收群数',filtered_channels:'被筛除群数',missing_count:'缺少群数',elapsed_seconds:'耗时（秒）',evidence_complete:'记录是否完整',dropped_records:'采集丢弃记录数',sync_session_id:'本轮同步',trigger:'同步原因',pages:'分页数',events_received:'收到事件数',events_committed:'保存事件数',events_returned:'返回事件数',sync_kind:'同步类型',requested_from_seq:'请求起始序号',response_next_seq:'响应下次序号',has_more:'是否还有下一页',deduplicated:'重复排除数',redacted:'不可见事件数',from_seq:'起始序号',next_seq:'下次序号',horizon_seq:'服务端最新序号',cohort_id:'本次统计窗口',path:'到达方式',matured:'已到确认期限',committed:'确认保存成功',failed:'保存失败',timeout:'超过确认期限',pending:'仍待确认',unknown:'结果未知',observed:'实际观察事件数',gap_id:'本次序号缺口',seq_domain:'序号类型',stream_epoch:'序号所属批次',expected_seq:'原本应收到的序号',received_seq:'实际收到的序号',declared_from_seq:'服务端声明缺口起点',declared_to_seq:'服务端声明缺口终点',missing_ranges:'缺少序号范围',local_contiguous_seq:'本机连续保存到',observed_high_seq:'本机观察到的最高序号',server_horizon_seq:'服务端最新序号',observed_at:'实际发生时间',checkpoint_version:'状态记录版本',local_persisted_high_seq:'本机已保存的最高序号',pending_ranges:'仍待补齐范围',evidence_scope:'统计范围',message_id:'消息标识',business_event_id:'业务事件标识',trace_id:'调用追踪标识',event_seq:'本次事件序号'};
  const values = {server_committed:'服务端已提交',audience_resolved:'服务端已确定接收范围',server_enqueued:'服务端已入发送队列',socket_written:'服务端已写入连接',ws_received:'已从连接收到',persist_terminal:'本机保存返回结果',client_received:'页面收到事件',client_applied:'页面已处理事件',success:'成功',failed:'失败',received:'收到',already_committed:'此前已保存',live_ws:'在线推送',stream_seq:'群内事件序号',legacy_event_seq:'兼容事件序号',channel_cursor:'本机连续序号',client_persistence:'本机接收到保存',true:'是',false:'否',login_started:'开始登录',login_sync_started:'开始恢复群聊',login_sync_terminal:'本次登录同步结束',runtime_stopped:'结束本次运行',delivery_cohort_closed:'本次保存统计已结束',diagnostic_coverage_gap:'部分过程未完整记录',late_terminal:'超过期限后返回保存结果',gap_opened:'发现序号缺口',gap_updated:'缺口范围变化',gap_terminal:'本次缺口处理结束',channel_inventory_snapshot_started:'开始查询群聊清单',channel_inventory_item:'服务端处理一个群聊',channel_inventory_snapshot_closed:'本次群聊清单已结束',channel_inventory_item_received:'本机收到群聊资料',channel_inventory_end_received:'本机收到批次结束',channel_inventory_item_committed:'本机已保存群聊资料',channel_checkpoint:'更新群聊保存位置',channel_inventory_page_returned:'服务端返回群聊清单',channel_inventory_persisted:'本机保存群聊清单',channel_inventory_applied:'页面接收群聊清单',sync_started:'开始补齐消息',sync_terminal:'本轮补齐结束',sync_page_returned:'服务端返回一页消息',sync_page_received:'本机收到一页消息',sync_persist_terminal:'本页消息保存结束',gap_observed:'观察到序号缺口',gap_repair_result:'返回缺口补齐结果',started:'开始',store:'读取本机缓存',persist:'保存到本机',publish_submitted:'已提交推送',connect:'建立连接',timeout:'超过确认期限',cancelled:'已取消',completed:'已完成',stopped:'已停止',initial_connection_failed:'首次连接失败',capacity_limit:'采集容量不足',identity_or_sequence_missing:'缺少身份或事件序号',terminal_result_unknown:'无法识别保存结果',retired_sequence_ambiguous:'旧序号再次到达，无法确认是否重复',outside_observed_deadline:'超时或未观察到对应接收记录'};

  const enumKeys = {
    stage: ['server_committed','audience_resolved','server_enqueued','socket_written','ws_received','persist_terminal','client_received','client_applied','store','persist','publish_submitted','connect'],
    result: ['success','failed','received','already_committed','started','timeout','cancelled','completed','stopped'],
    reason: ['initial_connection_failed','capacity_limit','identity_or_sequence_missing','terminal_result_unknown','retired_sequence_ambiguous','outside_observed_deadline'],
    path: ['live_ws'], seq_domain: ['stream_seq','legacy_event_seq','channel_cursor'], evidence_scope: ['client_persistence'],
    evidence_complete: ['true','false'], expected_known: ['true','false'], has_more: ['true','false'],
  };

  // 面板基线统一保留No data语义，不给未接入业务染成成功。
  function base(title, type, datasource, height = 8) {
    const panel = { id: id++, title, type, gridPos: { x: 0, y, w: 24, h: height },
      fieldConfig: { defaults: { noValue: '当前筛选时段没有记录', color: { mode: 'palette-classic' } }, overrides: [] }, targets: [] };
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

  // 表格从JSON保留精确字符串，不把seq经float转换；所有表均受1000行窗口限制。
  function logTable(title, events, fields, description, checkpoint = false, loginScope = false) {
    fields = [...new Set(['producer_service', ...fields])];
    const panel = base(title, 'table', loki, 10);
    panel.description = `${description} 来源为专用业务诊断JSON；最多${contract.maxLogLines}行，空数据不表示零故障。`;
    panel.targets = [{ refId: 'A', datasource: loki, expr: `${loginScope ? diagnosticPipeline(contract.scopeFields.filter(field => field !== 'channel_id')) : dq} | event=~"${events.join('|')}"`, queryType: 'range', maxLines: contract.maxLogLines }];
    panel.fieldConfig.defaults.noValue = '未提供';
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
    // 身份可能缺失，表格不为缺值生成会清空筛选条件的链接；统一使用顶部精确筛选。
    for (const field of new Set(['Time', ...contract.scopeFields, ...fields])) {
      const properties = [{id:'displayName',value:fieldLabels[field] ?? field}];
      if (['event','stage','result','reason','path','seq_domain','evidence_scope','evidence_complete','expected_known','has_more'].includes(field)) properties.push({id:'mappings',value:[{type:'value',options:Object.fromEntries((field === 'event' ? events : enumKeys[field] ?? []).filter(key => values[key]).map(key=>[key,{text:values[key]}]))}]});
      panel.fieldConfig.overrides.push({matcher:{id:'byName',options:checkpoint && ![...contract.scopeFields,'seq_domain','stream_epoch'].includes(field) ? `${field} (lastNotNull)` : field},properties});
    }
    panel.fieldConfig.overrides.push({ matcher: { id: 'byRegexp', options: '.*(seq|ranges|checkpoint_version).*' }, properties: [{ id: 'unit', value: 'string' }] });
    panels.push(panel);
  }

  // 诊断记录数只作为观测量，不把日志重试次数称为唯一消息或gap episode数量。
  function logCount(title, event, group) {
    const panel = base(title, 'timeseries', loki);
    panel.description = '诊断记录/秒；需去重才能解释为唯一业务事件数。无记录不填零；本地真实业务与PRE按环境隔离。';
    panel.targets = [{ refId: 'A', datasource: loki, expr: `sum by (${group}) (rate(${dq} | event="${event}" [$__interval]))`, queryType: 'range', legendFormat: `{{${group}}}` }];
    panel.fieldConfig.defaults.unit = 'ops';
    for (const [key, label] of Object.entries(values)) panel.fieldConfig.overrides.push({matcher:{id:'byName',options:key},properties:[{id:'displayName',value:label}]});
    panel.interval = '1m';
    panels.push(panel);
  }

  note('范围与数据可用性', `默认身份留空=当前授权范围全局；输入准确用户/群聊/登录/设备ID，在顶部筛选对应的记录。\n\n技术指标来源：71族技术指标，无业务身份标签，选择身份后技术面板为空。业务诊断使用 \`${contract.diagnosticService}\` JSON流。2026-09-08 Go PRE发布 c391ddcf 已验证提交与推送诊断；独立 Native 验证服务 helix-pre-verification / helix-pre-verification-v2 已完成在线保存及离线重登补齐；helix-pre-verification-final 为最终累计快照与自观测修复验证服务，已完成真实保存和导出中断恢复复验。该证据不代表当前客户端 Angular 登录收尾或渲染已验收；选择对应服务与时间窗查看。不能将空值解释为0%/100%。\n\n现有Grafana 12.2.1、Loki 2.9.3；客户端经有界后台直接写入日志；服务端经专用日志采集规则接入。变量不是租户授权，数据源必须限制可读范围。`, 7);
  logTable('登录后群聊是否完整', ['login_started', 'login_sync_started', 'login_sync_terminal', 'runtime_stopped', 'channel_inventory_page_returned', 'channel_inventory_persisted', 'channel_inventory_applied', 'channel_inventory_snapshot_started', 'channel_inventory_snapshot_closed', 'channel_inventory_end_received'],
    ['event', 'stage', 'result', 'reason', 'batch_id', 'inventory_snapshot_id', 'inventory_scope', 'inventory_filter', 'inventory_since', 'selected_channels', 'selected_set_hash', 'returned_set_hash', 'snapshot_complete', 'omitted_records', 'count', 'expected_known', 'membership_revision', 'expected_channels', 'returned_channels', 'persisted_channels', 'projected_channels', 'applied_channels', 'filtered_channels', 'missing_count', 'elapsed_seconds', 'evidence_complete', 'dropped_records'],
    '比较同一成员清单的返回、保存与展示结果；未知应有群数时无法判断完整。本区忽略群聊筛选，避免选中一个群后隐藏整次登录。', false, true);
  logTable('某个群聊查到了吗，收到并保存了吗', ['channel_inventory_item','channel_inventory_item_received','channel_inventory_item_committed'],
    ['event','batch_id','inventory_snapshot_id','result','filter_reason','trace_id','evidence_complete','dropped_records'],
    '服务端清单记录只证明该次请求选中的群聊。以原推送批次对照接收和保存，不把增量查询当作账号应有全部群；超过逐条记录上限时，需同时查看清单摘要中的遗漏数量。');
  logTable('重新登录或恢复连接后，消息是否补齐', ['sync_started', 'sync_terminal', 'sync_page_returned', 'sync_page_received', 'sync_persist_terminal'],
    ['event', 'sync_session_id', 'trigger', 'result', 'reason', 'pages', 'events_received', 'events_committed', 'count', 'events_returned', 'sync_kind', 'requested_from_seq', 'response_next_seq', 'has_more', 'deduplicated', 'redacted', 'from_seq', 'next_seq', 'horizon_seq', 'elapsed_seconds', 'evidence_complete'],
    '同轮、同设备的分页和终态；成功HTTP不等于同步完成。');
  logCount('收到的消息走到了哪一步', 'delivery_stage', 'stage');
  logTable('某条消息收到了吗，保存结果是什么', ['delivery_stage', 'late_terminal', 'diagnostic_coverage_gap'],
    ['stage','result','reason','message_id','business_event_id','seq_domain','event_seq','trace_id','evidence_complete','dropped_records'],
    '按用户、群聊、登录和设备联查逐条记录。调用追踪标识来自真实传播载体；缺失表示本条无法关联追踪，不补造标识。');

  // 同一完整关闭cohort的最后一份不可变摘要去重后相除；不存在/零分母不补值。
  const cohortFilter = `${dq} | event="delivery_cohort_closed" | evidence_complete="true" | matured > 0`;
  const cohortKey = 'tenant_id,user_id,channel_id,device_session_id,login_attempt_id,cohort_id';
  const numerator = `sum(last_over_time(${cohortFilter} | unwrap committed | __error__="" [$__range]) by (${cohortKey}))`;
  const denominator = `sum(last_over_time(${cohortFilter} | unwrap matured | __error__="" [$__range]) by (${cohortKey}))`;
  const incomplete = `sum(count_over_time(${dq} | event="delivery_cohort_closed" | evidence_complete!="true" [$__range])) > 0`;
  const gaps = `sum(count_over_time(${diagnosticPipeline(contract.scopeFields, true)} | event=~"diagnostic_coverage_gap|diagnostic_truncated" [$__range])) > 0`;
  const dropped = `sum(count_over_time(${dq} | dropped_records > 0 [$__range])) > 0`;
  const ratio = base('收到的事件有多少在三十秒内确认保存', 'stat', loki, 5);
  ratio.description = '只统计已关闭的本机观察窗口，每个事件去重。所选范围任一统计不完整、采集丢弃或覆盖缺口都会隐藏百分比。无法归属用户的实例级采集丢弃会保守隐藏同环境比例。超时只表示三十秒内未确认，不等于永久丢失；本值不代表服务端全部应收事件或页面展示成功率。';
  ratio.targets = [{ refId: 'A', datasource: loki, expr: `((${numerator} / ${denominator}) and (${denominator} > 0)) unless (${incomplete}) unless (${gaps}) unless (${dropped})`, queryType: 'instant' }];
  ratio.fieldConfig.defaults.unit = 'percentunit';
  ratio.options = { reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false }, colorMode: 'none', graphMode: 'none' };
  panels.push(ratio);
  logTable('消息保存成功、失败和仍待确认的情况', ['delivery_cohort_closed'],
    ['cohort_id', 'evidence_scope', 'path', 'observed', 'matured', 'committed', 'failed', 'timeout', 'pending', 'unknown', 'evidence_complete', 'dropped_records'],
    '同一窗口按事件去重；进程正常关闭时未确认的事件记为结果未知。崩溃前尚未关闭的窗口无法由本表证明完整。超过期限不等于永久丢失。');
  const gapCount = base('发现了多少次消息序号缺口', 'stat', loki, 5);
  gapCount.description = '仅统计窗口内有gap_opened记录且gap_id非空的episode；不等于真实丢失数，也不等于全部仍未关闭Gap。观测丢弃时是不完整证据。';
  gapCount.targets = [{ refId: 'A', datasource: loki, expr: `count(count by (tenant_id,user_id,channel_id,device_session_id,login_attempt_id,seq_domain,stream_epoch,gap_id) (count_over_time(${dq} | event="gap_opened" | gap_id!="" [$__range])))`, queryType: 'instant' }];
  gapCount.options = { reduceOptions: { calcs: ['lastNotNull'], fields: '', values: false }, colorMode: 'none', graphMode: 'none' };
  panels.push(gapCount);
  logCount('什么时候发现消息序号缺口', 'gap_opened', 'seq_domain');
  logTable('哪些消息序号不连续，后来是否补齐', ['gap_opened', 'gap_updated', 'gap_observed', 'gap_repair_result', 'gap_terminal'],
    ['event', 'gap_id', 'seq_domain', 'stream_epoch', 'expected_seq', 'received_seq', 'declared_from_seq', 'declared_to_seq', 'missing_ranges', 'local_contiguous_seq', 'observed_high_seq', 'server_horizon_seq', 'result', 'reason', 'elapsed_seconds', 'evidence_complete'],
    '跳号仅是疑似缺口；分清重排/重复/合法不可见/裁剪/提交失败/证据不足。精确seq保留字符串。');
  logTable('每个群的消息已保存到哪个序号', ['channel_checkpoint'],
    ['observed_at', 'checkpoint_version', 'local_contiguous_seq', 'local_persisted_high_seq', 'observed_high_seq', 'server_horizon_seq', 'requested_from_seq', 'response_next_seq', 'pending_ranges', 'evidence_complete'],
    '按设备/登录/群/seq域/epoch分组取窗口内最后观察。时钟偏差、迟到版本及窗口外频道需latest-state服务权威对账；不能用max(seq)。', true);
  note('全局运行情况：收发、保存和页面更新', '下方71个族来自实测快照，Counter使用rate，Histogram使用rate(bucket)分位数，Gauge保持当前值。所有身份过滤非空时不回退全局。技术指标不能单独证明当前用户送达或SQLite业务完整性。');

  const groups = new Map();
  for (const metric of contract.metrics) {
    if (!groups.has(metric.group)) groups.set(metric.group, []);
    groups.get(metric.group).push(metric);
  }
  for (const [name, metrics] of groups) {
    const row = { id: id++, type: 'row', title: `${name}`, collapsed: true, gridPos: { x: 0, y: y++, w: 24, h: 1 }, panels: [] };
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
  note('操作过程记录能说明哪些情况', 'Collector和Jaeger各有调用Counter与耗时Histogram。这是Span观测，受采样/重试/配置影响，两个管道不能相加作为业务消息数量。');
  for (const metric of contract.traceMetrics) {
    const panel = base(`全局 · ${metric.type === 'counter' ? '记录了多少次操作调用' : '操作调用耗时多久'}（${metric.job === 'jaeger' ? '追踪服务' : '采集服务'}）`, 'timeseries', prom);
    panel.description = 'Span派生，非业务送达。当前4个族缺deployment_environment；严格环境筛选下为空，不能撤掉过滤后把跨环境Span当本环境。选择身份时也不回退全局。';
    panel.fieldConfig.defaults.noValue = '缺环境/身份维度，或无数据';
    panel.targets = [{ refId: 'A', datasource: prom, expr: metric.type === 'counter'
      ? `sum(rate(${metric.name}{${technicalSelector(metric.job)}}[$__rate_interval]))`
      : `histogram_quantile(0.95, sum by (le) (rate(${metric.name}_bucket{${technicalSelector(metric.job)}}[$__rate_interval])))` }];
    panel.fieldConfig.defaults.unit = metric.type === 'counter' ? 'ops' : 'ms';
    panels.push(panel);
  }
  const variables = [
    { name: 'DS_PROMETHEUS', label: '运行指标数据源', type: 'datasource', query: 'prometheus', current: { text: contract.defaults.prometheusUid, value: contract.defaults.prometheusUid }, refresh: 1 },
    { name: 'DS_LOKI', label: '业务明细数据源', type: 'datasource', query: 'loki', current: { text: contract.defaults.lokiUid, value: contract.defaults.lokiUid }, refresh: 1 },
    { name: 'environment', label: '环境', type: 'custom', query: 'pre,dev,prod,local,test,diagnostic-local', current: { text: 'pre', value: 'pre' } },
    { name: 'service', label: '技术指标服务', type: 'textbox', query: contract.defaults.service, current: { text: contract.defaults.service, value: contract.defaults.service } },
  ];
  const labels = { tenant_id: '租户ID', user_id: '用户ID', channel_id: '群聊ID', login_attempt_id: '登录ID', device_session_id: '设备会话ID' };
  for (const field of contract.scopeFields) variables.push({ name: field, label: `${labels[field]}（空=全部）`, type: 'textbox', query: '', current: { text: '', value: '' }, skipUrlSync: false });
  return { id: null, uid: contract.uid, title: contract.title, description: '按用户与群聊查看真实收发、保存和同步事实；本地验证与PRE部署状态分开。', tags: ['Helix', 'MessageV3', 'Metrics', '用户诊断'],
    editable: true, schemaVersion: 39, version: 1, timezone: 'Asia/Shanghai', time: { from: 'now-30m', to: 'now' }, refresh: '30s',
    templating: { list: variables }, annotations: { list: [] }, panels };
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  fs.writeFileSync(outputPath, JSON.stringify(buildDashboard(), null, 2) + '\n');
  console.log(`生成 ${path.basename(outputPath)}：${contract.metrics.length} 个实测family，诊断producer状态=${contract.diagnosticsStatus}`);
}
