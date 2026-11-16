//! 全文索引用的分词。
//!
//! 中日韩文字切成重叠的二字词（"销售报表" → 销售 售报 报表），其余文字原样交给 SQLite 的
//! unicode61 分词器。不用词典，所以不占内存、不需要加载时间，新词和人名也能搜到；
//! 代价是精度略低于词典分词。实测 jieba-rs 加载词典要 97 ms、33 MB 常驻内存，
//! 超出 harness 40 MB 的内存目标，所以默认不用。

/// 把要入索引的文字改写成 unicode61 能正确切分的形式。
pub fn index_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut run: Vec<char> = Vec::new();
    for ch in text.chars() {
        if is_cjk(ch) {
            run.push(ch);
        } else {
            flush_run(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush_run(&mut run, &mut out);
    out
}

/// 把用户输入的搜索词改写成 FTS5 查询。每个空格分开的词都必须出现（AND），
/// 一个词内部按短语匹配。没有可搜的内容时返回 `None`。
pub fn match_query(query: &str) -> Option<String> {
    let mut clauses = Vec::new();
    for term in query.split_whitespace() {
        let cleaned: String = term.chars().filter(|ch| *ch != '"').collect();
        let mut chars = cleaned.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && is_cjk(ch)
        {
            // 单个汉字：匹配以它开头的二字词，或独立成词的它本身。
            clauses.push(format!("\"{ch}\"*"));
            continue;
        }
        let tokens = index_text(&cleaned);
        if tokens.chars().any(char::is_alphanumeric) {
            clauses.push(format!("\"{}\"", tokens.trim()));
        }
    }
    (!clauses.is_empty()).then(|| clauses.join(" "))
}

fn flush_run(run: &mut Vec<char>, out: &mut String) {
    match run.len() {
        0 => {}
        1 => {
            out.push(' ');
            out.push(run[0]);
            out.push(' ');
        }
        _ => {
            for pair in run.windows(2) {
                out.push(' ');
                out.push(pair[0]);
                out.push(pair[1]);
            }
            out.push(' ');
        }
    }
    run.clear();
}

/// 汉字、日文假名、韩文。
fn is_cjk(ch: char) -> bool {
    matches!(ch,
        '\u{3040}'..='\u{30FF}'       // 平假名、片假名
        | '\u{3400}'..='\u{4DBF}'     // 扩展 A
        | '\u{4E00}'..='\u{9FFF}'     // 基本区
        | '\u{AC00}'..='\u{D7AF}'     // 韩文音节
        | '\u{F900}'..='\u{FAFF}'     // 兼容汉字
        | '\u{20000}'..='\u{2EBEF}'   // 扩展 B 到 F
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cjk_into_overlapping_pairs() {
        assert_eq!(index_text("销售报表"), " 销售 售报 报表 ");
        assert_eq!(index_text("查 report 合同"), " 查  report  合同 ");
    }

    #[test]
    fn builds_phrase_queries() {
        assert_eq!(match_query("销售报表").as_deref(), Some("\"销售 售报 报表\""));
        assert_eq!(match_query("合同 report").as_deref(), Some("\"合同\" \"report\""));
        assert_eq!(match_query("合").as_deref(), Some("\"合\"*"));
        assert_eq!(match_query("  \"\" ，").as_deref(), None);
    }
}
