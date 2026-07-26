# G-15 Cross-repo Review

- Verdict: PASS
- Behavior: PASS
- Code boundary: PASS
- Frame V2: PASS
- Evidence completeness: PASS
- Required fail: 0
- Required skip: 0

Helix 是 `im_make_topic`、持久化屏障、HTTP/WS 双 correlation 与四份 canonical
Frame 的唯一业务真源。PC 仅执行 AcceptedCommand、bus replay/真实 ingress 与 DOM
渲染；Mobile 仅执行生产 SDK admission、FFI DomainEvent batch、SurfaceRuntime 原子
apply 与 QuickJS typed view。三仓 mirror 的五份文件 byte-exact 相同。

Mobile shared real-chain 的 G15a fresh-group 前置已通过，但 Mobile G15 不声明 real
lane；其 required 证据是链接生产 SDK/Bridge/SurfaceRuntime/QuickJS 的 Contract
GTest。G15 的 required real lane 由 PC single-real 与 dual-real 闭合。
