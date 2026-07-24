# G04 InteractionFrameV2 完整数据合同

字段真源复用 [PC G04 Frame 合同](../../interaction-frame-v2-全量替换-按UC/G04/07-Helix命令与Frame合同.md)。Mobile 必须接收整帧，不消费半个 VM。

```json
{
  "schemaVersion": 2,
  "transactionId": "<helix-owned>",
  "causationId": "<requestId-or-authority-fact>",
  "runtime": {
    "connection": "connecting-or-online-or-offline",
    "initialSync": "idle-or-syncing-or-ready-or-failed",
    "recovery": "idle-or-syncing-or-recovered-or-failed-or-blocked"
  },
  "mutations": [
    {
      "surface": "message.timeline",
      "scope": {"kind":"timeline","channelId":"channel-1","windowToken":"window-1"},
      "surfaceEpoch": 42,
      "revision": 8,
      "baseRevision": null,
      "causationId": "<matching-action-or-fact>",
      "mutation": {
        "kind": "timelineSnapshot",
        "view": "<TimelineWindowVm + ThreadDetailVm + DialogListVm>"
      }
    }
  ],
  "directives": [
    {
      "directiveId": "<helix-owned-nonempty-id>",
      "kind": "<closed-semantic-directive-kind>"
    }
  ]
}
```

## 当前 Mobile 解码差距

`CONTRACT_DRIFT`：当前 C++ decoder 仍只允许 Frame 根字段 `schemaVersion/transactionId/mutations`，
且 mutation 不接受 `surfaceEpoch`，会拒绝上面的正式 V2 envelope。证据见
[SurfaceRuntime.cpp](/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp:1695)。
首轮纯 V2 candidate 前必须先更新 C++ type/decoder/fixture；禁止删字段去迎合当前 consumer。

具体 scope、mutation discriminant、VM 字段与 directive 值必须从固定 candidate 的生成合同取得；
未在共享合同证明的值标记 `TARGET_DECISION_REQUIRED`，不得由 Mobile 自行扩展。
