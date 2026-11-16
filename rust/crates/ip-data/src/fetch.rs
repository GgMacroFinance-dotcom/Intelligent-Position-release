//! 从公开数据源拉取序列，写进 `csv_dir` 数据源的 CSV。由命令 `ip data pull` 调用，适合放进定时任务。
//!
//! 在 `catalog.yaml` 的序列上加 `fetch` 说明从哪里取：
//!
//! ```yaml
//!   - id: us_cpi
//!     name: 美国 CPI（季调）
//!     unit: 指数
//!     frequency: monthly
//!     fetch: { fred: { id: CPIAUCSL } }
//! ```
//!
//! - `fred`：美联储圣路易斯分行 FRED 的公开 CSV 下载，不需要 API key。按目录里的频率请求，
//!   原始数据更密时（比如日度利率放进月度序列）由 FRED 按 `aggregate`（avg / sum / eop，默认 avg）汇总。
//!
//! 国家统计局的数据不在这里拉：2026-11-15 起岗位直接用 `macro-mcp` 的统计局工具（`search_china_indicators`、
//! `browse_china_catalog`、`get_china_indicator`、`get_china_series`），同一份数据只留一个来源。
//! 旧的 `fetch: { nbs: ... }` 写法会报“unknown variant”。
//!
//! 拉到的数据和已有 CSV 合并（同一日期以新数据为准），写临时文件再改名，读的一方不会读到半个文件。

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::csv_dir::{CsvDir, SeriesInfo};
use crate::series::{Frequency, Series, fmt_value};

/// 序列的数据从哪里来。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Fetch {
    Fred(FredSpec),
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FredSpec {
    /// FRED 序列 id，如 `CPIAUCSL`。
    pub id: String,
    /// 原始数据比目录频率更密时怎么汇总：avg / sum / eop。
    #[serde(default)]
    pub aggregate: Option<String>,
}

/// 一次拉取的结果。
#[derive(Clone, Debug)]
pub struct Pulled {
    pub series: Series,
    /// 数据源对这个指标的叫法（有的话），用来核对代码。
    pub upstream_name: Option<String>,
}

pub struct Fetcher {
    http: reqwest::Client,
    fred_base: String,
}

impl Fetcher {
    pub fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            // 数据下载很小，用 HTTP/1.1 兼容性最好（部分代理和防火墙对 HTTP/2 处理有问题）。
            .http1_only()
            // 带上项目地址表明身份；FRED 会拒绝不认识的简短客户端名。
            .user_agent(concat!(
                "intelligent-position/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/GgMacroFinance-dotcom/intelligent-position)"
            ))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            http,
            fred_base: "https://fred.stlouisfed.org".into(),
        })
    }

    /// 测试时指向本地假服务。
    pub fn with_fred_base(mut self, fred: &str) -> Self {
        self.fred_base = fred.trim_end_matches('/').into();
        self
    }

    pub async fn pull(&self, fetch: &Fetch, freq: Frequency) -> Result<Pulled, String> {
        match fetch {
            Fetch::Fred(spec) => {
                let mut url = format!("{}/graph/fredgraph.csv?id={}", self.fred_base, encode(&spec.id));
                let fq = match freq {
                    Frequency::Daily => None,
                    Frequency::Monthly => Some("Monthly"),
                    Frequency::Quarterly => Some("Quarterly"),
                };
                if let Some(fq) = fq {
                    let fam = spec.aggregate.as_deref().unwrap_or("avg");
                    if !matches!(fam, "avg" | "sum" | "eop") {
                        return Err(format!("aggregate 只能是 avg / sum / eop，现在是 {fam}"));
                    }
                    url.push_str(&format!("&fq={fq}&fam={fam}"));
                }
                let text = self.get(&url).await?;
                Ok(Pulled {
                    series: parse_fred(&text, freq)?,
                    upstream_name: None,
                })
            }
        }
    }

    async fn get(&self, url: &str) -> Result<String, String> {
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| format!("请求失败：{}", chain(&e)))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("读取响应失败：{e}"))?;
        if !status.is_success() {
            return Err(format!("返回 {status}：{}", plain_excerpt(&text)));
        }
        Ok(text)
    }
}

/// 错误连同它的各层原因，网络错误的真正原因（证书、DNS、连接被拒）通常在里层。
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(s) = source {
        out.push_str(&format!("：{s}"));
        source = s.source();
    }
    out
}

/// 错误页面去掉 HTML 标签、合并空白，只留前 120 个字。
fn plain_excerpt(text: &str) -> String {
    let mut plain = String::new();
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                plain.push(' ');
            }
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(120)
        .collect()
}

/// 百分号编码 URL 查询参数里的值。
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// FRED CSV：表头一行，之后 `YYYY-MM-DD,数值`；缺失值是空或 `.`。
pub fn parse_fred(text: &str, freq: Frequency) -> Result<Series, String> {
    let mut points = Vec::new();
    for (i, line) in text.lines().enumerate().skip(1) {
        let Some((date, value)) = line.split_once(',') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || value == "." {
            continue;
        }
        let date = fred_date(date.trim(), freq).ok_or_else(|| format!("FRED 第 {} 行的日期 {date} 无法识别", i + 1))?;
        let v: f64 = value
            .parse()
            .map_err(|_| format!("FRED 第 {} 行的数值 {value} 不是数字", i + 1))?;
        points.push((date, v));
    }
    if points.is_empty() {
        return Err("FRED 没有返回数据，检查序列 id".into());
    }
    points.sort_by(|a, b| a.0.cmp(&b.0));
    points.dedup_by(|a, b| a.0 == b.0);
    Ok(Series { points })
}

fn fred_date(date: &str, freq: Frequency) -> Option<String> {
    if !Frequency::Daily.valid(date) {
        return None;
    }
    Some(match freq {
        Frequency::Daily => date.to_string(),
        Frequency::Monthly => date[..7].to_string(),
        Frequency::Quarterly => {
            let m: u32 = date[5..7].parse().ok()?;
            format!("{}-Q{}", &date[..4], (m - 1) / 3 + 1)
        }
    })
}

/// 把新数据并进已有序列（同一日期以新数据为准），原子写回 CSV。返回合并后的点数。
pub fn merge_into(dir: &CsvDir, info: &SeriesInfo, new: &Series) -> Result<usize, String> {
    let mut merged: std::collections::BTreeMap<String, f64> = dir.read(info)?.points.into_iter().collect();
    for (d, v) in &new.points {
        merged.insert(d.clone(), *v);
    }
    let mut text = String::from("date,value\n");
    for (d, v) in &merged {
        text.push_str(&format!("{d},{}\n", fmt_value(*v)));
    }
    write_atomic(&dir.dir().join(format!("{}.csv", info.id)), &text)?;
    Ok(merged.len())
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("csv.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("替换 {} 失败：{e}", path.display()))
}
