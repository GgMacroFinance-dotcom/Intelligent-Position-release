//! 密钥脱敏：对话内容交给模型提炼之前、提炼结果存进库之前各过一遍。
//!
//! 记忆会进以后每一轮的提示词，所以宁可多抹。这里只认形状明显的密钥，认不出来的靠提示词里的要求兜底。

use std::sync::LazyLock;

use regex::Regex;

const MARK: &str = "[已脱敏]";

/// 形状固定的令牌：整段换掉。
static TOKENS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // OpenAI、Anthropic 等 `sk-` 开头的 key。
        r"\bsk-[A-Za-z0-9_\-]{16,}",
        // AWS access key id。
        r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b",
        // Slack 令牌。
        r"\bxox[abeprs]-[A-Za-z0-9\-]{10,}",
        // Slack 应用级令牌。
        r"\bxapp-[A-Za-z0-9\-]{10,}",
        // GitHub 令牌。
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})",
        // JWT：三段 base64url。
        r"\beyJ[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}\.[A-Za-z0-9_\-]{8,}",
        // PEM 私钥。
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("内置的脱敏规则是合法的正则"))
    .collect()
});

/// `Authorization: Bearer xxx`：留下 `Bearer`，抹掉后面的值。
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/=\-]{8,}").expect("内置的脱敏规则是合法的正则"));

/// `名字 = 值`、`名字: 值`，名字里带 key、token、secret、password 之类：留下名字，抹掉值。
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([A-Za-z0-9_\-]*(?:api[_\-]?key|apikey|access[_\-]?key|token|secret|password|passwd|pwd)[A-Za-z0-9_\-]*)(["']?\s*[:=]\s*["']?)([^\s"',;]{6,})"#,
    )
    .expect("内置的脱敏规则是合法的正则")
});

/// URL 里的用户名密码：`https://user:pass@host`。
static URL_USERINFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b([a-z][a-z0-9+.\-]*://)[^\s/@:]+:[^\s/@]+@").expect("内置的脱敏规则是合法的正则")
});

/// 抹掉文本里形状明显的密钥。
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for token in TOKENS.iter() {
        if token.is_match(&out) {
            out = token.replace_all(&out, MARK).into_owned();
        }
    }
    if BEARER.is_match(&out) {
        out = BEARER.replace_all(&out, format!("${{1}}{MARK}")).into_owned();
    }
    if ASSIGNMENT.is_match(&out) {
        out = ASSIGNMENT
            .replace_all(&out, |c: &regex::Captures<'_>| {
                // 值已经被上面的规则换掉时不再套一层。
                if c[3].starts_with(MARK) {
                    c[0].to_string()
                } else {
                    format!("{}{}{MARK}", &c[1], &c[2])
                }
            })
            .into_owned();
    }
    if URL_USERINFO.is_match(&out) {
        out = URL_USERINFO.replace_all(&out, format!("${{1}}{MARK}@")).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::redact;

    #[test]
    fn obvious_secrets_are_removed_and_ordinary_text_is_kept() {
        for (input, gone) in [
            ("key 是 sk-proj-abcdefghijklmnop1234 记一下", "abcdefghijklmnop"),
            ("Authorization: Bearer abc.DEF-123_456xyz", "abc.DEF-123_456xyz"),
            ("export OPENAI_API_KEY=live_9f8e7d6c5b4a", "live_9f8e7d6c5b4a"),
            ("\"slack_bot_token\": \"xoxb-1234567890-abcdefg\"", "1234567890-abcdefg"),
            ("password: hunter2hunter2", "hunter2hunter2"),
            ("AKIAIOSFODNN7EXAMPLE", "IOSFODNN7"),
            ("ghp_abcdefghijklmnopqrstuvwxyz0123", "abcdefghijklmnop"),
            ("postgres://admin:s3cretpw@db.internal/app", "s3cretpw"),
            (
                "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N",
                "dozjgNryP4J3",
            ),
            (
                "-----BEGIN RSA PRIVATE KEY-----\nMIIEow\nabc\n-----END RSA PRIVATE KEY-----",
                "MIIEow",
            ),
        ] {
            let out = redact(input);
            assert!(!out.contains(gone), "{input} → {out}");
            assert!(out.contains("[已脱敏]"), "{input} → {out}");
        }
        // 名字留着，方便以后的人知道当时在说哪个配置项。
        assert_eq!(redact("MACRO_API_KEY=abcdef123456"), "MACRO_API_KEY=[已脱敏]");
        assert_eq!(redact("Bearer abcdefgh12345678"), "Bearer [已脱敏]");

        for plain in [
            "美国 10 年期国债收益率 4.88%，数据截至 2026-11-09。",
            "用 get_fred_series 取 DGS10，token 用量 1200。",
            "报告写到 reports/2026-11-11-cpi.md，密钥放在环境变量 MACRO_API_KEY 里。",
            "https://fred.stlouisfed.org/series/DGS10",
            "password 这个词本身不是密钥",
        ] {
            assert_eq!(redact(plain), plain);
        }
    }
}
