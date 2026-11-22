# F-21 之五：iPhone 的锁屏推送（交接给有 Mac 和 iPhone 的工程师）

2026-11-21 写，2026-11-22 改：代码已经写完（方案见 `docs/ip/m2-spec.md` D-35），剩下的是用维护者的苹果开发者账号签名、在真 iPhone 上收一次真推送。没做这一步之前，不写“可用”。

## 已经有的

| 部分 | 在哪 | 状态 |
| --- | --- | --- |
| 主机放行中转（`ip.yaml` 的 `push.hosts`，负载上限 2800 字节） | `rust/crates/ip-server/src/webpush.rs` | 已合并（#40） |
| 推送中转 `ip-relay` | `rust/crates/ip-relay/` | 已合并（#41） |
| Android：登记、开关、FCM 收推送、解密弹通知 | `apps/flutter/lib/src/push/lock_push.dart`，`apps/flutter/android/app/src/main/kotlin/dev/intelligent_position/ip_app/` | 已合并，只在假中转上测过 |
| iPhone：登记、开关、APNs 令牌、通知扩展解密改写、点通知 | `apps/flutter/ios/`（见下） | 本 PR，编过、解密测试跑过，没签名、没收过真推送 |

中转对 APNs 按 iOS 的需要发：`mutable-content: 1` 的通知，标题“intelligent-position”、正文“有一条新通知”是解不开时的兜底，密文放在 `userInfo["ip"]`（标准 base64，带填充），`apns-collapse-id` 是主机的 `Topic`（由标签算出），`apns-priority` 10 是“要我处理的”、5 是其余的。

## iPhone 这一半怎么做的

- **Dart**：`lock_push.dart` 的 `NativePushChannel`（原来的 `AndroidPushChannel`）两个平台共用，iPhone 的 `platform` 是 `apns`。方法通道 `ip/push`：`available`、`registration`（`{token, p256dh, auth}`，令牌是小写十六进制）、`reset`、`takeTapped`；原生调 Dart 的 `tapped`、`tokenChanged`。
- **App（`ios/Runner/`）**：`AppDelegate.swift` 在启动时把自己设成通知中心的代理（flutter_local_notifications 的说明也要这一步；之前没设，照插件的代码 F-16 的本地通知在 iPhone 上点了收不到回调，没在真机上试过），锁屏推送的通知自己处理，其余的经 `super` 交给插件。`IpPush.swift`：`registerForRemoteNotifications`，等令牌最多 30 秒；锁屏推送开着时每次启动重新要一次，令牌变了调 `tokenChanged`；点通知时 App 还没起来就先存着等 `takeTapped`。App 在前台时到的推送不弹（和 F-16 一样）。
- **共用（`ios/Shared/`，App 和通知扩展都编进去）**：`WebPushCrypto.swift` 照 RFC 8291 解密（CryptoKit，和 `WebPushCrypto.kt` 同一套逻辑），`alertId` 和 Dart 的一样；`PushKeychain.swift` 存私钥（32 字节）和 `auth`（16 字节），放在钥匙串分组 `<Team ID>.dev.intelligentposition.ipApp.push`，属性是“首次解锁后、只在这台设备”；Team ID 运行时从钥匙串读出来（探测项落在哪个分组），不用在构建设置里写；`PushNotice.swift` 是改写后的通知长什么样、怎么认出同一件事。
- **通知扩展（`ios/NotificationService/`，target `NotificationService`，Bundle ID `dev.intelligentposition.ipApp.NotificationService`）**：解开后改写标题、正文，`threadIdentifier` 用 `thread_id`，`userInfo` 记下对话和标签；解不开、超时显示兜底。iOS 不让通知扩展什么都不显示，所以 `silent` 是撤掉同一件事的旧通知（App 自己弹的按 `alertId` 的十进制 id，之前的推送按记下的标签），再显示一条不响铃、不亮屏（`interruptionLevel = .passive`）的；同一件事已经显示过时（App 在后台还连着时弹过），新到的推送撤掉那条、自己不响铃。
- **权利**：`Runner/Runner.entitlements` 是 `aps-environment`（`development`）和两个钥匙串分组（第一个是 App 自己的，主机令牌照旧只有 App 读得到；第二个和扩展共用）；`NotificationService/NotificationService.entitlements` 只有共用分组。工程文件里 Runner 依赖并嵌入扩展（“Embed Foundation Extensions”在“Thin Binary”之前）。

## 验证到哪一步

- Swift 的解密测试（`RunnerTests/WebPushCryptoTests.swift`，6 个，用 RFC 8291 附录 A 的例子，和 `WebPushCryptoTest.kt` 同一组数）：2026-11-22 在 Linux 容器里用 Swift 6.0.3 和 swift-crypto（接口和 CryptoKit 一样）跑过，全过。钥匙串、通知扩展、AppDelegate 这些只有苹果平台才有的代码在 Linux 上编不了。
- CI 的 `flutter-ios` 工作流（`.github/workflows/flutter-ios.yml`，改到 `apps/flutter/ios/` 时在 macOS 上跑）：`flutter build ios --debug --no-codesign`、`flutter build ios --debug --simulator`，在模拟器上跑 `WebPushCryptoTests` 和 `PushNoticeTests`（共 8 个）。结果见本 PR 的 CI。
- `flutter test`：`NativePushChannel` 的方法通道约定用模拟的原生端测过。
- 没签名，没在真 iPhone、真 APNs 上收过推送。

## 剩下要做的（要维护者的苹果开发者账号）

1. 苹果开发者后台：建 App ID `dev.intelligentposition.ipApp`（打开 Push Notifications、Keychain Sharing）和 `dev.intelligentposition.ipApp.NotificationService`（Keychain Sharing）；建一把 APNs 的 `.p8` 签名密钥。用 Xcode 的自动签名时这些大多会自动建。
2. Xcode 打开 `apps/flutter/ios/Runner.xcworkspace`，给 `Runner` 和 `NotificationService` 两个 target 选团队。不要把团队 ID 提交进仓库（`DEVELOPMENT_TEAM` 留在本地）。
3. 中转：`IP_RELAY_APNS_KEY_FILE`、`IP_RELAY_APNS_KEY_ID`、`IP_RELAY_APNS_TEAM_ID`、`IP_RELAY_APNS_TOPIC=dev.intelligentposition.ipApp`；开发版 App 要 `IP_RELAY_APNS_SANDBOX=1`（或者另起一个）。主机的 `ip.yaml` 把中转的域名写进 `push.hosts`。
4. 真机：`flutter run --release --dart-define=IP_PUSH_RELAY=https://<中转>`，配对主机，允许通知，“我”页看“锁屏推送”变成开着。然后：
   - 锁屏、从多任务里划掉 App，让主机推一条审批卡：锁屏上显示审批的标题和正文（不是“有一条新通知”），点开进那个对话。
   - 在控制台批掉这张卡：手机上那条换成“已在别处处理”，不响铃、不亮屏。
   - 推一条没有对话的（比如任务受阻）：点开去“待办”。
   - “我”页关掉锁屏推送：主机上的订阅删掉，之后的通知不再推到这台手机。
   - 重启手机、不解锁时推一条：显示兜底“有一条新通知”（钥匙串还读不到），解锁后再推的正常。
5. 跑通后改 `apps/flutter/README.md` 的“验证到哪一步”、`docs/ip/backlog.md` 和 `docs/ip/apps-plan.md` 的 F-21（⑥）、`rust/README.md` 的勾选项，写明在哪台 iPhone、哪个 iOS 版本上跑通的。
