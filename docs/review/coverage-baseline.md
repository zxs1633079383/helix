# 覆盖率基线（im-takeover P0 锁定）

> 工具 cargo-llvm-cov · crate helix-im · `--no-default-features`
> 生成 `scripts/coverage.sh --baseline`；快照时刻 commit `57afc1a` @ 2026-06-22 18:57:00 +0800

## 基线语义（只增不减）

- 本文件 = sync-v2 ~90% baseline 锁定时的全仓 helix-im 覆盖率快照。
- **后续 Phase 判据**：本 Phase 新增文件跑 100% lines+functions（`scripts/coverage.sh --phase <file>`）；
  全仓总覆盖率**只增不减**，不得因新增未覆盖代码回退。
- 度量命令：`cargo llvm-cov --no-default-features -p helix-im --summary-only`。

## 快照（`--summary-only`）

```
Filename                                          Regions    Missed Regions     Cover   Functions  Missed Functions  Executed       Lines      Missed Lines     Cover    Branches   Missed Branches     Cover
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
acl/from_tick.rs                                       14                 6    57.14%           1                 0   100.00%          11                 3    72.73%           0                 0         -
acl/to_effect.rs                                      541                44    91.87%          32                 4    87.50%         418                42    89.95%           0                 0         -
acl/to_effect_s1.rs                                   151                 0   100.00%          13                 0   100.00%         111                 0   100.00%           0                 0         -
base64.rs                                             142                 3    97.89%          10                 0   100.00%          69                 3    95.65%           0                 0         -
channel.rs                                            339                54    84.07%          20                 3    85.00%         223                30    86.55%           0                 0         -
channel_write/members.rs                              245                21    91.43%          22                 1    95.45%         135                 8    94.07%           0                 0         -
channel_write/patch.rs                                146                 2    98.63%          11                 0   100.00%          71                 0   100.00%           0                 0         -
channel_write/post.rs                                 233                13    94.42%          20                 2    90.00%         149                11    92.62%           0                 0         -
channel_write/program.rs                              258                 0   100.00%          10                 0   100.00%         199                 0   100.00%           0                 0         -
commands.rs                                           184                14    92.39%           8                 1    87.50%         114                 2    98.25%           0                 0         -
module.rs                                            1079               237    78.04%          67                19    71.64%         763               155    79.69%           0                 0         -
parser.rs                                             463                57    87.69%          65                14    78.46%         309                43    86.08%           0                 0         -
pending_send.rs                                        46                 0   100.00%           3                 0   100.00%          61                 0   100.00%           0                 0         -
state.rs                                              192                 9    95.31%          25                 3    88.00%         132                 9    93.18%           0                 0         -
sync_session.rs                                        38                 4    89.47%           5                 0   100.00%          38                 3    92.11%           0                 0         -
ws/context.rs                                           9                 0   100.00%           2                 0   100.00%          18                 0   100.00%           0                 0         -
ws/frame.rs                                           187                13    93.05%          29                 4    86.21%         131                 8    93.89%           0                 0         -
ws/handlers/channel_close.rs                           38                 6    84.21%           4                 2    50.00%          33                 4    87.88%           0                 0         -
ws/handlers/channel_created.rs                         28                 1    96.43%           2                 0   100.00%          22                 1    95.45%           0                 0         -
ws/handlers/channel_member_update.rs                   77                 4    94.81%           4                 0   100.00%          57                 2    96.49%           0                 0         -
ws/handlers/gate.rs                                    39                 6    84.62%           2                 0   100.00%          30                 4    86.67%           0                 0         -
ws/handlers/hello.rs                                   46                 1    97.83%           4                 0   100.00%          37                 1    97.30%           0                 0         -
ws/handlers/ignored.rs                                  6                 0   100.00%           2                 0   100.00%          11                 0   100.00%           0                 0         -
ws/handlers/increment_channel.rs                       79                 5    93.67%           4                 0   100.00%          58                 3    94.83%           0                 0         -
ws/handlers/increment_channel_end.rs                  127                 6    95.28%           9                 0   100.00%          90                 2    97.78%           0                 0         -
ws/handlers/legacy_event.rs                            24                 7    70.83%           2                 0   100.00%          20                 5    75.00%           0                 0         -
ws/handlers/post.rs                                   199                54    72.86%          16                 9    43.75%         135                37    72.59%           0                 0         -
ws/handlers/post_read.rs                               52                 8    84.62%           6                 2    66.67%          48                 5    89.58%           0                 0         -
ws/handlers/post_schedule_canceled.rs                  28                 4    85.71%           3                 1    66.67%          24                 3    87.50%           0                 0         -
ws/handlers/post_schedule_created.rs                   28                 4    85.71%           3                 1    66.67%          24                 3    87.50%           0                 0         -
ws/handlers/post_update.rs                             43                 4    90.70%           5                 1    80.00%          44                 3    93.18%           0                 0         -
ws/handlers/posted.rs                                 118                 7    94.07%          11                 1    90.91%          89                 2    97.75%           0                 0         -
ws/handlers/posts_update.rs                            79                15    81.01%           6                 3    50.00%          53                11    79.25%           0                 0         -
ws/handlers/update_channel.rs                          30                27    10.00%           4                 3    25.00%          26                23    11.54%           0                 0         -
ws/handlers/update_channel_member_nickname.rs          50                 6    88.00%           6                 2    66.67%          38                 4    89.47%           0                 0         -
ws/registry.rs                                        959                37    96.14%          60                10    83.33%         549                31    94.35%           0                 0         -
-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------
TOTAL                                                6317               679    89.25%         496                86    82.66%        4340               461    89.38%           0                 0         -
```
