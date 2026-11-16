//! 评测的检查项：都是纯函数，只看报告文字和工具结果，不调模型。
//!
//! 数字追溯的规则：
//! - 日期（2026-11-09、11月9日、2026年10月）、期限和时间范围（10年期、3个月、5 个交易日）、序列代码里的数字（DGS10）、
//!   指数基期（1982–84=100）、列表序号、标题行和链接地址里的数字都不算“数据”，不参与追溯。
//! - 报告里的一个数字，满足下面任一条就算能在工具结果里找到：
//!   1. 等于工具结果里的某个数，允许按报告写出的小数位四舍五入（4.876 写成 4.88）；
//!   2. 是工具结果里某个数乘以 10 的整数次幂（单位换算，0.252 十亿 = 2.52 亿）；
//!   3. 是同一条工具结果里两个数之差，或差的 100 倍（基点）。
//! - 比值、增速这类要自己算的数找不到来源，算“追溯不到”。岗位说明要求增速用工具算，不要心算。

use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

/// 报告里的一个数字。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Number {
    /// 报告里的原样写法（去掉千分位逗号前）。
    pub raw: String,
    pub value: f64,
    /// 小数位数，用来判断四舍五入。
    pub decimals: u32,
}

/// 一条工具结果里的数字。
pub struct ToolNumbers {
    numbers: Vec<Vec<f64>>,
}

static DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\d{4}\s*[-/.年]\s*\d{1,2}(?:\s*[-/.月]\s*\d{1,2}\s*日?)?\s*月?|\d{1,2}\s*月\s*\d{1,2}\s*日|\d{1,2}\s*月(?:份|末|初|中)?|\d{1,2}\s*日|\d{4}\s*年",
    )
    .unwrap()
});
static FULL_DATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{4}\s*[-/.年]\s*\d{1,2}(?:\s*[-/.月]\s*\d{1,2}\s*日?)?|\d{1,2}\s*月\s*\d{1,2}\s*日").unwrap()
});
static ISO_DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap());
static NOISE: LazyLock<Regex> = LazyLock::new(|| {
    // 行内代码、链接地址、裸网址。
    Regex::new(r"`[^`]*`|\]\([^)]*\)|https?://\S+").unwrap()
});
/// 指数基期，如 1982–84=100、1982-1984=100、2017=100：是指数的定义，不是观测数据。
static INDEX_BASE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{4}(?:\s*[-–—~至]\s*\d{2,4})?\s*=\s*100\b").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([A-Za-z_]*)(\d[\d,]*(?:\.\d+)?)([A-Za-z_]*)").unwrap());
static LIST_MARKER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:[-*]\s*)?\d+\s*(?:[、)）]|\.(?:\D|$))").unwrap());

/// 后面跟着这些字的数字是期限、时间范围或计数，不是数据（“最近 5 个交易日”里的 5 不是数据）。
const NOT_DATA_AFTER: [&str; 15] = [
    "年期",
    "年",
    "个月",
    "月",
    "周",
    "天",
    "个交易日",
    "交易日",
    "个工作日",
    "页",
    "条",
    "项",
    "次",
    "步",
    "句",
];
/// 数字后面可以接的英文单位。
const UNIT_SUFFIX: [&str; 4] = ["bp", "bps", "pp", "BP"];

/// 抽出报告或工具结果里的数据数字。标题行不看。
pub fn numbers(text: &str) -> Vec<Number> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with('#') {
            continue;
        }
        let line = LIST_MARKER.replace(line, " ");
        let line = NOISE.replace_all(&line, " ");
        let line = INDEX_BASE.replace_all(&line, " ");
        let line = DATE.replace_all(&line, " ");
        for caps in NUMBER.captures_iter(&line) {
            let (prefix, digits, suffix) = (&caps[1], &caps[2], &caps[3]);
            let after = line[caps.get(0).map_or(0, |m| m.end())..].trim_start();
            if !prefix.is_empty() || (!suffix.is_empty() && !UNIT_SUFFIX.contains(&suffix)) {
                continue; // DGS10、10Y、2s10s 这类代码
            }
            if suffix.is_empty() && NOT_DATA_AFTER.iter().any(|unit| after.starts_with(unit)) {
                continue;
            }
            let clean = digits.trim_end_matches(',').replace(',', "");
            let Ok(value) = clean.parse::<f64>() else { continue };
            let decimals = clean.split_once('.').map_or(0, |(_, frac)| frac.len() as u32);
            out.push(Number {
                raw: digits.trim_end_matches(',').to_owned(),
                value,
                decimals,
            });
        }
    }
    out
}

impl ToolNumbers {
    pub fn new<'a>(outputs: impl IntoIterator<Item = &'a str>) -> Self {
        Self {
            numbers: outputs
                .into_iter()
                .map(|output| numbers(output).into_iter().map(|n| n.value).collect())
                .collect(),
        }
    }

    /// 这个数能否在工具结果里找到（规则见模块说明）。
    pub fn traces(&self, number: &Number) -> bool {
        let v = number.value.abs();
        let tolerance = 0.5 * 10f64.powi(-(number.decimals as i32)) + 1e-9;
        let close = |t: f64| (t - v).abs() <= tolerance;
        for output in &self.numbers {
            for &t in output {
                let t = t.abs();
                if close(t) || (-12..=12).any(|k| k != 0 && close(t * 10f64.powi(k))) {
                    return true;
                }
            }
        }
        for output in &self.numbers {
            if output.len() > 400 {
                continue; // 太长的结果两两相减几乎什么都能凑出来
            }
            for (i, &a) in output.iter().enumerate() {
                for &b in &output[i + 1..] {
                    let diff = (a - b).abs();
                    if diff > 0.0 && (close(diff) || close(diff * 100.0)) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// 表格里含数据数字却没写日期的行（表头和分隔行不算）。
pub fn table_rows_without_date(report: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in report.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') {
            in_table = false;
            continue;
        }
        let is_separator = trimmed.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
        if !in_table {
            in_table = true; // 第一行是表头
            continue;
        }
        if is_separator {
            continue;
        }
        if !numbers(trimmed).is_empty() && !FULL_DATE.is_match(trimmed) {
            rows.push(trimmed.to_owned());
        }
    }
    rows
}

/// 报告里写到、工具结果里也出现过的观测日期（ISO 格式）。
pub fn shared_dates<'a>(report: &str, outputs: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let tool_dates: std::collections::BTreeSet<&str> = outputs
        .into_iter()
        .flat_map(|output| ISO_DATE.find_iter(output).map(|m| m.as_str()))
        .collect();
    let normalized = report.replace('/', "-");
    let mut found: Vec<String> = ISO_DATE
        .find_iter(&normalized)
        .map(|m| m.as_str().to_owned())
        .filter(|d| tool_dates.contains(d.as_str()))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// 标题行里没出现的必需小节。
pub fn missing_sections(report: &str, required: &[String]) -> Vec<String> {
    let headings: Vec<&str> = report.lines().filter(|l| l.trim_start().starts_with('#')).collect();
    required
        .iter()
        .filter(|name| !headings.iter().any(|h| h.contains(name.as_str())))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raws(text: &str) -> Vec<String> {
        numbers(text).into_iter().map(|n| n.raw).collect()
    }

    #[test]
    fn skips_dates_tenors_codes_and_list_markers() {
        let text = "1. 10年期收益率从 2026-11-03 的 4.95% 降至 11月9日的 4.88%，下降 7 个基点（DGS10，10Y）。\n\
                    ## 3 解读\n2026年10月 CPI 指数 337.476；准备金 2,930,088 百万美元；利差 -2bp；见 `reports/2026-11-10.md`";
        assert_eq!(raws(text), ["4.95", "4.88", "7", "337.476", "2,930,088", "2"]);
        // 时间范围里的数字也不是数据
        assert_eq!(
            raws("按最近 5 个交易日（20交易日均值 3.90%），共 3 个工作日缺值"),
            ["3.90"]
        );
        // 指数基期不是数据
        assert_eq!(
            raws("CPIAUCSL 337.476（指数，1982–84=100）；PCE 128.3（2017=100）；1982-1984 = 100"),
            ["337.476", "128.3"]
        );
    }

    #[test]
    fn traces_rounding_scaling_and_differences() {
        let tools = ToolNumbers::new([
            "DGS10, newest first:\n2026-11-09  4.88\n2026-11-03  4.95",
            "RRPONTSYD:\n2026-11-09  0.252",
            "CPIAUCSL 2026-10  337.476",
        ]);
        let check = |text: &str| numbers(text).iter().map(|n| tools.traces(n)).collect::<Vec<_>>();
        // 原值、四舍五入、单位换算、基点差、百分点差
        assert_eq!(check("4.88 4.9 2.52 7 0.07"), [true, true, true, true, true]);
        // 比值算出来的增速、凭空的数
        assert_eq!(check("0.35 5.12"), [false, false]);
        // 不同结果里的数不能相减
        assert_eq!(check("332.526"), [false]);
    }

    #[test]
    fn finds_table_rows_without_dates() {
        let report = "| 指标 | 数值 | 日期 |\n|---|---:|---|\n| DGS10 | 4.88% | 2026-11-09 |\n| SOFR | 3.88% | 最新 |\n| 说明 | 未取得 | — |\n";
        assert_eq!(table_rows_without_date(report), ["| SOFR | 3.88% | 最新 |"]);
    }

    #[test]
    fn shared_dates_and_sections() {
        let outputs = ["2026-11-09  4.88"];
        assert_eq!(
            shared_dates("截至 2026/11/09 的数据，2026-11-10 无", outputs),
            ["2026-11-09"]
        );
        let report = "# 题目\n## 结论\n## 关键数据表\n## 数据来源\n";
        let required = ["结论", "关键数据", "解读", "数据来源"].map(String::from);
        assert_eq!(missing_sections(report, &required), ["解读"]);
    }
}
