//! intelligent-position 桌面 App（backlog F-11，`docs/ip/apps-plan.md`）。
//!
//! 一个 Tauri 2 壳，窗口里是自带的一份网页控制台（`rust/web`，打包时由 `scripts/bundle.mjs` 拷进来），
//! 不加载主机上的网页。壳只做网页做不了的几件事：
//!
//! - 连哪台主机记在配置目录的 `host.json`，设备令牌放系统钥匙串，不放网页存储；
//! - 配对（`POST /pair`）在壳里发，主机不用为桌面 App 开跨域；
//! - 系统通知、托盘图标上的待办数；关掉窗口后留在托盘里，连接不断，照样收通知；
//! - 开机启动（启动时只进托盘，不弹窗口）。
//!
//! 网页通过 `window.__TAURI__.core.invoke` 调下面这几条命令，别的原生接口一律不开放
//! （`capabilities/default.json` 只给了 `core:default`）。

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// 系统钥匙串里的服务名；账户名是主机地址。
const KEYRING_SERVICE: &str = "intelligent-position";
/// 配置目录里记着连哪台主机。
const HOST_FILE: &str = "host.json";
const TRAY: &str = "main";
/// 开机启动时带这个参数：只进托盘，不弹窗口。
const HIDDEN_ARG: &str = "--hidden";
/// 主机可能刚醒（比如容器冷启动），配对多等一会儿。
const PAIR_TIMEOUT: Duration = Duration::from_secs(60);

/// 存在 `host.json` 里的，不含令牌。
#[derive(Serialize, Deserialize)]
struct Saved {
    url: String,
}

/// 给网页的：主机地址和这台设备的令牌。
#[derive(Serialize)]
struct Host {
    url: String,
    token: Option<String>,
}

/// 连着的主机；还没连过时为 `null`，网页显示“连接主机”。
#[tauri::command]
fn get_host(app: AppHandle) -> Result<Option<Host>, String> {
    let Some(saved) = read_saved(&app)? else {
        return Ok(None);
    };
    let token = match entry(&saved.url)?.get_password() {
        Ok(token) => Some(token),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => return Err(format!("读系统钥匙串失败：{e}")),
    };
    Ok(Some(Host { url: saved.url, token }))
}

/// 用 6 位配对码换这台设备的令牌（主机上 `ip device pair` 或别的控制台“设备”页生成）。
#[tauri::command]
async fn pair(app: AppHandle, url: String, code: String) -> Result<Host, String> {
    let url = normalize(&url)?;
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err("配对码是 6 位数字".into());
    }
    // 配对码只能用一次：先确认钥匙串存得进去，免得换到了令牌却存不下，白白用掉配对码。
    check_keyring()?;
    let client = reqwest::Client::builder()
        .timeout(PAIR_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;
    let res = client
        .post(format!("{url}/pair"))
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        .map_err(|e| format!("连不上主机：{}", e.without_url()))?;
    let status = res.status();
    let body: serde_json::Value = res.json().await.unwrap_or_default();
    if !status.is_success() {
        return Err(body["error"]
            .as_str()
            .map_or_else(|| format!("配对失败（{status}）"), str::to_string));
    }
    let token = body["token"].as_str().ok_or("主机的回复里没有令牌")?;
    save(&app, url, token)
}

/// 直接填访问令牌（主机的共用令牌，或者已有的设备令牌）。
#[tauri::command]
fn save_token(app: AppHandle, url: String, token: String) -> Result<Host, String> {
    let url = normalize(&url)?;
    let token = token.trim();
    if token.is_empty() {
        return Err("填 6 位配对码或访问令牌".into());
    }
    save(&app, url, token)
}

/// 断开这台主机：删掉钥匙串里的令牌和 `host.json`。
#[tauri::command]
fn forget_host(app: AppHandle) -> Result<(), String> {
    if let Some(saved) = read_saved(&app)? {
        delete_token(&saved.url)?;
    }
    match std::fs::remove_file(host_file(&app)?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("删除 {HOST_FILE} 失败：{e}")),
    }
}

/// 弹一条系统通知。哪些事要通知、要不要预览，主机已按这台设备的设置筛过（`docs/ip/m2-spec.md` D-32）。
#[tauri::command]
fn notify(app: AppHandle, title: String, body: String) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| format!("弹通知失败：{e}"))
}

/// 用系统浏览器打开链接；只认 http、https、mailto。
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    let parsed = url::Url::parse(&url).map_err(|_| "不是网址".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https" | "mailto") {
        return Err("只打开 http、https、mailto 链接".into());
    }
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|e| format!("打开链接失败：{e}"))
}

/// 托盘上显示待办数（和控制台“待办”页的角标一样）。
#[tauri::command]
fn set_pending(app: AppHandle, count: u32) {
    let Some(tray) = app.tray_by_id(TRAY) else {
        return;
    };
    let tip = if count == 0 {
        "intelligent-position".to_string()
    } else {
        format!("intelligent-position：{count} 件待办")
    };
    let _ = tray.set_tooltip(Some(&tip));
    // Windows 的托盘图标旁边不能显示文字，只有提示。
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let _ = tray.set_title((count > 0).then(|| count.to_string()));
}

/// 主机地址只留 `scheme://主机[:端口]`。
fn normalize(input: &str) -> Result<String, String> {
    let input = input.trim().trim_end_matches('/');
    let url = url::Url::parse(input).map_err(|_| format!("{input} 不是网址，例如 https://ip.example.com"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("主机地址要以 https:// 或 http:// 开头".into());
    }
    if url.host_str().is_none_or(str::is_empty) || !url.username().is_empty() || url.password().is_some() {
        return Err("主机地址缺少主机名，或者带了用户名".into());
    }
    if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
        return Err("主机地址只写到域名和端口，例如 https://ip.example.com".into());
    }
    Ok(url.origin().ascii_serialization())
}

fn entry(url: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, url).map_err(|e| format!("系统钥匙串不可用：{e}"))
}

fn keyring_error(e: keyring::Error) -> String {
    format!("令牌存不进系统钥匙串（Linux 上要有 GNOME 钥匙串或 KWallet 在运行）：{e}")
}

/// 往钥匙串里写一条再删掉，看存不存得进去。
fn check_keyring() -> Result<(), String> {
    let probe = entry("probe")?;
    probe.set_password("ok").map_err(keyring_error)?;
    let _ = probe.delete_credential();
    Ok(())
}

fn delete_token(url: &str) -> Result<(), String> {
    match entry(url)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("从系统钥匙串删令牌失败：{e}")),
    }
}

fn save(app: &AppHandle, url: String, token: &str) -> Result<Host, String> {
    entry(&url)?.set_password(token).map_err(keyring_error)?;
    // 换了主机：旧主机的令牌不留。
    if let Some(old) = read_saved(app)?
        && old.url != url
    {
        delete_token(&old.url)?;
    }
    let path = host_file(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建配置目录失败：{e}"))?;
    }
    let text = serde_json::to_string(&Saved { url: url.clone() }).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("写 {HOST_FILE} 失败：{e}"))?;
    Ok(Host {
        url,
        token: Some(token.to_string()),
    })
}

fn host_file(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("找不到配置目录：{e}"))?;
    Ok(dir.join(HOST_FILE))
}

fn read_saved(app: &AppHandle) -> Result<Option<Saved>, String> {
    let path = host_file(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{HOST_FILE} 格式不对：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读 {HOST_FILE} 失败：{e}")),
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn tray(app: &tauri::App) -> tauri::Result<()> {
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let show = MenuItem::with_id(app, "show", "打开控制台", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", "开机启动", true, autostart_on, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&show, &autostart, &separator, &quit])?;
    let toggle = autostart.clone();
    let mut builder = TrayIconBuilder::with_id(TRAY)
        .tooltip("intelligent-position")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "autostart" => {
                let launcher = app.autolaunch();
                let was_on = launcher.is_enabled().unwrap_or(false);
                let result = if was_on { launcher.disable() } else { launcher.enable() };
                if let Err(e) = result {
                    eprintln!("改开机启动失败：{e}");
                }
                let _ = toggle.set_checked(launcher.is_enabled().unwrap_or(was_on));
            }
            "quit" => app.exit(0),
            _ => {}
        })
        // Linux 的托盘图标收不到点击，只能用菜单。
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        // 再打开一次 App 时，把已经开着的窗口叫出来，不另起一个。要第一个注册。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_host,
            pair,
            save_token,
            forget_host,
            notify,
            open_url,
            set_pending
        ])
        .setup(|app| {
            tray(app)?;
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show_main(app.handle());
            }
            Ok(())
        })
        // 关窗口只是藏到托盘：连接不断，照样收通知。从托盘菜单“退出”才真的退出。
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("启动桌面 App 失败")
        .run(|_app, _event| {
            // macOS 上点程序坞图标：窗口藏着时叫出来。
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                show_main(_app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn host_addresses_keep_only_the_origin() {
        assert_eq!(normalize("https://ip.example.com/").unwrap(), "https://ip.example.com");
        assert_eq!(normalize(" https://IP.example.com ").unwrap(), "https://ip.example.com");
        assert_eq!(normalize("http://127.0.0.1:7878").unwrap(), "http://127.0.0.1:7878");
        assert_eq!(
            normalize("https://ip.example.com:443").unwrap(),
            "https://ip.example.com"
        );
        for bad in [
            "ip.example.com",
            "ftp://ip.example.com",
            "https://ip.example.com/console",
            "https://ip.example.com/?token=x",
            "https://user:pw@ip.example.com",
            "https://",
        ] {
            assert!(normalize(bad).is_err(), "{bad} 应该不认");
        }
    }
}
