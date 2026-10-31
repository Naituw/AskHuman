# iOS 视频附件

2026-10-30 用户反馈相册里不能选视频，确认超出 10 MB 时自动压缩，并要求“明显压缩不了的就不要尝试”。
后续确认：仅预估 ≤8 MB 才开始，不加 30 秒自动取消；界面显示进度，让用户自己决定取消。
行为写入 `docs/specs/ios-channel.md` §6.5；实现沿用现有附件上传协议，不增加 Mac / 网关限额。

## 处理规则

1. 回答、插话与新任务共用的相册入口允许图片 / 视频混选，`preferredItemEncoding: .current` 避免选择器先做隐式转码。
   图片表示优先保留既有图片处理，视频使用文件型 `Transferable`，不把整个大视频加载为 `Data`。
2. Photos 给出的文件 URL 只保证在导入期间有效，因此先取得应用自有临时副本；成功、失败、取消或传输值被丢弃时释放。
   从“文件”选入的视频也走相同的准备流程，并保持 security-scoped URL 的访问期覆盖整个准备过程。
3. 大小不超过 `Wire.maxAssetBytes`（10 × 1024² bytes）时保留原字节、扩展名和 MIME。
4. 超限时检查完整时长和视频轨，向系统询问 1080p / 720p / 480p / LowQuality 的 MP4 导出兼容性与体积预估。
   没有有效预估时不盲目编码；没有预估 ≤8 MB（预算的 80%）的档位则提示先裁短。
5. 有可用档位时，按画质从高到低选预估不超过预算 80% 的档位，不回退到贴近上限的预估。
   只做一次完整导出，设置 `fileLengthLimit`，随后重新检查实际大小和源 / 输出时长（相差不得达 0.25 秒）。
   系统的体积限制允许略微超过，不能代替最终的严格字节检查，也不能用截短内容来满足限制。
6. 读取 / 检查显示状态；压缩进度条、百分比与取消按钮直接显示在作答区 / 插话栏，不必打开菜单。
   不设置自动超时，由用户决定继续等或取消。离开组件取消该批任务；取消后不添加晚到附件。
   AVAssetExportSession 不是 Sendable，所有状态与取消留在 MainActor；实际编码由 AVFoundation 执行。
7. AnswerComposer / InterjectComposer / TaskComposer 按 token 集合跟踪准备任务。所有准备完成后才恢复发送，一个任务结束不能提前放开另一任务的发送。
   视频按普通文件上传，`isImage: false`，答案使用 files 字段；附件行用电影图标、文件名与大小表示视频。

## 验证

Swift 包的 `VideoAttachmentTests` 包括纯预估选择、限额内原样保留、并发准备的发送门控、视频进入文件字段，
以及 AVAssetWriter 实际生成的噪声视频：压缩后大小与完整时长、极小预算在任何编码进度出现前被拒绝、取消不返回附件。

```bash
swift test --package-path ios/Packages/AskHumanKit --filter VideoAttachmentTests
```

相册真实 UI 使用独立模拟器，避免污染日常 / 真服务测试库。准备四个视频（ffmpeg）：

```bash
mkdir -p /tmp/askhuman-video-qa
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=320x240:rate=15' -t 2 \
  -c:v libx264 -preset ultrafast -crf 28 /tmp/askhuman-video-qa/SmallVideo.mp4
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=1280x720:rate=15' -t 6 \
  -c:v libx264 -preset ultrafast -b:v 25M -minrate 25M -maxrate 25M -bufsize 50M \
  -x264-params 'nal-hrd=cbr' /tmp/askhuman-video-qa/LargeVideo.mp4
ffmpeg -v error -y -f lavfi -i 'color=c=navy:size=320x240:rate=1' -t 1800 \
  -c:v libx264 -preset ultrafast -b:v 120k -minrate 120k -maxrate 120k -bufsize 120k \
  -x264-params 'nal-hrd=cbr' /tmp/askhuman-video-qa/LongVideo.mp4
ffmpeg -v error -y -f lavfi -i 'testsrc2=size=3840x2160:rate=60' -t 1 \
  -c:v libx264 -preset ultrafast -crf 20 /tmp/askhuman-video-qa/CancelSource4K.mp4
ffmpeg -v error -y -stream_loop 44 -i /tmp/askhuman-video-qa/CancelSource4K.mp4 \
  -t 45 -c copy /tmp/askhuman-video-qa/CancelVideo4K.mp4
```

前三者本轮约 94 KB、17 MB、26 MB，时长为 2 秒、6 秒、30 分钟；第 4 个为 45 秒约 693 MiB 的 4K / 60 fps，保证在快速机器上也有时间验证进度和手动取消。加入专用模拟器：

```bash
xcrun simctl addmedia <UDID> /tmp/askhuman-video-qa/SmallVideo.mp4 \
  /tmp/askhuman-video-qa/LargeVideo.mp4 /tmp/askhuman-video-qa/LongVideo.mp4 \
  /tmp/askhuman-video-qa/CancelVideo4K.mp4
xcodegen generate --spec ios/project.yml
TEST_RUNNER_AH_VIDEO_PICKER_QA=YES xcodebuild test \
  -project ios/AskHuman.xcodeproj -scheme AskHuman \
  -destination 'platform=iOS Simulator,id=<UDID>' -derivedDataPath ios/build/DerivedDataVideoAttachments \
  CODE_SIGNING_ALLOWED=NO -parallel-testing-enabled NO -collect-test-diagnostics never \
  -only-testing:AskHumanUITests/AskHumanVideoAttachmentUITests
```

UI 用例检查照片 / 小视频混选、超限视频添加、长视频拒绝、插话入口，以及进度可见 / 手动取消后保留草稿并恢复发送；读取的是系统相册的实际视频条目。
未设置 fixture 环境变量时跳过，避免普通 UI 全套因没有预置相册内容而失败。Mock 模式不启动真实 Agent。

本轮验证（2026-10-30）：

- 6 个 `VideoAttachmentTests` 通过，包含真实视频导出和取消。
- Swift 包全套 454 个测试通过、43 个需真实服务的 E2E 测试按默认规则跳过，0 失败。
- iOS Simulator 的 `build-for-testing` 通过；本地化目录重新生成并通过编译。
- iOS 27.0 独立模拟器中，上述五个系统相册流程的断言全部完成（包含 4K 实际压缩中的进度条 / 百分比、手动取消、取消后保留文字并恢复发送；日志出现
  `VIDEO_ATTACHMENT_UI_ASSERTIONS_COMPLETE`）。当前 Xcode 的 XCTest 随后卡在收集待处理崩溃日志，
  采样栈为 `XCTestCase._waitForPendingCrashlogsIfNeeded → XCTCrashLogTracker.waitForPendingCrashlogs`；
  未得到 suite 成功报告，停止该次测试运行。没有把它计为完整 UI suite 通过。
  验证后删除本轮创建的专用模拟器和大型生成素材，未改日常模拟器的相册。
- `./scripts/install.sh` 通过；它验证 / 安装 Mac 主程序，不会把 iOS App 安装到手机。
- iPhone Air 当前不可连接，尚未在真机验证，也未上传 TestFlight。

参考：[PhotosPicker 的 Transferable](https://developer.apple.com/documentation/photokit/bringing-photos-picker-to-your-swiftui-app)、
[AVAssetExportSession.fileLengthLimit](https://developer.apple.com/documentation/avfoundation/avassetexportsession/filelengthlimit)。
