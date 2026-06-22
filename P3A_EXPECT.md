# P3a — posts 读族 outbound（worktree-local 预期对账，HX-C012）

> SESSION.md 根文件在并行 worktree 黑名单（P3a∥P3b 同改会冲突）；EXPECT 落本 worktree-local
> 协调文件，main merge 后由主对话汇编进 SESSION §0。

🎯 EXPECT P3a posts 读族 outbound @2026-06-22 +0800 | 赌注 中 | 可逆 是
  ↳ 可证伪预判：
  ① 新建**单一** `crates/helix-im/src/outbound/posts_read.rs`（distributed_slice 独立注册，**零碰** channel*/registry.rs 以外文件；mod.rs 加一行 `pub mod posts_read;`——mod.rs 是 P3a 唯一共享触点，append-only 不改既有行）。
  ② 17 个读命令各一 `OutboundCommand` 实现 + `#[distributed_slice(OUTBOUND_COMMANDS)]` 注册，`build(args)->(path,body)` 锚现网 Go 真源（posts.go/post_bookmark.go/posts.go announcement* 逐字 body）：
     - posts/*（11）：getSchedule(`posts/getSchedule` body`{channelId,userId}`)、postContext(`posts/postContext` `{postId,before}`)、top20(`posts/top20` body **snake_case** `{channel_id}` 陷阱)、get(`posts/get` `{postIds:[]}`)、getPostsAfterIndex(`posts/getPostsAfterIndex` `{postIds:"<单id>"}` 单数 string 陷阱)、getReplies(`posts/getReplies` `{replyId,pageNumber,pageSize,revoke?}`)、getReplyBranch(`posts/getReplyBranch` `{replyFirstLevelId,pageNumber,pageSize}`)、queryTodoList(`posts/queryTodoList` `{postIds:[]}`)、getUpdatedPosts(`posts/getUpdatedPosts` `{timeStamp,limit}` **大写S** 陷阱)、getLatestPost(`posts/getLatestPost` `{channelId,timestamp}`)
     - bookmark(3)：create(`post/bookmark/create` `{channelId,userId,postIds:[]}`)、delete(`post/bookmark/delete` `{userId,postId}`)、load(`post/bookmark/load` `{channelId,userId}`)
     - announcement 读(3)：acceptList(`post/announcement/acceptList` `{postId}`)、list(`post/announcement/list` `{channelId,postId?}`)、detail(`post/announcement/detail` `{postIds:[]}`)
     - post/list 已读位历史读(1，非 bitmap)：postReadList(`post/read/list` `{postIds:[]}`)
  ③ **架构裁定（非偏离，记此供 main 复核）**：读命令在接管模型下是 **request/response**——helix-im 只负责 build 正确 wire body，driver fire HTTP 后把响应**直接回灌给 await 的语义 invoke**（前端纯渲染消费返回值）。`Effect::Http{corr}` 回报后的「emit 投影」属 **WS 推送态**（post:received/channel:update，已在册）+ **driver 投影命令**（store_get_messages，P6 M2 接缝）两条路径，**不**在 P3a 为每个读命令新造 corr→projection 通路（projection-schema.md 尚未冻结 = C2 未到位；且需改 state.rs `CorrelationContext`+module.rs，破「零交集不碰 P3b 文件」）。故 P3a 落 build 层（与 P2 写族同构），response→projection 的字段集断言降级为「build body 字段集锚 Go 真源」断言。
  ④ 新 `tests/p3a_posts_read_test.rs`：17 命令各 1 正常 build 用例 + 三命名陷阱 casing 断言（top20 snake_case `channel_id` / getPostsAfterIndex `postIds` 单数 string / getUpdatedPosts `timeStamp` 大写S）+ 缺必填 Err 不 panic + endpoint 前缀陷阱（announcement/bookmark 走 `post/` 单数 vs posts 读走 `posts/`）+ 注册数==命令数性质（ALL_P3A_READ len == dispatch 集）。casing 真源 = Go struct json tag 逐字（不自造 fixture，C5）。
  ⑤ gate：`cargo test -p helix-im` 全绿（含既有 P2 registry 计数性质——P3a 18 命令并入后 registry 总数变化，既有 p2 `ALL_OUTBOUND` 仅断言其 17 个子集 is_outbound 命中，不断言全表 size，故不破）；`cargo check` 四 crate 过；core 零运行时 grep 空；C3 im/src 无 ambient time；`posts_read.rs` 行+函数覆盖 100%（排除自身 `#[cfg(test)]` 块）；structure-gate PASS（posts_read.rs ≤300）。
  ⑥ BLOCKER 候选：projection-schema.md 未冻结（C2）→ read response→projection emit 通路推迟到 P4/P6；本 Phase 不阻塞（build 层自洽可验）。
