# PRE cses post 推送诊断交接

更新时间：2026-09-17（Asia/Shanghai）

## 当前阶段

- 状态：`DIAGNOSED`；只读排查已完成，未修改 Kubernetes、Pulsar、代码或数据库。
- 目标环境：PRE；kubeconfig `/Users/xpa-12/.kube/config`，context `kubernetes-admin@kubernetes`，API `https://192.168.6.66:16443`。
- 目标：namespace `cses` 的 `cses-im-server` 收不到部分 post WS 推送。

## 结论

- `cses`、`cses-pre3`、`cses-pre4` 三个实例的 ConfigMap 都是 `imEnv: "dev"`、`discovery: ""`、同一 PRE Pulsar broker、`key_shared`、`postSenderWorkers: 4`。
- 三个实例因此共同消费裸 topic `persistent://public/default/post_sender-1` 的唯一订阅 `post_sender-1`；这只有在三者属于同一逻辑 PRE 工作队列时才合理。
- 最新 Pulsar stats 采样：仅 1 个订阅 `post_sender-1`；`msgInCounter=160`、`msgOutCounter=160`、`backlogSize=0`、`unackedMessages=0`；192 个活跃 consumer，地址对应 `10.200.8.214`（cses）、`10.200.9.217`（cses-pre3）、`10.200.9.180`（cses-pre4），每个 64 个 consumer。
- 这是 Key_Shared 工作队列：一条消息只交给一个 consumer/pod。故 `cses` 不会收到所有 post；被其他 namespace 取走的消息不会在 `cses` backlog 中等待。
- `discovery: ""` 会让代码选择 `ws.NewNoopTransport()`，不注册 `ForwardSenderTopic` 的跨 Pod WS 转发；如果消息由 pre3/pre4 落库，连接在 cses 的客户端不会收到该 WS 推送。
- `cses` Pod 当前 Ready、重启数为 0，未见明显 Pulsar 错误；这不等于具体 post 已完成持久化和 WS 到达。

## 代码语义证据

- `internal/app/cses_post_create.go:95-105`：生产端将 `Message.Key` 设置为 `post.ChannelId`；批量多目标频道会逐条改写目标 `ChannelId`。
- `internal/infra/pulsar/topic.go:17,49-55`：`SenderTopic` 为 `post_sender-1`；`dev/pre/prod` 返回裸 topic。
- `internal/infra/pulsar/broker_consumer.go:47-76`：SenderTopic 的订阅名等于 topic，KeyShared 按 Message.Key 在共享 consumer 之间分发；同一 channel key 保序。
- `internal/consumerapi/consumerapi.go:107-110,152-160`：SenderTopic 使用共享工作队列消费。
- `cmd/server/main.go:318-328`：`discovery` 非空才接入 `ForwardSenderTopic`；为空时使用 noop transport。
- `cmd/server/main.go:330-350`：`ImEnv` 决定 topic 命名，consumer mode 决定订阅类型。

## 下一步（需用户授权的变更）

- 若三者是同一逻辑 PRE 集群：保留同一 SenderTopic 工作队列，但把 `discovery` 配为 PRE 实际服务发现实现，并确保 ForwardSenderTopic 有消费者；所有副本共用同一 topic instance，Pod 的 `nodeId` 必须唯一。
- 若三者是独立部署：将 `cses-pre3/pre4` 从该工作队列移出并路由到独立 topic/namespace；仅把 `imEnv` 改成 `pre` 无效，因为代码对 dev/pre/prod 都使用裸 topic。
- 不要仅为 `cses` 新建独立订阅来“补收”消息：SenderTopic 是落库工作队列，独立订阅可能导致重复持久化；若需求是每个 Pod 都收到一份，应使用明确的广播链路。
- 对某个具体 post 做 first-loss 对账还需要 `channelId`、post/trace/corr 标识和创建时间，逐跳核对 HTTP → Pulsar → consumer → persist → WS。

## 边界

- 本轮未执行 delete、truncate、restart、patch、消费消息或端口转发；“deleting persistent://...”按 topic URI 理解。
