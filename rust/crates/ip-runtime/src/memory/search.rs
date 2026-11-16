//! 在一个岗位的对话记忆里找东西（`docs/ip/context-memory-design.md` 5.5）。
//!
//! 筛选在数据库里做（`Store::search_thread_memories`，逐份子串比对）。一个岗位的对话记忆最多几百到几千份、
//! 每份几百字，这样就够快（见 `ip bench store`），所以没有另建全文索引。这里负责拆词和截片段。

use ip_store::ThreadMemory;

/// 片段在命中处前后各留多少字。
const SNIPPET_BEFORE: usize = 30;
const SNIPPET_AFTER: usize = 90;
/// 一次查询最多用几个词。
const MAX_TERMS: usize = 8;

/// 一条命中。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit<'a> {
    pub memory: &'a ThreadMemory,
    /// 第一个词命中处附近的一段。
    pub snippet: String,
}

/// 按空格把查询拆成词（英文转小写，最多 [`MAX_TERMS`] 个）。
pub fn terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .take(MAX_TERMS)
        .collect()
}

/// 每个词都要出现在标题或正文里（英文不分大小写）。保持传入的顺序，最多 `limit` 条。
pub fn search<'a>(memories: &'a [ThreadMemory], terms: &[String], limit: usize) -> Vec<Hit<'a>> {
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for memory in memories {
        if hits.len() >= limit {
            break;
        }
        let Some(summary) = memory.summary.as_deref() else {
            continue;
        };
        // 只转英文字母的大小写，字节位置和原文一致。
        let body = summary.to_ascii_lowercase();
        let slug = memory.slug.as_deref().unwrap_or_default().to_ascii_lowercase();
        if !terms
            .iter()
            .all(|t| body.contains(t.as_str()) || slug.contains(t.as_str()))
        {
            continue;
        }
        let at = terms
            .iter()
            .find_map(|t| body.find(t.as_str()).map(|byte| summary[..byte].chars().count()))
            .unwrap_or(0);
        hits.push(Hit {
            memory,
            snippet: snippet(summary, at),
        });
    }
    hits
}

fn snippet(text: &str, at_char: usize) -> String {
    let total = text.chars().count();
    let start = at_char.min(total).saturating_sub(SNIPPET_BEFORE);
    let end = (at_char + SNIPPET_AFTER).min(total);
    let body: String = text
        .chars()
        .skip(start)
        .take(end - start)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body.trim(),
        if end < total { "…" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::{search, terms};
    use ip_store::{MemoryStatus, ThreadMemory};

    fn memory(thread: &str, slug: &str, summary: &str) -> ThreadMemory {
        ThreadMemory {
            role_id: "researcher".into(),
            thread_id: thread.into(),
            source_updated_ms: 1,
            generated_ms: 1,
            status: MemoryStatus::Ok,
            slug: Some(slug.into()),
            summary: Some(summary.into()),
            attempts: 0,
            retry_after_ms: None,
        }
    }

    #[test]
    fn every_term_must_match_title_or_body() {
        let long = format!(
            "{}收益率用 get_fred_series 取 DGS10。{}",
            "前".repeat(50),
            "后".repeat(200)
        );
        let memories = [
            memory("t1", "收益率口径", &long),
            memory("t2", "CPI 点评", "张三要求带数据截至日期。\n第二行。"),
            memory("t3", "空的", ""),
        ];
        let ids = |q: &str, limit| {
            search(&memories, &terms(q), limit)
                .iter()
                .map(|h| h.memory.thread_id.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("dgs10", 10), ["t1"]);
        assert_eq!(ids("cpi 截至日期", 10), ["t2"], "标题和正文各中一个词也算");
        assert_eq!(ids("cpi dgs10", 10), Vec::<String>::new());
        assert_eq!(ids("  ", 10), Vec::<String>::new());
        assert_eq!(ids("。", 1).len(), 1);

        // 片段是命中处附近的一段，两头省略，换行变空格。
        let hit = &search(&memories, &terms("DGS10"), 10)[0];
        assert!(
            hit.snippet.starts_with('…') && hit.snippet.ends_with('…'),
            "{}",
            hit.snippet
        );
        assert!(hit.snippet.contains("get_fred_series 取 DGS10") && hit.snippet.chars().count() < 130);
        assert_eq!(
            search(&memories, &terms("第二行"), 10)[0].snippet,
            "张三要求带数据截至日期。 第二行。"
        );
    }
}
