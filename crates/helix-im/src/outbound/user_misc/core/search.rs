//! search 四件套；body 由共享 helper 对齐 Go 的 DoSearchRequest。

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

// Go 尚未实现聚合搜索，保留 fire-and-forget 标记，待服务端补齐后再切读回灌。
misc_cmd!(
    SearchDoCommand,
    SEARCH_DO_REG,
    "im_search_do",
    false,
    |args, cmd| { Ok(("search/do", do_search_body(args, cmd)?)) }
);
