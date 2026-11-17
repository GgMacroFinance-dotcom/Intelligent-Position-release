# 桌面 App

intelligent-position 的桌面 App（backlog F-11，方案见 [`docs/ip/apps-plan.md`](../../docs/ip/apps-plan.md)）。用 [Tauri 2](https://tauri.app) 做的壳，窗口里是自带的一份网页控制台，和主机上 `/` 的控制台是同一份代码（`rust/web`），不加载主机上的网页。

和在浏览器里打开控制台比，多了这几样：

- 连哪台主机记在本机配置目录的 `host.json`，设备令牌放系统钥匙串（macOS 钥匙串、Windows 凭据管理器、Linux 的 GNOME 钥匙串或 KWallet），不放网页存储。
- 关掉窗口后留在托盘里，连接不断，有事照样弹系统通知；托盘菜单“退出”才真的退出。
- 托盘图标上显示待办数（macOS、Linux 显示在图标旁边，Windows 在鼠标悬停的提示里）。
- 托盘菜单里能开关“开机启动”，开机后只进托盘，不弹窗口。
- 消息里的链接用系统浏览器打开。

## 连主机

1. 主机要配访问令牌（`ip.yaml` 的 `auth_token_env`）。没配令牌的主机只接受本机浏览器，桌面 App 连不上。
2. 在主机上生成配对码：`ip device pair --config ip.yaml --name 张三的电脑`，或者在已经连上的控制台“设备”页点“生成配对码”。
3. 打开桌面 App，填主机地址（例如 `https://ip.example.com`，只写到域名和端口）和 6 位配对码，点“连接”。
4. 也可以直接填访问令牌（共用令牌或已有的设备令牌）。之后要换主机、重新配对或断开，点右上角“令牌”。

配对码只能用一次。App 会先确认钥匙串能存，再去换令牌，免得换到了令牌存不下。

## 开发

要 Rust（1.85 以上）和 Node.js。Linux 另外要装 WebKitGTK 等系统库：

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

```sh
cd apps/desktop
npm ci
npx tauri dev      # 开发：改了 rust/web 的文件要重启一次（打包脚本在启动前拷文件）
npx tauri build    # 打安装包，出在 src-tauri/target/release/bundle/
```

`scripts/bundle.mjs` 把 `rust/web` 的页面、脚本、样式、图标拷到 `dist/`，`tauri dev` 和 `tauri build` 开始前会自动跑。控制台在桌面 App 里时，`window.__TAURI__` 存在，`app.js` 据此改用壳的命令连主机、弹通知、打开链接（搜 `desktop`）。

壳给网页的命令只有 `src-tauri/src/lib.rs` 里那几条：`get_host`、`pair`、`save_token`、`forget_host`、`notify`、`open_url`、`set_pending`。`capabilities/default.json` 只给了 `core:default`，网页调不了文件、命令行等别的原生接口。

检查（CI 的 `desktop` 工作流跑同样几项）：

```sh
cd apps/desktop && node scripts/bundle.mjs
cd src-tauri
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny --locked --all-features check licenses   # 允许清单在本目录的 deny.toml
```

三个平台的安装包由 CI 打：在 Actions 页手动运行 `desktop` 工作流，或者推一个 `desktop-v*` 标签，产物在那次运行的 Artifacts 里。没有签名证书，macOS 第一次打开要右键“打开”，Windows 会弹 SmartScreen 提示。

## 验到哪一步

- 2026-11-16 在 Linux（Ubuntu 24.04 容器，Xvfb 虚拟屏幕，GNOME 钥匙串）上跑过真的 App：显示“连接主机”，用配对码连上本机一台配了令牌的测试主机（假模型），令牌存进钥匙串，关掉再开能读回来、直接连上。
- 控制台在桌面模式下的其余行为（换主机、断开、托盘待办数、弹通知、链接交给系统浏览器）用 Playwright 加一个假的壳核对过。
- 没验过：托盘图标和菜单、系统通知、开机启动（虚拟屏幕里没有托盘和通知服务）；macOS、Windows 上一次都没跑过，三个平台的安装包还没打过。
- 已知的限制：点系统通知只能把窗口叫出来，去不了那条对话（Tauri 的通知插件在桌面上收不到点击）；控制台里有几处用了浏览器自带的确认框（`confirm`），macOS 上的 WebView 是否支持没验过。
