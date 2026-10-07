# 统一错误来源、热路径与 OTel 接入交接
更新时间：2026-10-07 Asia/Shanghai

## 目标与授权
- 失败保留HTTP路径/WS action、来源Command、req_id/corr及原始业务原因。
- 用户要求低侵入接入当前OTel span；前端携带同一TraceId，并完成验证。
- 本地实现/隔离测试/分类提交已授权；未提版、发布或push，未改真实业务数据。

## 代码身份
- 仓库：/System/Volumes/Data/workspace/angular/visual/helix；分支reactor/MessageV3-helix。
- 功能提交d2bbae39e8f91839792b4b5c42a674fa14b19613；优化提交ac378160b3377eee1e949d67dad8aa55296c3313。
- 本次身份由包含此交接的提交与上述父提交diff确定；无关未跟踪文档保留。

## 当前设计
- 诊断不解析Command payload，借用已有CorrelationContext，Arc<Origin>/OnceLock共享身份。
- 成功Emit由私有标记跳过诊断JSON；只访问本轮Effect后缀，最近来源历史上限256。
- HTTP显式业务失败在读取data前拒绝；普通error保留领域错误与原始业务原因。
- core新增纯数据TraceId/StepDiagnostics，由宿主显式注入，不读取ambient OTel或引入SDK。
- TraceId固定32字节Copy；诊断Vec跨step复用，最多16条、每条4096字符；无追踪不收集。
- host TraceScope::step借用执行壳，回收诊断并更新现有core.step和当前Tick根span。
- span设置error.type，既有后台exporter映射OTLP Error和exception.type/message；不新建脱链trace。
- 普通error及机器码errorContext附trace_id；固定*-failed保留原因码，在message附TraceId。
- 成功bytes不变；driver仍原样交付业务bytes，不在dispatch重新解析JSON。
- 每轮/start/stop清空trace上下文，CoreError保留source链；不同WS carrier不串线。
- Native/FFI共享host泵接入；Web完成编译，未新增Web OTel exporter。
- OTel开启时诊断bytes取决于显式TraceId输入；普通step(None)行为兼容。
- 前一批DomainEventBytes旧tuple解构的Rust源级兼容限制仍存在；本批新增纯诊断API及HostSpanSnapshot.failures。

## 本轮验证与证据
- 红：/tmp/helix-otel-red.log，真实IM错误未带当前TraceId，断言失败。
- Host定向：124通过（107库+4场景+13边界），host-final.log；增强同文案断言后4场景再次通过，final-scenarios.log。
- IM诊断9项、来源10项通过：im.log/integration.log；core共26通过：core.log。
- IM全量1644通过/4失败/11忽略：im-full.log；四个失败目标与ac378160前批证据一致。
- 四目标：artifact_schema、MV3-G01a、MV3-G01f、MV3-G07b，不扩张修复。
- Host全量在category_chain_storage遇到2失败；隔离ac378160复验相同失败，host-full.log/host-baseline.log。
- 五crate check、Web wasm build通过：check.log/wasm.log；core-tree.log无运行时依赖。
- release native stress_engine 4项、stress_storage 3项通过：native-regression.log；非生产负载验收。
- rustfmt定向check、git diff --check通过；audit.log既有clone告警人工核对。
- 新增exception消息复制只在失败与exporter后台发生；没有新增成功payload扫描、同步网络或全Effect扫描。
- 上述本轮日志统一前缀/tmp/helix-otel-；临时基线worktree及本轮Collector端口转发已移除。

## 真实 Collector / Jaeger 验证
- 2026-10-07 18:52 CST，候选公开IM Command→合成HTTP200业务失败→真实PRE Collector→Jaeger。
- 测试service=helix-error-verification，deployment.environment=test；未调用真实Go业务接口。
- 客户端错误TraceId=000000000000000018dc39d4ab5393b2；Jaeger省略前导零为18dc39d4ab5393b2。
- 同trace共4 span；port_reply=3d9d9eda31407f4b、core.step=b5ddcb7f43fc605f均error=true/status=ERROR。
- 两span各一条exception，含im_channel_load_increment_by_channel_id、HTTP路径、req_id/corr及ent: channel_member not found，与客户端错误一致。
- live.log和jaeger.json保存导出及MCP同ID查询证据；不是仅凭exporter ready判断成功。
- 查询有child_outside_parent提示：跨Tick延续已结束的command parent，既有异步层级时窗现象；不作耗时关键路径判断。
- 真实导出后仅修正noop测试快照读取语义及增强断言/格式，未改变真实export路径；证据按此diff影响复用。

## 之前的性能/现场证据
- /tmp/helix-failure-perf/paired-{before,after}.txt：诊断组件小Command分配14→1，256KiB读结果约106us→33ns。
- 仅组件微基准，带分配计数，不代表全链加速比；本轮不新增性能数字主张。
- 原PRE失败：HTTP200/failed/ent: channel_member not found，trace18dc336a48c20d0d。
- 未修复成员缺失根因；未安装候选客户端做对话框验收，未发布/push。
