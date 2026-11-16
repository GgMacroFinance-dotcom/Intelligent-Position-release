//! `/ws` 握手前的来源检查：挡住别的网页借浏览器连本机控制台（跨站 WebSocket、DNS rebinding）。
//!
//! 规则：
//! - 没配访问令牌时（只能监听本机地址），`Host` 必须是本机地址：`localhost`、`127.x.x.x` 或 `[::1]`。
//!   这一条挡 DNS rebinding：恶意域名解析到 127.0.0.1 后，浏览器发来的 `Host` 仍是那个域名。
//! - 请求带 `Origin`（浏览器总会带）时，它必须和 `Host` 同源，或者在配置的 `allowed_origins` 里。
//!   只比较 `Origin` 和 `Host` 挡不住 DNS rebinding，所以要和上一条一起用。
//! - 不带 `Origin` 的是非浏览器客户端：配了令牌就要令牌；没配令牌时只有本机进程连得上，
//!   视为和服务同一信任范围。

use axum::http::uri::{Authority, Scheme};
use axum::http::{HeaderMap, Uri, header};

/// 规范化后的来源：小写的 `scheme://host[:port]`，默认端口省略。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    https: bool,
    host: String,
    port: u16,
}

impl Origin {
    /// 解析 `Origin` 头或配置里的一项。只认 `http`、`https`，不能带路径、查询或用户名。
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let uri: Uri = text.parse().map_err(|_| format!("{text:?} 不是合法的来源"))?;
        let https = match uri.scheme() {
            Some(s) if *s == Scheme::HTTPS => true,
            Some(s) if *s == Scheme::HTTP => false,
            _ => return Err(format!("{text:?} 要以 http:// 或 https:// 开头")),
        };
        let has_path = uri.path_and_query().is_some_and(|p| p.as_str() != "/");
        let authority = uri.authority().ok_or_else(|| format!("{text:?} 缺少主机名"))?;
        if has_path || authority.as_str().contains('@') || authority.host().is_empty() {
            return Err(format!("{text:?} 只能写 scheme://主机[:端口]，不能带路径或用户名"));
        }
        Ok(Self {
            https,
            host: authority.host().to_ascii_lowercase(),
            port: authority.port_u16().unwrap_or(if https { 443 } else { 80 }),
        })
    }

    /// 和请求的 `Host` 是否同源。`Host` 没写端口时按本来源的协议取默认端口。
    fn same_as_host(&self, host: &Authority) -> bool {
        let port = host.port_u16().unwrap_or(if self.https { 443 } else { 80 });
        self.host.eq_ignore_ascii_case(host.host()) && self.port == port
    }
}

/// 检查不通过的原因，会原样回给客户端（403）。
pub fn check(headers: &HeaderMap, token_configured: bool, allowed: &[Origin]) -> Result<(), &'static str> {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<Authority>().ok());
    if !token_configured && !host.as_ref().is_some_and(is_loopback) {
        return Err("没有配置访问令牌时只接受本机地址（localhost、127.0.0.1、[::1]）");
    }
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    // `null`、乱码、非 http(s) 一律不认。
    let origin = origin
        .to_str()
        .ok()
        .and_then(|v| Origin::parse(v).ok())
        .ok_or("来源（Origin）不被信任")?;
    let same_origin = host.as_ref().is_some_and(|h| origin.same_as_host(h));
    if same_origin || allowed.contains(&origin) {
        Ok(())
    } else {
        Err("来源（Origin）不被信任")
    }
}

fn is_loopback(host: &Authority) -> bool {
    let name = host.host();
    if name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let bare = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')).unwrap_or(name);
    bare.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(host: Option<&str>, origin: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(v) = host {
            h.insert(header::HOST, HeaderValue::from_str(v).unwrap());
        }
        if let Some(v) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn parses_and_normalizes() {
        let a = Origin::parse("HTTPS://IP.Example.com").unwrap();
        assert_eq!(a, Origin::parse("https://ip.example.com:443/").unwrap());
        assert_ne!(a, Origin::parse("http://ip.example.com").unwrap());
        assert_eq!(
            Origin::parse("http://[::1]:7878").unwrap(),
            Origin {
                https: false,
                host: "[::1]".into(),
                port: 7878
            }
        );
        for bad in [
            "null",
            "",
            "ip.example.com",
            "ws://localhost:7878",
            "file:///x",
            "https://a.example/path",
            "https://u@a.example",
        ] {
            assert!(Origin::parse(bad).is_err(), "{bad} 应当不合法");
        }
    }

    #[test]
    fn without_token_only_loopback_hosts() {
        for host in [
            "127.0.0.1:7878",
            "localhost:7878",
            "LOCALHOST",
            "[::1]:7878",
            "127.1.2.3:80",
        ] {
            assert!(check(&headers(Some(host), None), false, &[]).is_ok(), "{host}");
        }
        // DNS rebinding：域名解析到本机，Host 和 Origin 都是攻击者的域名，彼此同源。
        let rebinding = headers(Some("evil.example:7878"), Some("http://evil.example:7878"));
        assert!(check(&rebinding, false, &[]).is_err());
        assert!(check(&headers(Some("192.168.1.2:7878"), None), false, &[]).is_err());
        assert!(check(&headers(None, None), false, &[]).is_err());
        // 配了令牌时 Host 不限，由令牌把关。
        assert!(check(&rebinding, true, &[]).is_ok());
    }

    #[test]
    fn origin_must_match_host_or_allow_list() {
        let host = Some("127.0.0.1:7878");
        assert!(check(&headers(host, Some("http://127.0.0.1:7878")), false, &[]).is_ok());
        for bad in [
            "http://untrusted.example",
            "http://127.0.0.1:3000",
            "http://localhost:7878",
            "null",
        ] {
            assert!(check(&headers(host, Some(bad)), false, &[]).is_err(), "{bad}");
            assert!(check(&headers(host, Some(bad)), true, &[]).is_err(), "{bad}（有令牌）");
        }
        // 反向代理把 Host 改成了上游地址，浏览器的来源是对外的域名：要写进 allowed_origins。
        let proxied = headers(host, Some("https://ip.example.com"));
        assert!(check(&proxied, true, &[]).is_err());
        let allowed = [Origin::parse("https://ip.example.com").unwrap()];
        assert!(check(&proxied, true, &allowed).is_ok());
        // 代理保留了 Host 时同源即可，默认端口写不写都一样。
        let kept = headers(Some("ip.example.com"), Some("https://ip.example.com"));
        assert!(check(&kept, true, &[]).is_ok());
        let kept = headers(Some("ip.example.com:443"), Some("https://ip.example.com"));
        assert!(check(&kept, true, &[]).is_ok());
    }
}
