# G16/G16b/G16c Implementation Handoff

## 状态

`PARTIAL - dedicated wire/UI guards ready; five terminal chains pending shared integration`

## 已完成

- `im_channel_member_role` 在 HTTP 前拒绝非 `CREATOR|ADMIN|MEMBER` 领域角色。
- 新增 channel-management UI，close/self-leave/member/role/settings 为不同 intent；version conflict 保留旧 Frame 并提示 hydrate。
- focused Rust contract 分列 close、self leave、member remove、role、settings 五种 canonical wire。

## 证据

- Angular：`evidence/angular-focused.log`，2/2 PASS。
- Rust：focused final 尚待 root 在 shared 编译修复后聚合重跑。

## 未决

- shared 五种 ActionDispatch、对应 corr/authority/PersistAtomic、viewer-specific terminal Frame 和 apply 后 navigation directive。
- 新组件尚需 shell owner 接入现有 group-settings drawer；不能以 standalone component 冒充 runtime-wired。
