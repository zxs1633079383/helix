# 分片上传与断点续传交接
## 目标与授权
- Java/Helix/客户端支持<=1GiB；保持入口、稳定文件引用、消息Persist/WS成功屏障；TDD、开发分支worktree、分类本地提交，禁止push/发布。
- 最新参数：>100MiB自动分片，<=100MiB简单PUT；8MiB/片、单文件3槽、host池最多6槽；合同见contract.md。
- 消息目标：5000人/群、5消息RPS/群、多群并行、1秒可达；本任务没有容量达标证据。
## 代码身份
- Helix：/Users/xpa-12/.codex/worktrees/multipart-1gib/helix，codex/multipart-1gib-20261014，base e337e37e111b811144957eee4dfcdb5ae4921847。
- 已提交：61ec18c5合同；52f228035a6b3298bd0b35e5628d6b89c3864811 IM；dadcf70cc2514fd634017376022a7d674247794b core/host；9ef188420ffed87cab62f7a5cb034eeb684abaed drain/100MiB；65680f075c053c89c4ac373378e164a4f864cf94本地集成。
- Java CSES：/System/Volumes/Data/workspace/worktrees/multipart-1gib-20261014/cses，同分支，HEAD 130de35c664e5536af0592052c182288b1f5990d；继承通用OSS，无需源改动。
- paas-service：同父目录，同分支；HEAD 913075c4bf9e21d38becdbea8bfecd0b37a4ef70；54d0773c23b340c0a6295006bc52d189ebd5758b新接口，c008460恢复/防覆盖；913075c大小边界；已快进合入原internal开发分支。
- 客户端：同父目录cses-client，同分支；base 96c6a531d7efd63929f289bc7c1101f613b64ba9；提交b36c5935ae50fa1a9413335ae8cd262b5ce2367c。
- 原仓dirty未复制或暂存。远端client/Java已有后续开发差异，保持用户本地开发快照，不扩张合并。
## 当前阶段与剩余
- Java durable owner/session、init/parts/sign/complete/abort、防覆盖、完成丢响应恢复已完成。
- IM14项focused+parser1+G02d3绿；drain同时等待旧PUT与checkpoint ACK；Host分片本地集成1绿。
- 100MiB阈值和边界已完成；101MiB/13片真实本地HTTP/Host SQLite→稳定引用屏障→模拟Go/WS通过。
- 客户端1GiB/流式/sidecar恢复/4GiB账号与8GiB全局配额已完成；待补成功terminal回收、崩溃半成品及声明偏小预写校验。
## 已验证（按身份复用）
- dadcf70c core/host diff：core29通过+1既有doc ignored；core wasm check、runtime依赖tree通过。
- host upload_range6、upload_budget1、upload诊断3、旧media_pipeline2、upload_url_contract2通过；本地HTTP/SQLite模拟边界。
- 9ef18842+dadcf70c：五crate check、Web wasm build通过；当前Helix开发分支新增f307fcf3(0.1.64)待合入再复验。
- IM：multipart14、MV3-G02d3、artifact schema11，cargo check通过；逐文件格式通过，仓内其他既有格式漂移未改。
- Java：原30项单测+legacy boundary8通过，生产SDK adapter编译；复用缓存javassist3.30.2的/tmp init脚本，非依赖版本改动。
- CSES composite dependencyInsight通过，OSS编译完成；server全编译被sms/push两组Maven403阻断，不扩大修复。
- 客户端：TS12+12、package-boundaries通过；Rust stage28、lifecycle1、command1通过；临时本地Helix路径cargo check通过，manifest/lock已恢复。
- 旧media_pipeline两次15s超时，根因未定；随后7轮各2项通过，失败保留，未放宽断言。
- 本地日志目录：.perf/multipart-20261014/；Java/CSES/client摘要已复制于此，原详证见/tmp各multipart-20261014文件。
## 未验证与交付边界
- Java controller/Redis/真实OSS、真实Go/WS/Tauri、完整1GiB慢网上传和容量均NOT_VERIFIED；当前证据不代替真实验收。
- Web本地路径上传未纳入本次实现，只保证协议/wasm编译兼容；公开上传取消入口N/A，宿主关闭需释放执行任务。
- 未完成OSS fragment运维生命周期配置/真实验收；Java会话保留cleanup ID，不静默TTL丢失。
- 客户端正常pin未擅改；ba8472b8309883175244a3d74185431475f5982b增加显式with-local-helix验收入口，退出恢复manifest/lock。本地新Helix源码验证使用临时path，不提交绝对路径/猜测版本。发布及正式pin更新待后续授权。
## 下一步
- 完成客户端lease失败重试/微型terminal事实收尾；合入最新本地开发分支并复验、保留其他改动；用户已授权本地merge，绝不push。
