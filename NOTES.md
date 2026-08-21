# 教学备注

- 用户是 Rust 初学者，希望边读当前 Helix 源码边学，不希望先从抽象语法课开始。
- 讲解顺序：先给一张链路地图，再按 `Tick → Effect → PortReply → Persist → Emit` 逐跳读。
- 必须明确区分 `sending`、HTTP admission、WS echo、数据库 `sent` 和 `im:post:received`。
- 每节课保留少量检索练习；用户回答后才把内容提升为 learning record。
