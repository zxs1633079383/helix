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
