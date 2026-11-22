//! 全文索引用的分词。
//!
//! 中日韩文字切成重叠的二字词（"销售报表" → 销售 售报 报表），其余文字按字母数字连续段切开、转小写。
//! 不用词典，所以不占内存、不需要加载时间，新词和人名也能搜到；代价是精度略低于词典分词。
//! 实测 jieba-rs 加载词典要 97 ms、33 MB 常驻内存，超出 harness 40 MB 的内存目标，所以不用。
//!
//! 切好的词直接拼成 PostgreSQL 的 `tsvector`、`tsquery` 字面量交给数据库，不经过 `to_tsvector`：
//! PostgreSQL 自带的分词器认不认中文要看库的 locale，客户的库不一定对。这样也不需要任何扩展。

/// 单个词的最大字节数；PostgreSQL 的上限是 2047，超长的截断。
const MAX_TOKEN_BYTES: usize = 200;
/// `tsvector` 里位置的上限是 16383，再往后的词都记在这个位置上。
const MAX_POSITION: usize = 16_383;

/// 把文字切成词，按出现顺序。
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cjk: Vec<char> = Vec::new();
    let mut word = String::new();
    for ch in text.chars() {
        if is_cjk(ch) {
            flush_word(&mut word, &mut out);
            cjk.push(ch);
        } else if ch.is_alphanumeric() {
            flush_cjk(&mut cjk, &mut out);
            word.extend(ch.to_lowercase());
        } else {
            flush_word(&mut word, &mut out);
            flush_cjk(&mut cjk, &mut out);
        }
    }
    flush_word(&mut word, &mut out);
    flush_cjk(&mut cjk, &mut out);
    out
}

/// 要入索引的文字写成 `tsvector` 字面量（`'销售':1 '售报':2`）。没有可搜的词时返回 `None`。
pub fn index_text(text: &str) -> Option<String> {
    let words = tokens(text);
    if words.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(text.len() * 3);
    for (i, word) in words.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        push_lexeme(&mut out, word);
        out.push(':');
        out.push_str(&(i + 1).min(MAX_POSITION).to_string());
    }
    Some(out)
}

/// 把用户输入的搜索词改写成 `tsquery` 字面量。每个空格分开的词都必须出现（`&`），
/// 一个词内部按短语匹配（`<->`）。没有可搜的内容时返回 `None`。
pub fn match_query(query: &str) -> Option<String> {
    let mut clauses = Vec::new();
    for term in query.split_whitespace() {
        let mut chars = term.chars();
        if let (Some(ch), None) = (chars.next(), chars.next())
            && is_cjk(ch)
        {
            // 单个汉字：匹配以它开头的二字词，或独立成词的它本身。
            let mut clause = String::new();
            push_lexeme(&mut clause, &ch.to_string());
            clause.push_str(":*");
            clauses.push(clause);
            continue;
        }
        let words = tokens(term);
        if words.is_empty() {
            continue;
        }
        let mut clause = String::new();
        for (i, word) in words.iter().enumerate() {
            if i > 0 {
                clause.push_str(" <-> ");
            }
            push_lexeme(&mut clause, word);
        }
        if words.len() > 1 {
            clause = format!("({clause})");
        }
        clauses.push(clause);
    }
    (!clauses.is_empty()).then(|| clauses.join(" & "))
}

/// 词只含字母数字，不会有引号和反斜杠；仍按字面量规则转义，免得以后改了切分规则出错。
fn push_lexeme(out: &mut String, word: &str) {
    out.push('\'');
    for ch in word.chars() {
        if ch == '\'' || ch == '\\' {
            out.push(ch);
        }
        out.push(ch);
    }
    out.push('\'');
}

fn flush_word(word: &mut String, out: &mut Vec<String>) {
    if word.is_empty() {
        return;
    }
    let mut end = word.len().min(MAX_TOKEN_BYTES);
    while !word.is_char_boundary(end) {
        end -= 1;
    }
    out.push(word[..end].to_string());
    word.clear();
}

fn flush_cjk(run: &mut Vec<char>, out: &mut Vec<String>) {
    match run.len() {
        0 => {}
        1 => out.push(run[0].to_string()),
        _ => out.extend(run.windows(2).map(|pair| pair.iter().collect())),
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
        assert_eq!(tokens("销售报表"), ["销售", "售报", "报表"]);
        assert_eq!(tokens("查 Report，合同"), ["查", "report", "合同"]);
        assert_eq!(tokens("gpt-6 发布"), ["gpt", "6", "发布"]);
        assert_eq!(index_text("查 report").as_deref(), Some("'查':1 'report':2"));
        assert_eq!(index_text("，。 "), None);
    }

    #[test]
    fn builds_phrase_queries() {
        assert_eq!(
            match_query("销售报表").as_deref(),
            Some("('销售' <-> '售报' <-> '报表')")
        );
        assert_eq!(match_query("合同 Report").as_deref(), Some("'合同' & 'report'"));
        assert_eq!(match_query("合").as_deref(), Some("'合':*"));
        assert_eq!(match_query("  \"\" ，").as_deref(), None);
    }

    #[test]
    fn truncates_long_words_on_a_char_boundary() {
        let long = "é".repeat(300);
        let words = tokens(&long);
        assert_eq!(words.len(), 1);
        assert!(words[0].len() <= MAX_TOKEN_BYTES);
    }
}
