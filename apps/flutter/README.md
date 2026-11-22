# 各平台 App（Flutter）

intelligent-position 的 App，一份 Dart 代码出 Android、iOS、macOS、Windows、Linux 五个平台。连用户自己的主机（`ip serve`），在手机和电脑上处理待办、和数字员工对话。为什么用 Flutter、和网页控制台、Tauri 桌面 App 是什么关系，见 [`docs/ip/apps-plan.md`](../../docs/ip/apps-plan.md) 第 9 节；推进清单是 backlog F-13 起。

现在能做的（F-13 到 F-17、F-22）。导航是待办、星图、对话、任务、岗位五页，“我”在顶栏右上角（宽屏在导航栏底部）：

- **连接主机**：填主机地址和 6 位配对码，或者直接粘贴控制台给的配对链接。先请人确认主机地址再配对；明文连局域网地址（不是 HTTPS、不是 Tailscale 的 `*.ts.net`、不是本机）要勾“我信任这个网络”。配对拿到的设备令牌放系统的安全存储。
- **待办**（手机首页）：员工提问、等你决定的审批卡片（选项整行可点）、受阻的任务（显示受阻原因、执行对话和派活对话，状态能直接改），“最近 7 天”完成的任务和新报告。导航上的角标是待决定的卡片加受阻的任务。
- **看报告**（F-14）：员工写在工作目录 `reports/` 下的报告，手机上整页、宽屏上浮层，按 Markdown 显示。消息、任务结果、报告里提到的 `reports/<文件名>.md` 都能点开。
- **星图**（F-22）：照维护者给的 Obsidian 图谱样稿，背景是星空。幕僚长（协调员）是中心的太阳，其他员工是八大行星和知名恒星；离太阳越近越紧急，图标越大越重要。悬停高亮关联，点一颗天体弹出可拖动的半透明气泡：为什么在这条轨道上、为什么这么大，员工的提问可以直接选，右边是和这位员工的对话。紧急度和重要度怎么算见 [`apps-plan.md`](../../docs/ip/apps-plan.md) 9.7。
- **对话**：对话列表、历史、发消息（选员工或自动分派）、员工的流式回复，员工回复按 Markdown 显示（表格横向滚动，不加载图片，链接只用系统浏览器开 http、https、mailto），工具调用和摘要折叠显示。宽屏上列表和对话并排，窄屏上点开对话推进新的一屏。桌面上 Enter 发送、Shift+Enter 换行。
- **任务**（F-15）：未完成和已结束的任务，状态能直接改；新建任务（标题、说明、负责人、截止日期），过期的标出来。
- **岗位**（F-15）：今天的模型费用合计和提示词缓存命中率；每个岗位的模型、今天花费对比每日预算、缓存命中、员工（点了和他开新对话）、最近的报告。
- **我**：连的主机、设备名、权限、协议版本；这台设备的通知设置；断开并忘掉这台主机。
- **管理**（F-18，只给“管理”权限的设备，在“我”页里）：**设备**页添加设备（6 位配对码、二维码、配对链接）、改名、改权限、吊销；**定时任务**页看下次和上次运行，立即运行一次；**记忆**页按岗位改决策者笔记和岗位记忆摘要，删更正，抹掉、关掉对话的记忆，清空；**路由规则**页试一试、采纳或忽略规则草案、编辑整套规则（条件、交给谁、顺序、语义匹配、兜底）。
- **系统通知**（F-16）：App 不在前台（手机切到别的 App、桌面窗口没有焦点）时，主机发来的通知弹成系统通知，同一件事的新通知替换旧的，在别处处理过了就撤掉；点通知去那个对话，没有对话的去待办。通知哪些事、带不带预览、静音、按岗位关，在“我”页改，存在主机上（和控制台“设备与通知”页是同一份设置）。连上主机后才向系统要通知权限。手机锁屏、App 被系统关掉以后要靠下面的锁屏推送。
- **桌面托盘**（F-17，macOS、Windows、Linux）：关窗口只是藏到托盘，连接不断，照样弹通知；从托盘菜单“退出”才真的退出。托盘提示“N 件待办”，macOS、Linux 图标旁边再显示数字。托盘菜单里勾“开机启动”：Linux、Windows 开机后只在托盘里，不弹窗口；macOS 要 13 以上，在“系统设置 > 通用 > 登录项”里也能关。点托盘图标、点通知、再打开一次 App 都把开着的那个窗口叫出来，不另起一个。Linux 的托盘要桌面支持 AppIndicator（GNOME 要装 AppIndicator 扩展）。
- **扫码配对**（F-19，Android、iOS、macOS）：“连接主机”页点“扫配对二维码”，扫控制台或 App“设备”页上的码，扫到后和手填一样先确认主机再配对。相机画面在本机解码（纯 Dart 的 zxing2），不用谷歌的 ML Kit。第一次扫时系统会要相机权限。
- **锁屏推送**（F-21，Android 和 iPhone）：App 被系统关掉、手机锁屏以后也能收到通知。连上主机、有了通知权限后，App 向推送中转（`rust/crates/ip-relay`）登记系统的推送令牌（Android 是 FCM，iPhone 是 APNs）和这台手机自己的 Web Push 公钥，再把中转给的地址告诉主机；主机照旧发加密的 Web Push，中转原样转给 FCM 或 APNs，手机用自己的私钥解开，中转看不到内容。Android 由原生代码（`android/app/src/main/kotlin/…/IpPush.kt`）解密、弹通知，“已在别处处理”直接撤掉原来那条。iPhone 由通知扩展（`ios/NotificationService/`）解密、改写苹果送来的通知，解不开时显示“有一条新通知”；App 在后台还连着时自己弹过的同一件事，推送到了只替换、不再响铃；“已在别处处理”撤掉原来那条，换成一条不响铃、不亮屏的（iOS 不让通知扩展什么都不显示）。“我”页“这台设备的通知”里有“锁屏推送”开关，关掉会删掉主机上的订阅和手机上的密钥。Android 要谷歌 Play 服务；构建时没给推送参数、iPhone 版没用开发者账号签名（见下）就没有这个功能。
- 断线自动重连（500 毫秒起翻倍，最多 8 秒），带上次的事件序号让主机补发断开期间的事件；补不上就整个重新拉。App 从后台回到前台时马上重连。主机吊销了这台设备（401）时不再重连，提示重新配对。

还没有的（按顺序做，见 backlog）：锁屏推送还没用真的 Firebase、APNs、推送中转在真手机上收过（F-21 之六）。

## 锁屏推送的构建参数（F-21）

Firebase 的配置和推送中转的地址不放进仓库，构建 Android 版时给：

```sh
flutter build apk --release \
  --dart-define=IP_PUSH_RELAY=https://push.example.com \
  -PIP_FCM_APP_ID=1:1234567890:android:abc -PIP_FCM_API_KEY=… \
  -PIP_FCM_PROJECT_ID=ip-demo -PIP_FCM_SENDER_ID=1234567890
```

`-P` 的四项也可以用同名的环境变量给，值在 Firebase 控制台“项目设置”里这个 Android 应用下面（包名 `dev.intelligent_position.ip_app`）。都没给时 App 照常能用，只是“锁屏推送”开关是灰的。主机的 `ip.yaml` 要把中转的域名写进 `push.hosts`（m2-spec D-35）。

iPhone 版的中转地址同样用 `--dart-define=IP_PUSH_RELAY=…` 给，推送要用维护者的苹果开发者账号签名：在 Xcode 里给 `Runner` 和 `NotificationService` 两个 target 选团队，自动签名按 `ios/Runner/Runner.entitlements`、`ios/NotificationService/NotificationService.entitlements` 开推送和钥匙串共享（私钥放在 App 和通知扩展共用的钥匙串分组里）。Bundle ID 是 `dev.intelligentposition.ipApp`，中转的 `IP_RELAY_APNS_TOPIC` 填它。开发版签名（`aps-environment` 是 `development`）只收苹果测试环境的推送，要连 `IP_RELAY_APNS_SANDBOX=1` 的中转；打 App Store、TestFlight 包时 Xcode 按发布用的描述文件换成 `production`，连不带这一项的中转。没签名的构建（`--no-codesign`、模拟器上不选团队）“锁屏推送”开关是灰的。改 Bundle ID 时两个 `.entitlements` 和 `ios/Shared/PushKeychain.swift` 的 `groupSuffix` 一起改。

```sh
flutter build ios --release --dart-define=IP_PUSH_RELAY=https://push.example.com
```

解密的单元测试（RFC 8291 的例子、和 Dart 那边同样的通知 id）。iPhone 版的在模拟器上跑，要 Xcode（CI 的 `flutter-ios` 工作流在改到 `ios/` 时跑）：

```sh
flutter build ios --simulator --debug
xcodebuild test -workspace ios/Runner.xcworkspace -scheme Runner -destination 'platform=iOS Simulator,name=iPhone 16' \
  -only-testing:RunnerTests/WebPushCryptoTests -only-testing:RunnerTests/PushNoticeTests
```

Android 版的在 JVM 上跑，要 Android SDK：

```sh
cd android && ./gradlew :app:testDebugUnitTest
```

## 协议类型从 Rust 生成（F-20）

Rust 的协议类型由 ts-rs 导出成 `rust/web/src/types/*.ts`，`tool/gen_protocol.dart` 再把它们生成成 `lib/src/protocol/wire.dart`（不要手改）。改了 Rust 的协议类型时：

```sh
cd rust && cargo test          # 重新导出 .ts
cd ../apps/flutter && dart run tool/gen_protocol.dart
```

两份生成结果都要提交，CI 两边都会查。`models.dart`、`router.dart` 先用 `wire.dart` 解析，再转成界面用的样子，`client.dart` 发的请求也用 `wire.dart` 的 `Request` 拼，所以 Rust 改了字段、重新生成后，用到的地方会编不过；少了协议里必有的字段会报错，不悄悄补默认值。`number` 生成成 `num`（TS 里分不出整数和小数）；认不出的枚举值落到 `unknown`，认不出的那一种落到 `XxxUnknown`，主机比 App 新时不会解析失败。

## 桌面交互验证

本轮在 Linux 用假主机跑过 53 个 Flutter 测试，包含跨断点保留草稿、从待办打开会话和星图手势。macOS 窗口的两项 XCTest 和真触控板手感仍待 Mac 验证。

- 对话按扣除导航后的可用宽度分栏。打开一条对话并输入草稿，将窗口从 1280 拖窄到 880，再到手机宽度，当前会话、草稿和返回列表都应正常。从待办点“查看对话”也应直接看到正文。
- 星图上双指移动应平移，捏合和鼠标滚轮应以指针所在的位置缩放。Dart 回归测试使用合成的指针事件。真机验收时，在同一颗天体旁连续平移、捏合再反向缩放。
- Mac 第一次打开时目标内容区为 1120 × 760 点，以屏幕可用空间为限。移动并调整大小后用 ⌘Q 退出，再打开应回到原位置；拔掉外接屏后应能找到标题栏。安装好 Flutter 和 Xcode 后，在本目录执行 `flutter build macos --debug`，再执行 `xcodebuild -workspace macos/Runner.xcworkspace -scheme Runner -configuration Debug -destination 'platform=macOS' test`，可运行使用独立窗口保存名的 AppKit 测试。

## 和主机怎么连

- 主机要配 `auth_token_env`（设备配对要它），在主机上运行 `ip device pair --config ip.yaml --name <设备名>`，或者在控制台“设备”页点“添加设备”，拿到 6 位配对码（2 分钟内有效，只能用一次）。
- App 用 `POST /pair` 换这台设备专属的令牌（m2-spec D-30），之后 `/ws` 用 `Authorization: Bearer` 带令牌，不放进地址。原生 App 不带 `Origin`，主机按非浏览器客户端处理，主机那边不用改配置。
- 新配对的设备是“日常”权限（m2-spec D-31），消息和审批按设备名署名。
- 令牌存在哪：iOS、macOS 钥匙串（macOS 用普通的登录钥匙串，不用“钥匙串共享”，没签名也能用），Android 用 Keystore 加密，Windows 凭据管理，Linux 走 Secret Service（GNOME 钥匙串、KWallet 等，要装 `libsecret-1-0`）。

## 代码结构

| 位置 | 内容 |
| --- | --- |
| `lib/src/protocol/models.dart` | `/ws` 协议里 App 用到的类型（界面用的样子），从 `wire.dart` 的生成类型转过来；认不出的事件、帧不抛错 |
| `lib/src/protocol/router.dart` | 路由规则的类型（F-18），可改；进出都经过 `wire.dart` |
| `lib/src/protocol/client.dart` | `/ws` 客户端：hello、请求和回复按 id 配对、事件序号、断线补发、`lagged`、`alert`、`dismiss`、重连退避；和网页控制台的 `client.js` 同一套做法 |
| `lib/src/host.dart` | 主机地址的规整和信任规则、配对链接解析 |
| `lib/src/pairing.dart` | `POST /pair` |
| `lib/src/storage.dart` | 主机列表和令牌放安全存储 |
| `lib/src/qr/` | 扫二维码（F-19）：`decode.dart` 把相机画面转成灰度图再用 zxing2 解，`scanner.dart` 是各平台的相机 |
| `lib/src/notifications.dart` | 系统通知：弹、按标签撤、点开回调（F-16） |
| `lib/src/push/lock_push.dart` | 锁屏推送（F-21）：向推送中转登记、`subscribe_push`、令牌变了换地址、开关；原生那一半：Android 在 `android/app/src/main/kotlin/dev/intelligent_position/ip_app/`（`IpPush.kt` 收 FCM、存密钥、弹通知，`WebPushCrypto.kt` 解密）；iPhone 在 `ios/`（`Runner/IpPush.swift` 要 APNs 令牌、方法通道、点通知，`Shared/` 是 App 和通知扩展共用的解密、钥匙串、通知改写，`NotificationService/` 是通知扩展） |
| `lib/src/desktop/` | 桌面的托盘和窗口（`shell.dart`）、开机启动（`autostart.dart`）（F-17）；只开一个、`--hidden` 在 `linux/runner/`、`windows/runner/` 里，macOS 的开机启动在 `macos/Runner/MainFlutterWindow.swift` |
| `lib/src/app_state.dart` | App 的状态：连接、员工、对话、审批、任务、哪些员工正在处理或上一轮失败 |
| `lib/src/star_map.dart` | 星图的数据：谁坐太阳、每位员工的紧急度、重要度、理由和天体 |
| `lib/src/ui/admin/` | “管理”权限才有的页（F-18）：`devices_page.dart` 是设备页，`schedules_page.dart` 是定时任务页，`memory_page.dart` 是记忆页，`rules_page.dart` 是路由规则页 |
| `lib/src/ui/` | 各页（`task_card.dart` 是待办和任务页共用的任务卡片）；`star_map_page.dart` 是星图，`celestial_painter.dart` 画太阳、行星和恒星，`report.dart` 是报告链接和看报告 |
| `test/` | 单元测试和界面测试，跑在假主机上（`test/fake_host.dart`） |
| `integration_test/` | 连真主机的端到端测试，不在 CI 跑 |
| `lib/src/protocol/wire.dart` | 从 Rust 生成的协议类型（F-20），`tool/gen_protocol.dart` 生成，不要手改 |
| `tool/gen_protocol.dart` | 从 ts-rs 导出的 `rust/web/src/types/*.ts` 生成 `wire.dart`；`--check` 只查是不是最新的 |
| `tool/check_licenses.dart` | 依赖许可检查 |

## 开发

装 Flutter 3.47.9（和 CI 一致，`.github/workflows/flutter.yml` 的 `FLUTTER_VERSION`）。

```sh
cd apps/flutter
flutter pub get
flutter run -d linux        # 或 macos、windows、一台连着的手机、模拟器
```

Linux 上编译要 `libgtk-3-dev`、`libsecret-1-dev`、`libayatana-appindicator3-dev`、`ninja-build`、`clang`、`cmake`；运行要 `libsecret-1-0`、`libayatana-appindicator3-1`。Android 要 Android SDK 和 JDK 17；iOS、macOS 要 Xcode。

本地改了用到的图标后，发布版里图标显示成方框或乱码时，是图标字体的裁剪结果被缓存了：`flutter clean` 后再编（CI 每次从头编，不受影响）。

Android 模拟器里连电脑上的主机用 `http://10.0.2.2:7878`（会让你勾“我信任这个网络”）；真手机建议走 HTTPS 或 Tailscale，见 [`docs/ip/deploy-docker.md`](../../docs/ip/deploy-docker.md)。

## 检查（和 CI 一致）

```sh
cd apps/flutter
dart format --output=none --set-exit-if-changed lib test tool integration_test
flutter analyze --fatal-infos
flutter test
dart run tool/check_licenses.dart
dart run tool/gen_protocol.dart --check
flutter build linux --release
```

CI 的 `flutter` 工作流在 PR 改到 `apps/flutter/` 或 `rust/web/src/types/` 时跑上面这些（只用 Linux）。五个平台的安装包只在手动触发或打 `app-v*` 标签时打：Linux、Android（APK，调试签名）、macOS、iOS（不签名，只看能不能编过）、Windows。都没有签名证书。

依赖只用和 Apache-2.0 兼容的许可（BSD、MIT、Apache-2.0），`tool/check_licenses.dart` 按每个包的 LICENSE 正文查，认不出的就失败。现在的直接依赖：`flutter_secure_storage`、`flutter_markdown_plus`、`markdown`、`url_launcher`、`flutter_local_notifications`（都是 BSD-3-Clause），`tray_manager`、`window_manager`（MIT），扫码用的 `camera`、`zxing2`（BSD-3-Clause）和 `camera_macos`（MIT）。按包放行的例外只有 `dbus`（MPL-2.0，`flutter_local_notifications` 在 Linux 上用，原样使用）。Android 原生那边另有两个 Gradle 依赖：`com.google.firebase:firebase-messaging`（Apache-2.0，运行时要手机上闭源的谷歌 Play 服务）和 `androidx.core`（Apache-2.0）；只在测试时用的 `junit`（EPL-1.0）不进 App。

## 连真主机的端到端测试

在真的 App 里配对、看待办、开新对话发消息、等员工流式回复，再走一遍“路由拿不准 → 待办里选定岗位 → 员工接着回复”，最后忘掉主机。用系统真的安全存储和网络。

```sh
# 主机上（配了 auth_token_env，有一条路由规则，匹配不上的走决策卡）
ip device pair --config ip.yaml --name "测试设备"
# App 这边，2 分钟内
flutter test integration_test -d <设备> \
  --dart-define=IP_E2E_URL=https://ip.example.com \
  --dart-define=IP_E2E_CODE=<配对码>
```

最后还会打开星图，确认每位员工都在上面。可选：`IP_E2E_TEXT`（第一条消息，默认问 9 月 CPI，要能被路由规则分给某个员工）、`IP_E2E_UNROUTED`（一句路由规则匹配不上的话，给空字符串跳过决策卡那段）、`IP_E2E_SHOTS`（截屏目录，只在 Linux X11 上有用）。测完主机那边的设备记录还在，到控制台“设备”页吊销。

## 验证到哪一步

2026-11-21：

- `flutter test`：119 个测试，假主机。
- 真的 Linux 版 App（Xvfb 虚拟屏幕、GNOME 钥匙串）连本机真的 `ip serve`，**模型是假的**（一个本机的假 OpenAI 流式接口，回固定文字）：端到端测试在宽屏（1280 像素）和手机宽度（390 像素）各跑通一次；另外用 `xdotool` 在发布版里用真键盘配对、连发两条消息、Shift+Enter 换行。
- 星图：同一台假模型的主机上配了一位协调员和十位员工，用脚本造了员工提问、协调员派活、任务过期、受阻、快到期、一轮失败、慢回复几种情况，在真的 Linux 版里看了位置、大小、理由和气泡，宽屏和手机宽度都点过。Impeller 的 GL 后端在这台机器的 Mesa 软件渲染上画 1 像素的圆描边会出方块、在透明度图层里画背景模糊会整块丢掉，已经绕开（轨道画成折线，气泡不做淡入动画）；真显卡上没试过。
- 待办和看报告（F-14）：同一台主机上把手写的报告放进员工工作目录，在真的 Linux 版（手机宽度）里从完成的任务结果打开报告、在报告里打开上一份、把受阻的任务改成已取消，主机上核对过状态。
- 任务页和岗位页（F-15）：真的 Linux 版（手机宽度）里用真键盘新建了任务（选负责人、选截止日期），主机上署了设备名、另开了执行对话；岗位页显示了主机算的当天费用和缓存命中，从岗位页点开和员工的新对话。
- 锁屏推送（F-21 之四）：`flutter test` 里 7 个测试用假的推送通道和假中转走登记、换令牌、开关、点通知；Kotlin 的解密用 RFC 8291 附录 A 的例子在 JVM 上核对过（`./gradlew :app:testDebugUnitTest`，4 个）。没用真的 Firebase 项目、没在真手机上收过推送。
- 锁屏推送（F-21 之五，iPhone）：2026-11-22，Swift 的解密测试（RFC 8291 附录 A 的例子，6 个）在 Linux 容器里用 Swift 6.0.3 和接口相同的 swift-crypto 跑过；`flutter test` 里用模拟的原生端测过方法通道 `ip/push` 的约定。CI 的 `flutter-ios` 工作流（macOS、Flutter 3.47.9）第一次跑（#50）：`flutter build ios --debug --no-codesign` 和 `--simulator` 都编过（App 带着通知扩展），8 个 Swift 测试（解密 6 个、通知改写 2 个）在 iPhone 模拟器上全过。没签名，没在真 iPhone、真 APNs 上收过推送；真机上怎么验在 `docs/ip/f21-iphone-handoff.md`。
- Android：2026-11-21 在容器里装了 Android SDK，`flutter build apk --debug` 编过（改锁屏推送之前和之后各一次），没在模拟器或真手机上跑过。iOS 只在 CI 上编过、在模拟器上跑过 Swift 单元测试，App 没在模拟器或真机上打开过；macOS、Windows 没编过也没跑过（容器里没有 Xcode）；CI 的安装包任务没触发过。真手机、真主机上的 HTTPS、Tailscale 都没试过。
