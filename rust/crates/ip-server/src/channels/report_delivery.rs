//! 识别回复里的报告，只改写发到外部会话的文字。
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::LazyLock;

use ip_connectors::{ConversationKey, Outbound};
use ip_core::AgentId;
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::reports::{read_in, valid_name};

// 文件名分隔符和前后边界与控制台 markdown.js 一致。
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"reports/([^\s/\\`'"<>()（）\[\]{}|*，。；：、！？]+?\.md)(?:[^A-Za-z0-9_.-]|$)"#).unwrap()
});
static LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[([^\[\]\n]*)\]\((reports/[^()\n]+)\)").unwrap());

fn names(text: &str) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    let mut offset = 0;
    while let Some(c) = PATH.captures_at(text, offset) {
        let m = c.get(0).unwrap();
        let filename = c.get(1).unwrap();
        offset = filename.end();
        if m.start().checked_sub(1).is_some_and(|i| {
            let previous = text.as_bytes()[i];
            previous.is_ascii_alphanumeric() || b"_./-".contains(&previous)
        }) {
            continue;
        }
        let name = filename.as_str();
        if valid_name(name).is_err() {
            continue;
        }
        if seen.insert(name) {
            names.push(name);
        }
    }
    names
}

fn rewrite(text: &str, mentioned: &[&str], attached: &HashSet<&str>) -> String {
    LINK.replace_all(text, |c: &regex::Captures<'_>| {
        let name = &c[2]["reports/".len()..];
        if attached.contains(name) {
            format!("{}（附件 `{name}`）", &c[1])
        } else if mentioned.contains(&name) {
            format!("{}（`{}`）", &c[1], &c[2])
        } else {
            c[0].to_string()
        }
    })
    .into_owned()
}

pub(super) fn prepare(
    text: &str,
    workspaces: &HashMap<AgentId, PathBuf>,
    author: &AgentId,
    conversation: &ConversationKey,
) -> (String, Vec<(String, Outbound)>) {
    let mentioned = names(text);
    let mut attached = HashSet::new();
    let mut files = Vec::new();
    for &name in &mentioned {
        if files.len() == 3 {
            break;
        }
        let path = format!("reports/{name}");
        let report = read_in(workspaces, author, &path).ok().or_else(|| {
            let mut others = workspaces
                .keys()
                .filter(|id| *id != author)
                .filter_map(|id| read_in(workspaces, id, &path).ok());
            let first = others.next()?;
            others.next().is_none().then_some(first)
        });
        let Some(r) = report.filter(|r| !r.truncated && r.text.len() as u64 <= crate::reports::MAX_READ_BYTES) else {
            continue;
        };
        let title = r
            .text
            .lines()
            .filter_map(|line| line.strip_prefix("# "))
            .map(str::trim)
            .find(|s| !s.is_empty())
            .unwrap_or(name)
            .to_owned();
        let digest = Sha256::digest(r.text.as_bytes());
        let hash = u64::from_be_bytes(digest[..8].try_into().expect("SHA-256 前八字节"));
        files.push((
            format!("report:{}:{name}:{hash:016x}", conversation.0),
            Outbound::Report {
                conversation: conversation.clone(),
                filename: name.to_owned(),
                title,
                markdown: r.text,
            },
        ));
        attached.insert(name);
    }
    (rewrite(text, &mentioned, &attached), files)
}

/// 基准使用与投递器相同的识别和改写逻辑。
pub fn benchmark_rewrite(text: &str) -> String {
    let mentions = names(text);
    rewrite(text, &mentions, &mentions.iter().copied().take(3).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_safe_unique_reports_in_mention_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = HashMap::new();
        for id in ["a", "b", "c"] {
            let path = dir.path().join(id);
            std::fs::create_dir_all(path.join("reports")).unwrap();
            ws.insert(AgentId::from(id), path);
        }
        let write = |id: &str, name: &str, body: &str| {
            std::fs::write(ws[&AgentId::from(id)].join("reports").join(name), body).unwrap()
        };
        write("a", "first.md", "前言\n# 作者的标题");
        write("b", "first.md", "其他员工");
        write(
            "a",
            "large.md",
            &"x".repeat((crate::reports::MAX_READ_BYTES + 1) as usize),
        );
        write("b", "ambiguous.md", "b");
        write("c", "ambiguous.md", "c");
        write("b", "second.md", "唯一员工");
        write("a", "third.md", "3");
        write("a", "fourth.md", "4");
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            ws[&AgentId::from("a")].join("reports/first.md"),
            ws[&AgentId::from("a")].join("reports/link.md"),
        )
        .unwrap();
        let msg = "/tmp/reports/fourth.md reports/../first.md reports/.hidden.md reports/link.md [大](reports/large.md) [缺](reports/no.md) [歧义](reports/ambiguous.md) reports/first.md `reports/second.md` [重](reports/first.md) reports/third.md [多](reports/fourth.md)";
        let (body, files) = prepare(msg, &ws, &"a".into(), &"room".into());
        assert_eq!(files.len(), 3);
        let reports: Vec<_> = files
            .iter()
            .map(|(_, out)| match out {
                Outbound::Report { filename, title, .. } => (filename.as_str(), title.as_str()),
                _ => panic!(),
            })
            .collect();
        assert_eq!(
            reports,
            [
                ("first.md", "作者的标题"),
                ("second.md", "second.md"),
                ("third.md", "third.md")
            ]
        );
        assert!(body.contains("大（`reports/large.md`） 缺（`reports/no.md`） 歧义（`reports/ambiguous.md`）"));
        assert!(body.ends_with("重（附件 `first.md`） reports/third.md 多（`reports/fourth.md`）"));
    }
}
