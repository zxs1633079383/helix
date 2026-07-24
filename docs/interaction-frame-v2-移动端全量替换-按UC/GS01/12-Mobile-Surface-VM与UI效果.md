# GS01 Mobile Surface、VM 与 UI 效果

| Surface | VM | Venus/QuickJS 目标效果 |
|---|---|---|
| `message.left-dialog-list`、`message.unread-summary` | DialogListVm + UnreadSummaryVm | 连接完成后，会话列表与未读摘要一次出现；连接受理不能显示为空列表成功 |

## 原子性

由同一 authority fact 影响的已 attach Surface 必须在一个 Frame 内 stage 并 swap。任何 mutation
校验、frame-cache 写入或 directive 前置条件失败，整帧不提交。

## 平台责任

- C++：校验、原子 cache、pending overlay、ack/resync。
- QuickJS：按 renderer key 机械读取完整 VM。
- Venus 页面：具体布局、焦点、滚动和动画。
- Helix：事实、排序、权限、ActionBinding、完整 VM 与 directive。

真实产品页面路径当前为 `SOURCE_MISSING`；文档给出的 UI 效果是 V2 验收合同，不是已完成声明。
