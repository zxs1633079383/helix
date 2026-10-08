# 统一错误来源、OTel与错误文本预算交接
更新时间：2026-10-08 Asia/Shanghai

## 目标与授权
- 失败保留HTTP路径/WS action、来源Command、req_id/corr、原始原因与当前TraceId。
- 本轮优化错误文本共享与字节预算，成功业务路径保持现状；说明客户端和Trace表现。
- 用户已追加授权提交远端并发布版本；目标四公共crate 0.1.47，完成版本提交/注释Tag/推送。
- 发布前plan通过，合并远端纯文档310c9536，SESSION冲突保留双方记录；验证沿用代码身份5eb8fa03。

## 代码身份
- 仓库：/System/Volumes/Data/workspace/angular/visual/helix；分支reactor/MessageV3-helix。
- 父提交f096993172ce520d492fb10ee19c3ac961d1ef6d；本轮由包含此交接的提交及其diff确定。
- 历史来源修复d2bbae39e8f91839792b4b5c42a674fa14b19613；成功路径优化ac378160b3377eee1e949d67dad8aa55296c3313。
- 本批仅core diagnostics、host otel/exporter/hooks及对应测试、SESSION/本交接；无关未跟踪文件保留。

## 当前设计
- 来源诊断借用CorrelationContext，Arc/OnceLock共享身份；成功Emit不解析JSON，只处理新增Effect后缀。
- HTTP业务失败先于data校验；普通error含来源与原始原因，机器码errorContext另附详情。
- core只收显式TraceId/诊断，无ambient OTel、SDK或运行时依赖；TraceId为Copy。
- FailureDiagnostic私有Arc<str>，入口一次UTF-8安全截断，随后step/root/SDK共享文本。
- 单条与单step/span文本合计均≤4096字节，最多16事件；FailureBuffer内部维护预算。
- 不扫描完整文本，UTF-8边界最多回退3字节；诊断Vec跨step复用，无追踪不收集。
- 超限仍保留Error，省略计数映射OTLP dropped_events_count；客户端业务错误不受此预算截断。
- 8192槽队列错误文本逻辑上限32MiB，另有在途批次；不含其他属性/容器/编码开销。
- HostSpanSnapshot.failures变为FailureBuffer；FailureDiagnostic.message变私有，使用构造器/借用访问器。
- 上述Rust源码API变更未发布；业务JSON/HTTP/WS协议不变。
- Native/FFI复用host，Web编译通过但没有新增Web OTel exporter；每轮清理Trace防串线。

## 本轮验证（日志统一前缀 /tmp/helix-otel-share-）
- red.log：实现前UTF-8字节/整批预算红测；随后core-final.log共29通过/1忽略。
- host-final.log：108库+13边界通过；scenarios.log最终5场景通过/1真实导出默认忽略。
- 覆盖Arc指针在step/root/SDK一致、Unicode边界、100错误只留4条并上报96省略。
- check.log五crate check、wasm.log Web wasm build通过；core-tree.log无运行时依赖。
- im-full.log：1644通过/4失败/11忽略，仍为历史artifact_schema/G01a/G01f/G07b。
- Host历史category_chain_storage两失败已在ac378隔离复现，见/tmp/helix-otel-host-baseline.log；不扩张修复。
- 定向rustfmt与diff --check通过；audit.log的clone告警均在既有属性/上下文代码，本批文本为Arc共享。
- 未改engine/storage，复用前批release native stress_engine 4项、stress_storage 3项的范围证据。

## 性能证据
- 同机i7-13700K/x86_64、rustc1.95、release opt-level=z/LTO、分配计数开启。
- /tmp/helix-failure-perf源码；before-host旧二进制来自f096，本轮host为候选代码。
- bench-before/after及其-repeat.txt：正反顺序各一次，每场景7轮中位数，预热1000次。
- 4KiB错误span创建/记录/析构：7.84–8.08us→1.15–1.19us；约15.06→12.06次分配。
- 该场景含noop测试快照，错误原文预建，不含入口构造、网络与OTLP编码；非整条IM链耗时。
- 4KiB共享clone约11ns/0分配；入口构造约79–97ns/1分配，后续不随文本长度拷贝。
- 成功step始终0分配；StepDiagnostics为预算计数增加16栈字节，TraceScope增加32字节。
- bench-success.txt：旧ac378诊断与候选同binary交替11轮，小Command 87→83ns/1→1分配；
  256KiB成功19.6→20.2ns/0→0；1000前置Effect 28→21ns/0→0；ns差异视为噪声。
- 不把以上数字与历史机器/轮次的1.90us、106us等直接相减或宣称全链加速。

## 真实导出与客户端边界
- 2026-10-08 11:31 CST：候选公开IM Command→合成HTTP200业务失败→真实PRE Collector→Jaeger。
- service=helix-error-verification，environment=test；未调用真实Go业务接口或升级客户端。
- TraceId=000000000000000018dc705d16c4f212；Jaeger省略前导零为18dc705d16c4f212。
- 4 span中port_reply(d42beb550af6aed7)、core.step(dc68a6876f230ff1)均Error；
  exception.type=im.operation.failed，message含Command、HTTP路径、req_id/corr、member缺失原因及同TraceId。
- live.log与jaeger.json为同轮真实导出/查询证据；PRE monitoring Collector UID仍2ff070cc-9dca-4e14-be92-e3ced3bce82b。
- 本轮端口转发已关闭；child_outside_parent为跨Tick延续已结束parent，不能作关键路径耗时依据。
- 客户端源码helixTauriImGate.service.ts:2549包装为HELIX_HYDRATION_FAILED，message.component.ts:1050以externalMessageJump报告。
- 普通error可见完整TraceId；机器码错误另有errorContext，但当前客户端此处只读取error，可能只显示机器码。
- 弹窗仅源码推导，未安装候选做UI验收；未修复原PRE成员缺失业务根因，未发布/push。
