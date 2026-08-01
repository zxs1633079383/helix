# MessageV3 Phase 1 Announcement Gate 测试域

本目录覆盖 `MV3-G07b`。公告创建或修改只调用公告专用接口；HTTP admission 后由 WS/read-back
形成唯一 Post 事实，禁止额外调用 `posts/create`。

测试必须覆盖专用 HTTP body、零重复 Post、Persist 屏障、sender/receiver 独立投影、重复 WS
幂等和 Storage 失败。公告查询、删除与已读属于后续 Phase，不得借用本 Gate 的证据。
