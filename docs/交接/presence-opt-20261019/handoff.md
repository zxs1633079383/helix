# Presence 错峰与退避（2026-10-19）

分支 reactor/MessageV3-helix；不改版本、协议或client依赖，不发布。

## 行为

- 普通状态首次快照仍立即查询；首次定时刷新按宿主auth_user_id与服务端connection_id确定性散列增加0..5000ms相位。后续成功稳定刷新仍30秒，45秒状态新鲜度不变。
- 退避按用户记录连续失败，最早重试门槛为上次发送起30/60/120/240/300秒；定时轮询在原30秒节拍上执行，实际重试还可能有一个节拍内的调度等待。一个失败目标不会拖慢别的目标。
- 网络错误、业务失败、无有效metadata、15秒请求超时均进入退避；成功清除该用户退避。失败不续状态有效期，过期仍unknown。
- 普通滚动补查不能绕过退避。显式切换普通/重点观察沿用原来的即时刷新，清该用户退避；重连也立即重新快照。离开观察、账号清理后不残留重试。
- arm考虑首次轮询截止时间，避免5秒定时器把不同相位取整到同一时刻。
- 不引入随机/时钟依赖，不改core、port、driver或schema，native/FFI/web共用同一逻辑。

## 回归及证据

原始日志在 artifacts/presence-opt-20261019/。公开ImModule::handle测试先红后绿；presence_test共22项通过，覆盖错峰与确定性、稳定周期、逐用户退避恢复、所有失败类别、退避封顶、资源清理以及原有乱序/多scope/权限/焦点闪烁回归。cargo check -p helix-im通过（既有9项警告）。git diff --check通过。

## 范围

没有运行真实Go/新Helix/桌面组合链或容量压测。客户端仍精确依赖registry 0.1.74，未接入本次源码；实机后台验证仅覆盖未改动的UI可见性与旧版已存在的释放逻辑。首个即时快照的集中启动流量不在周期错峰内。

## 联调补充：空观察初始化

真实Native启动先收到空观察，query此前仍初始化next_poll；延迟到首次有目标时，过期的deadline导致连接相位被跳过。公开Tick回归模拟空观察后等待60秒，先红（首次轮询仅5秒）后绿：无目标时query直接返回，不改变30秒稳定周期、错误退避或协议。22项presence_test与cargo check -p helix-im通过（9项已有warnings）。原始记录见artifacts/presence-opt-20261019/empty-phase-{red,green,check}.log。

组合验证与最终候选证据在 /Users/mac17/workspace/front-end/cses-client/docs/交接/presence-opt-20261019/；没有版本或发布动作。

## 2026-10-19 Presence v2 合并订阅（本地实现，未发布）

- Client 已合并滚动订阅；Helix 通过原 presence_watch 发送 version=2、generation、requestId、userIds（最多200）和 focusedUserIds（最多10且属于userIds）。旧协议仍兼容。
- Go 每 Pod 汇总活跃订阅：同 viewer/company/target 的权限结果最多缓存30秒，授权批次100人、4个worker、整轮4秒；Redis按目标去重，重点5秒、普通30秒，读取预算1秒。连接订阅60秒到期，解除观察后缓存清理；没有数据库结构或配置变更。
- 响应 action=presence，包含 version=2、generation、requestId、单调sequence、kind（snapshot/delta/confirm）、statuses、validUntil与checkedAt。首包全量，后续只发变化；续订确认不延长旧授权。即将到期且已经刷新有效期的样本及时确认，避免同状态抑制新有效期。
- validUntil取授权到期与样本新鲜度的较早值；权限查询失败/拒绝/到期输出unknown。Helix基于请求发送时间校准截止时间，不依赖机器时钟一致。断序重新订阅，重连重建，过期unknown；新协议健康时忽略旧协议和在途HTTP结果。
- 健康v2停止普通HTTP查询，每20秒续订，40秒无有效帧后恢复原HTTP错峰退避（还受原查询节拍影响）。没有提高正常刷新频率；后台释放观察沿用Client已验证路径。

### 验证与边界

- Go presence race通过；ws整包100.0%覆盖率、vet/build通过。回归覆盖共享授权、目标去重、重点/普通节拍、权限失败/撤销/续订、队列满、取消与缓存回收。
- Helix presence 25项公开Tick回归通过；native/FFI/web检查、wasm最终构建和core测试通过。helix-im全套剩7项频道查询失败，与此前0.1.75基线名单完全一致；不宣称全套绿色。
- 真实Tauri加载本地Helix，连接本机两份Go；隔离PostgreSQL，PRE身份库只读，Redis使用miniredis测试替身。确认跨进程online/offline、多设备剩一个仍online、最后设备关闭offline；79秒健康窗口0次普通HTTP，撤销权限11266ms变unknown，丢弃v2后46149ms恢复HTTP。不是K8s集群/真实Redis生产压测。
- 证据目录：客户端 artifacts/presence-v2-20261019，Go artifacts/presence-v2-20261019；联合证据 joint-result.json、devices-result.json、events-final.log、wire.jsonl。最终截止时间确认修正另做复验并记录。
- 未改版本，未push或发布；客户端使用本地Helix临时构建，Cargo.toml/lock由原有helper恢复，普通开发二进制hash未变。

最终服务端重建后再次经真实Tauri验证：断线重连、多设备online/offline、70秒健康订阅0次HTTP、新截止时间确认通过，见 v2-corrected-result.json。授权硬截止与5秒调度边界仍可能短暂unknown，下一轮确认恢复；没有延长授权来掩盖过期。


## 2026-10-19 重连和前后台在线图标连续性

同身份的断连/重连不主动清除尚未过期的状态；仍刷新generation、清理旧请求关联、重建订阅，旧帧不能延长状态。观察释放后最多保留200份原有效期内的warm cache；主动观察优先占用额度，闲置项按原截止时间清除。无观察时只有到期定时器，不发HTTP、WS查询或续订。切账号/租户、shutdown、明确撤销仍清除，后台不无限保绿。

消费侧MessagePresenceService把重报订阅与身份清除分开，保留有界已接受用户用于投影，以覆盖visibilitychange早于IntersectionObserver的顺序；Helix绝对快照缺失/unknown仍清灰。相同映射不更新Signal；指令可见性只控制观察，不把有效状态转成unknown，presenceEnabled继续控制展示。

公开seam：im_presence_observe是远端读意图；成功由im:presence:update返回，失败不延长已有期限，generation/corr隔离迟到结果；Persist/远端写为N/A。无新命令、wire字段、Port、依赖或配置；三端共享业务状态机保持sans-IO。

验证：新增重连有效期、后台无网络与过期、cache上限与租户隔离回归；旧代码2项失败后修复，最终28项presence测试通过，cargo check -p helix-im通过（9项既有warning）。客户端20项服务、1项指令DOM、6项Host合同共27项通过，边界/type/lint及前端/本地Helix原生候选构建通过。未重跑无关全套IM/三端driver测试；未改port/driver/schema。

真实macOS Tauri候选加载当前Helix与当前前端，使用真实本机双Go、Redis/Pulsar、隔离PostgreSQL和只读PRE身份库。通过本机真实HTTP建私聊，原生pointer点击私聊，代理断开真实WS（wire generation已变化），原生hide/show；MutationObserver跟踪现有私聊行data-line-status/class。点击、重连、短暂隐藏恢复、22秒授权续期均无非online变更；隐藏32秒后unknown且0次HTTP查询，恢复重新online，对端实际断开后offline。见 flicker-dom-result.json；原始DOM/network/provenance/screenshot在cses-client/artifacts/presence-flicker-20261019。不是K8s容量、Windows或多窗口验收；发现注册仍用本地fixture。未提版、push或发布。
