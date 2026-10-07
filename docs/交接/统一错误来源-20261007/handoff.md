# 统一错误来源与热路径优化交接
更新时间：2026-10-07 Asia/Shanghai

## 目标与授权
- 失败保留具体HTTP路径/WS action、来源Command、req_id/corr及原始业务原因。
- 本轮用户要求消除诊断的重复payload解析、成功结果重解析及Effect前缀扫描。
- 本地实现/隔离验证/分类提交已授权；未提版、未发布、未push，未修改真实成员数据。

## 代码身份
- 仓库：/System/Volumes/Data/workspace/angular/visual/helix；分支reactor/MessageV3-helix。
- 功能提交/优化基线：d2bbae39e8f91839792b4b5c42a674fa14b19613。
- 当前优化身份由包含此文件的提交及父提交diff确定；无关未跟踪产物保留未提交。

## 当前设计
- Command入口不读payload；从已有CorrelationContext借用Option<&str>，仅首次绑定时转为owned。
- Arc<Origin>跨Tick共享，Cow静态名称不复制；OnceLock只初始化一次req_id，让早发timer共享身份。
- 内部状态不保存短生命周期借用；长于512字节的外部req_id不复制到诊断索引。
- 业务producer利用已有Value决定观测标记，随DomainEventBytes移动/clone；core只搬运标记，不判断IM。
- 标记不进入wire；普通成功Emit直接跳过诊断JSON解析。HTTP业务解析与render_scope原有校验仍存在。
- EffectSink::visit_mut_from借用[start..]原地访问，不遍历前缀；仍按本轮新增Effect数量线性工作。
- clean_text改单缓冲，在输出错误时清洗命令/req_id；不再在每次Command入口反复构造临时字符串。
- HTTP显式失败在读取data前拒绝；原始原因与领域错误并存，机器码保留并附加errorContext。
- 固定字段的*-failed投影保持合同，来源进入结构化日志；CoreError保留原source链。
- 活跃关联在回包/timer结束时释放；stop/身份切换清空；最近诊断历史最多256条，不恢复业务请求。
- 超出历史范围无法证明WS来源时标server_push；metadata无body/headers，URL不含authority/query。

## Rust API边界
- DomainEventBytes增加私有观测标记，以类型别名和同名函数保留DomainEventBytes(bytes)构造与.0读取。
- 旧单字段tuple模式解构不再兼容；仓内及客户端当前调用检索未发现此用法。未发布，不宣称全生态源级兼容。
- 新增visit_mut_from API；无Effect/Port/WS wire新变体，无DB/schema变更，无core运行时依赖。
- cargo-public-api未安装，按现有gate规则跳过；未手工伪造自动生成的API baseline。

## 可复现性能证据
- /tmp/helix-failure-perf/：Cargo.toml、src/main.rs、src/bin/before.rs与源码基线副本。
- release opt-level=z/LTO/单codegen；锁定依赖，i7-13700K，7轮中位数；对照d2bbae39诊断源码与当前源码，共用当前core隔离组件成本。
- paired-before.txt / paired-after.txt：小Command 1905→97ns，分配14→1；256KiB Command 40325→92ns。
- 成功读Emit 16KiB 5083→22ns，256KiB 106227→33ns；普通Emit约1960→23ns。
- HTTP登记/回包额外分配31→7次（扣除构造本来就有的2次），耗时约6.15→2.64us。
- 1000前置Effect/无新增：359→24ns；空Tick 17→20ns，不宣称所有分支都变快。
- 仅诊断组件微基准，带分配计数开销，未测完整IM/WS/UI或producer分类成本；系统调度有波动。
- Command基准使用静态名称，动态名称仍需要owned复制；不可将上述数字当整条链路加速比。

## 验证
- 性能红测先失败：/tmp/helix-failure-perf/red.log（Command不应重新解析payload）。
- 476项lib+来源集成通过：final-focused.log；最后边界调整复验8项诊断+10项来源：final-{resources,integration}.log。
- 全量IM：1642通过/4失败（full-im.log）；随后仅命名、超长标识边界及日志清洗调整，已聚焦复验。
- 四个失败目标与优化基线一致：artifact_schema、MV3-G01a、MV3-G01f、MV3-G07b，不扩张修复。
- 五crate check、core测试、core wasm check、Web wasm build通过；证据final-check.log/core.log/core-wasm.log/final-wasm.log。
- core-tree.log运行时依赖匹配为空；fmt/diff通过。audit.log的clone告警为Arc/静态Cow元数据共享，无payload深拷贝。
- 上述日志均在/tmp/helix-failure-perf/；上一功能批次30Gate生成/cutover记录在/tmp/helix-failure-context-*。

## 现场与未验证
- PRE 16:54:43，/api/cses/channel/load/incrementByChannelId返回HTTP200+failed/ent: channel_member not found。
- trace18dc336a48c20d0d；query trace18dc336a48be2273；req_id mv3-hydration-1791363283365-ahn2gm9a。
- 真实Go/Native/UI未安装候选验收；没有修复成员缺失本身。后续发布需单独授权。
