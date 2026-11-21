# F-21 之五：iPhone 的锁屏推送（交接给在 Mac 上做的工程师）

2026-11-21 写。方案见 `docs/ip/m2-spec.md` D-35。主机、中转、Android 已经做完，iPhone 只差 App 这一半。

## 已经有的

| 部分 | 在哪 | 状态 |
| --- | --- | --- |
| 主机放行中转（`ip.yaml` 的 `push.hosts`，负载上限 2800 字节） | `rust/crates/ip-server/src/webpush.rs` | PR #40 已合并 |
| 推送中转 `ip-relay` | `rust/crates/ip-relay/` | PR #41 已合并 |
| Android：登记、开关、FCM 收推送、解密弹通知 | `apps/flutter/lib/src/push/lock_push.dart`，`apps/flutter/android/app/src/main/kotlin/dev/intelligent_position/ip_app/` | 和这份说明同一个 PR |

中转对 APNs 已经按 iOS 的需要发：`mutable-content: 1` 的通知，标题“intelligent-position”、正文“有一条新通知”是解不开时的兜底，密文放在 `userInfo["ip"]`（base64，标准字母表、带填充），`apns-collapse-id` 是主机的 `Topic`，`apns-priority` 10 是“要我处理的”、5 是其余的。

## iPhone 要做的

1. **Dart 这边几乎不用改**：`lock_push.dart` 的 `PushChannel` 是平台无关的接口。`platformPushChannel()` 里加 iOS：可以直接复用 `AndroidPushChannel`（改名成 `NativePushChannel`），只是 `platform` 返回 `apns`。方法通道名 `ip/push`，约定和 Android 一样：
   - `available` → Bool：这个构建能不能收推送（有 `aps-environment` 权利、`IP_PUSH_RELAY` 给了）。
   - `registration` → `{token, p256dh, auth}`：`token` 是 APNs 设备令牌的**小写十六进制**（中转只认十六进制）；`p256dh` 是 65 字节未压缩 P-256 公钥、`auth` 是 16 字节随机数，都是 base64url 不带填充。没有就生成一套存起来。
   - `reset`：删掉密钥，调 `unregisterForRemoteNotifications`。
   - `takeTapped` → String?：App 是被点推送通知打开的，返回 `thread_id`（没有对话时空串），只给一次。
   - 原生调 Dart：`tapped(threadId)`、`tokenChanged`。
2. **Runner（`ios/Runner/AppDelegate.swift`）**：`registerForRemoteNotifications`，在 `didRegisterForRemoteNotificationsWithDeviceToken` 里拿令牌回给 `registration`；令牌和上次不同时调 `tokenChanged`。
3. **密钥**：用 CryptoKit 的 `P256.KeyAgreement.PrivateKey` 生成，存在钥匙串里，放在 App 和通知扩展共用的钥匙串分组（`keychain-access-groups` 权利），扩展才读得到。`kSecAttrAccessibleAfterFirstUnlock`，锁屏时也要能读。
4. **通知扩展（新建 Notification Service Extension target）**：`didReceive` 里取 `userInfo["ip"]`，按 RFC 8291 解密（`aes128gcm`，只有一条记录）：
   - 格式：盐 16 字节 | 记录大小 4 | 密钥 id 长度 1（=65）| 发送方临时公钥 65 | 密文。
   - `ecdh = sharedSecretFromKeyAgreement(发送方公钥)`；`IKM = HKDF(salt: auth, ikm: ecdh, info: "WebPush: info\0" + 自己的公钥 + 发送方公钥, 32)`；`CEK = HKDF(salt: 盐, ikm: IKM, info: "Content-Encoding: aes128gcm\0", 16)`；`NONCE = HKDF(…, "Content-Encoding: nonce\0", 12)`；AES-128-GCM 解开，去掉末尾的 0 和最后的分隔符 `0x02`。
   - 解出 JSON `{title, body, tag, thread_id?, silent?}`：改写通知的标题、正文，`threadIdentifier` 用 `thread_id`，`userInfo` 里带上 `thread_id` 供点开用。
   - `silent` 是“已在别处处理”：iOS 不能不显示，用 `removeDeliveredNotifications` 撤掉同一件事的旧通知，再显示这条（collapse-id 相同，会替换）。
   - Kotlin 版可以照抄逻辑：`WebPushCrypto.kt`。测试用 RFC 8291 附录 A 的例子，数值在 `android/app/src/test/.../WebPushCryptoTest.kt`。
5. **点通知**：`UNUserNotificationCenterDelegate` 的 `didReceive response` 取 `thread_id` 调 `tapped`。注意 `flutter_local_notifications` 也会把自己设成 delegate，两边要串起来（先看它的 `userInfo` 是不是自己的通知）。
6. **权利和配置**：`aps-environment`（开发版 `development`）、`keychain-access-groups`；Bundle ID 要和中转的 `IP_RELAY_APNS_TOPIC` 一致；开发版 App 要配 `IP_RELAY_APNS_SANDBOX=1` 的中转（或者另起一个）。都要维护者的苹果开发者账号签名。
7. **文档**：做完在同一个 PR 里改 `apps/flutter/README.md`（锁屏推送一条、验证到哪一步）、`docs/ip/apps-plan.md` F-21 那一行、`docs/ip/backlog.md` 和 `rust/README.md` 的勾选项；`rust/CONTRIBUTING.md` 敏感路径的“推送”一类加上 iOS 存私钥、解密的文件。

## 验证

没在真 iPhone、真 APNs 上收到过之前，不写“可用”。能做的：通知扩展的解密用 RFC 8291 的例子写 XCTest；`flutter build ios --no-codesign` 编过；有账号后在真机上配对、开锁屏推送、杀掉 App，让主机推一条审批卡。
