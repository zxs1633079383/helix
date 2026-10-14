# PC导航Token核对
- 仓库：/System/Volumes/Data/workspace/angular/visual/cses-client，dev-message，HEAD 21a66006966275115fc45cb9b8396be53df8f736；当前相关源码无本轮修改。
- 生产src、src-tauri/src和packages/message/src未发现NativeToken/nativeToken/navigationToken/navigation_token生产使用；测试明确断言locate payload无navigation_token。
- helixTauriImGate.service.ts:3864起locate按reqId登记pending与runtimeGeneration，:3897发送im_locate_message(channelId,messageId,pageSize,reqId,operation)。:2936起按req_id、generation、channelId、targetPostId/targetIndex结算；首屏/分页仍有独立windowToken。
- Helix当前parse_locate_request_for_command允许可选navigationToken；dispatch_read.rs:286-292明确不使用该token，locate窗口生命周期由Angular处理。MR13兼容snake字段不意味着PC当前发送该字段。
- 定向命令：./node_modules/.bin/jest --runInBand --runTestsByPath src/app/message-host/runtime/im-gate/helix/__tests__/timeline-locate-adapter.test.ts。
- PASS：1 suite / 8 tests，19.313秒；Mock Tauri bus的生产Angular适配器测试，不是安装版Native/UI验收。

- UI仍保留本地初始化token和导航revision：chat-content.component.ts:87-93的initMessageToken/navigationRevision，以及:601-613的jumpRevision、AbortSignal、频道初始化过期检查；这些不作为navigationToken发给Helix。
- UI定向命令：./node_modules/.bin/jest --runInBand --silent --runTestsByPath packages/message/src/lib/components/chat-content/__tests__/timeline-locate.test.ts；40/40 PASS，18.684秒，包含新导航取消旧完成/切群后不恢复旧来源/旧请求abort不伤新locate。原输出/tmp/helix-review-fix11/.pc-timeline-locate.log。
