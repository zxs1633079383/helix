# 回应取消与HTTP失败回传（2026-10-06）

## 范围

G-06完整post_update携带quickReply=[]时，edit_content_op显式patch quick_reply列，保持缺失字段不覆盖。quickReply沿用既有req_id/OutboundReadReply，HTTP业务FAIL或HTTP错误返回im:read:result.error；成功回包不提前伪造post_updated，业务事实仍在PersistOk后投影。无新Command、表、schema、版本或依赖。

## 证据

- G06新增空数组原子patch/PersistOk投影、HTTP业务失败回传红绿回归；最终G06六项、registry两项通过。
- cargo check -p helix-im通过。
- 30 Gate生成全部通过，产物在/tmp/reaction-30-gates；冻结schema校验全部通过；cutover inventory=30通过。消费者参考资产未同步，不作为发布已完成。
- helix-im宽测在artifact_schema之前658项通过；artifact_schema九项通过、一项存量失败：Phase1 MV3-G03a inbound-contract出现unexpected go字段。未修改该无关资产，也未宣称整仓绿色。
- 最终Rust文件格式检查与diff检查通过；整仓fmt存量driver-host差异未扩大修复。
- 客户端真实Tauri当前HTTP回应成功、即时显示通过，但PRE消息回显未确认；实机业务链路未验收通过。OTLP DNS用户确认已知。见客户端docs/交接/reaction-feedback-20261006.md。

## 交付

本仓实现可独立定向验收，按根AGENTS自动提交授权创建本地提交；不push、不发布、不升级版本。暂无新增Harness卡/索引；G06回归补充现有合同生命周期证据。

## 后续空回应分流 EXPECT

- 真实取消帧含 quickReply=[] 与 expediteMap={} 时，空回应必须进入完整编辑原子写，不被加急稀疏提交吞掉。
- 投票/平均分完整快照与不含 quickReply 的加急更新保持现有路径。先运行 G06 红测，再做最小修正、定向绿测及真实 Tauri 验收。

分流修正：明确 quickReply=[] 优先走完整 post edit 原子提交，保留该快照中的加急/模板字段。G06实际handler红测已失败（empty authority must clear reaction column）；修复后 G06 6、G07 4、post_update 单测3全部通过，cargo check通过（存量warning保留）。当前为局部业务分流修复，不新增协议、port或driver，不声明全量/三端通过；真实Tauri重建与确认验收继续中。
