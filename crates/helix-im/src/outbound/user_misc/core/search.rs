//! Search Gate 四件套；共享 helper 负责 public snake_case 到 Go camelCase 的零信任转换。

use super::*;

misc_cmd!(
    SearchPostCommand,
    SEARCH_POST_REG,
    "im_search_post",
    true,
    |args, cmd| { Ok(("search/post", do_search_body(args, cmd)?)) }
);

misc_cmd!(
    SearchUserCommand,
    SEARCH_USER_REG,
    "im_search_user",
    true,
    |args, cmd| { Ok(("search/user", do_search_body(args, cmd)?)) }
);

misc_cmd!(
    SearchChannelCommand,
    SEARCH_CHANNEL_REG,
    "im_search_channel",
    true,
    |args, cmd| { Ok(("search/channel", do_search_body(args, cmd)?)) }
);

// 聚合搜索与其他读族共用 req_id 回灌；Go 返回三段权威 JSON，Helix 不重建 reducer。
misc_cmd!(
    SearchDoCommand,
    SEARCH_DO_REG,
    "im_search_do",
    true,
    |args, cmd| { Ok(("search/do", do_search_body(args, cmd)?)) }
);
