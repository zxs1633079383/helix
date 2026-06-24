# HX-C016 — 真 Go e2e：WS 环境恢复 + 快 harness 模式 + 快 smoke vs 精确 per-UC 取舍

> 状态：active　|　沉淀：2026-06-24（用户明确 + 长会话反复实证）

## 一句话

真 Go e2e 的 **WS hello 依赖 tp/gRPC 隧道·会随时间断**（恢复有固定 playbook）；全 UC e2e **该用单脚本双持久连接 + wait_for（~3min）**而非 LLM-agent workflow（30-60min）；但**单脚本快 smoke 只稳覆盖简单命名命令（~13），精确全 33 仍需 per-UC body 准确**——两者按目的取舍。

## ① WS 环境恢复（「WebSocket 一直断」真因）

- **症状**：`nc 127.0.0.1:8065` REST 可达，但 host-cli 连上 **WS hello 0 字节**（抓不到 `action:"hello"`）→ connectionId 不盖章 → 整条 echo 链死、e2e 第一步 create 就无 `channel_created`。
- **真因**：hello 洪泛依赖 cluster 内 paas/x9 **gRPC（经 telepresence 隧道）**，隧道**随时间断**（实测一个长会话内：早些 hello 正常 → 几小时后 hello 0）。**不是 ping/pong**（host-cli `ping_interval_ms=8000` 心跳正常，比服务端 60s 要求更勤）。
- **恢复 playbook**（用户 2026-06-24 授权自动执行）：
  1. `! /Users/mac28/workspace/java/zlc_ai/GenericAgent/tp-connect.sh`（**sudo·密码 `sh`**·含 route/killall）
  2. 重启 `/workspace/java/cses`
  3. host-cli probe（settle ≥4s 等编译+hello）抓到 `action:"hello"`+`connectionId` → 通
  4. `telepresence status` 看 `Traffic Manager: Connected`

## ② 快 e2e harness 模式（`scripts/host-cli-FULL-uc-e2e.sh`）

慢的真因**不是 e2e**：workflow 把每 phase 做成 LLM agent（读/推理/构造/解析 = 每簇几分钟）+ 盲等 `SETTLE_MS` + 每动作 `cargo run` + 多脚本串行。e2e 本身（HTTP 发 + WS 等回声）秒级。

快模式：
- sender(444)+receiver(678) 各**一条持久 WS**（FIFO held：`exec 8>fifo`/`9>fifo` + sleep holder·全程 1 次 hello）
- 每动作 `feed` 后 `wait_for <action>` / `emit_seen <event>` **命中即返**（轮询 0.2s·不盲等）
- 全 UC 顺序在同连接跑完 → 末尾对累计 LOG 一次性出 PASS/FAIL/SKIP 表
- **一次** `cargo build`
- **依赖 DAG 编排**：`send→wait post→拿 PID`，post 衍生（quickReply/urgent/read/makeTopic/load_older/get_reply_branch/create_posts）全在 **revoke（删帖·终态）之前**；close（毁群）全局最后。

## ③ 快 smoke vs 精确 per-UC（取舍·别混用）

- **快单脚本**：稳覆盖**简单命名命令**（create/send/revoke/nickname/member/schedule/close/在线… ~13），3min 出表·**适合 merge 回归 smoke**（0 FAIL = 核心路径零回归）。
- **够不到全 33**：~20 个 UC 的 body 字段集/期待回声需 **per-UC 精确构造**（registry 真名 + 真 body + 对的 emit/action）。快脚本里猜名/猜体 → SKIP（非回归）。
- **全 33 权威验证**仍靠**每 UC 精确 agent/脚本**（早轮做法·慢但准·台账真源）。
- **坑词典**：① 命令名用 registry 真名别猜（`im_get_reply_branch`≠get_replies·`im_channel_change_info`≠change_channel_info·`im_channel_change_top`·`im_channel_member_role`·转发=`im_create_posts`）② `im_load_older_context` 体必带 `anchor_post_id`+`anchor_create_at`（parse 缺即 Err）③ 状态依赖 UC（4.1 hello增量/4.2 needSync/4.5 陌生channel/2.2 上拉）需**真账号 DB**（隔离空库 0 channel → SKIP）④ self-only 动作（转发/置顶/公告写）无独立 WS 帧 → 走 HTTP200/`im:read:result` 验或双账号广播。

## 关联

- 全局 memory：[[helix-ws-env-recovery]] · [[helix-fast-e2e-harness]] · [[cses-java-verify-recovery-playbook]]
- [[helix-send-echo-not-self-omit]]（send 验证口径）· HX-C008（sync 契约）· HX-C015（完成靠通知非轮询）
