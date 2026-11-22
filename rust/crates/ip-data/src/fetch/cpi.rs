use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate};
use serde::Deserialize;

use super::{Fetcher, Pulled};
use crate::series::{Frequency, Series};

// 全国居民消费价格总指数（上年同月=100）。每次换基，目录和指标 id 都会变化。
const CATALOGS: &[(i32, i32, &str, &str)] = &[
    (
        1,
        2015,
        "954cfd7597e34b919ec71caf6aeead51",
        "4c1065dd4e984b25a21190c843551697",
    ),
    (
        2016,
        2020,
        "9d4eec43537742a7ab5d63db97fa2f51",
        "e5c318ffdbbc4d38898e52b52267eb25",
    ),
    (
        2021,
        2025,
        "809d2522b0fe4be89142650341b19083",
        "4ae9047687934a6390984c21d6ddab96",
    ),
    (
        2026,
        i32::MAX,
        "5c7452825c7c4dcba391db5ca7f335c5",
        "53180dfb9c14411ba4b762307c85920c",
    ),
];
const NATIONAL: &str = "000000000000";
pub(super) const URL: &str = "https://data.stats.gov.cn/dg/website/publicrelease/web/external/stream/esData";

fn month_label(code: &str) -> Option<String> {
    let digits = code.strip_suffix("MM")?;
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let label = format!("{}-{}", &digits[..4], &digits[4..]);
    Frequency::Monthly.valid(&label).then_some(label)
}

#[derive(Deserialize)]
struct Response {
    success: bool,
    data: Option<Vec<Period>>,
}

#[derive(Deserialize)]
struct Period {
    code: String,
    values: Vec<Cell>,
}

#[derive(Deserialize)]
struct Cell {
    #[serde(rename = "_id")]
    id: String,
    da: String,
    i_showname: Option<String>,
    value: Option<String>,
}

impl Fetcher {
    pub(crate) async fn pull_national_cpi(
        &self,
        offset: f64,
        count: u32,
        url: &str,
        now: NaiveDate,
    ) -> Result<Pulled, String> {
        if count == 0 {
            return Err("CPI 的 last 必须大于 0".into());
        }
        // 包含尚未公布的月份，最后按实际有值的月份取最近 count 期。
        let start = (i64::from(now.year()) * 12 + i64::from(now.month0()) - i64::from(count) - 2).max(12);
        let first_year = (start / 12) as i32;
        let dts = format!(
            "{first_year:04}{:02}MM-{:04}{:02}MM",
            start % 12 + 1,
            now.year(),
            now.month()
        );
        let mut months = BTreeMap::new();
        let mut upstream_name = None;
        for &(from, to, cid, id) in CATALOGS {
            if to < first_year || from > now.year() {
                continue;
            }
            let body = serde_json::json!({
                "cid": cid, "indicatorIds": [id], "daCatalogId": "", "das": [{"text":"全国","value":NATIONAL}],
                "showType": "1", "dts": [dts],
            });
            let text = self.send(self.http.post(url).json(&body)).await?;
            let response: Response =
                serde_json::from_str(&text).map_err(|e| format!("国家统计局 CPI 响应格式错误：{e}"))?;
            if !response.success {
                return Err("国家统计局 CPI 查询失败".into());
            }
            for period in response.data.ok_or("国家统计局 CPI 响应缺少 data")? {
                let Some(cell) = period.values.into_iter().find(|c| c.id == id && c.da == NATIONAL) else {
                    continue;
                };
                let raw = cell.value.as_deref().unwrap_or("").trim();
                if raw.is_empty() {
                    continue;
                }
                let date =
                    month_label(&period.code).ok_or_else(|| format!("国家统计局 CPI 月份无效：{}", period.code))?;
                let index = raw
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("国家统计局 CPI {date} 数值无效：{raw}"))?;
                months.insert(date, index + offset);
                if let Some(name) = cell.i_showname {
                    upstream_name = Some(name.trim().to_owned());
                }
            }
        }
        if months.is_empty() {
            return Err("国家统计局没有返回全国 CPI 数据".into());
        }
        let skip = months.len().saturating_sub(count as usize);
        Ok(Pulled {
            series: Series {
                points: months.into_iter().skip(skip).collect(),
            },
            upstream_name,
        })
    }
}
