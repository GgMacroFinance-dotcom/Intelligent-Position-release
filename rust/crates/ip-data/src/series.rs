//! 时间序列和常用变换（同比、环比、差值）。
//!
//! 日期是字符串：月度 `YYYY-MM`，季度 `YYYY-Qn`，日度 `YYYY-MM-DD`。同一序列格式一致，
//! 所以按字符串比较就是按时间先后。同比、环比按日期找对应的那一期，不按行数偏移，中间缺期也不会算错。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    Daily,
    Monthly,
    Quarterly,
}

impl Frequency {
    pub fn label(self) -> &'static str {
        match self {
            Self::Daily => "日度",
            Self::Monthly => "月度",
            Self::Quarterly => "季度",
        }
    }

    /// 日期格式是否符合频率。
    pub fn valid(self, date: &str) -> bool {
        let b = date.as_bytes();
        let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
        match self {
            Self::Monthly => b.len() == 7 && digits(0..4) && b[4] == b'-' && digits(5..7) && month(date).is_some(),
            Self::Quarterly => b.len() == 7 && digits(0..4) && &date[4..6] == "-Q" && matches!(b[6], b'1'..=b'4'),
            Self::Daily => {
                b.len() == 10 && digits(0..4) && b[4] == b'-' && digits(5..7) && b[7] == b'-' && digits(8..10)
            }
        }
    }

    /// 上一期的日期；日度数据没有固定的上一期，返回 `None`。
    fn previous(self, date: &str) -> Option<String> {
        let year: i32 = date.get(0..4)?.parse().ok()?;
        match self {
            Self::Monthly => {
                let m = month(date)?;
                Some(if m == 1 {
                    format!("{}-12", year - 1)
                } else {
                    format!("{year}-{:02}", m - 1)
                })
            }
            Self::Quarterly => {
                let q: u32 = date.get(6..7)?.parse().ok()?;
                Some(if q == 1 {
                    format!("{}-Q4", year - 1)
                } else {
                    format!("{year}-Q{}", q - 1)
                })
            }
            Self::Daily => None,
        }
    }

    /// 去年同期的日期。
    fn year_ago(self, date: &str) -> Option<String> {
        let year: i32 = date.get(0..4)?.parse().ok()?;
        Some(format!("{}{}", year - 1, date.get(4..)?))
    }
}

fn month(date: &str) -> Option<u32> {
    date.get(5..7)?.parse().ok().filter(|m| (1..=12).contains(m))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transform {
    /// 原值。
    #[default]
    Level,
    /// 同比增速（%）。
    Yoy,
    /// 环比增速（%）。
    Mom,
    /// 与上一期的差值（原单位；原值是百分比时就是百分点）。
    Diff,
}

impl Transform {
    pub fn label(self) -> &'static str {
        match self {
            Self::Level => "原值",
            Self::Yoy => "同比（%）",
            Self::Mom => "环比（%）",
            Self::Diff => "较上期变化",
        }
    }
}

/// 按日期升序排列的观测值。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Series {
    pub points: Vec<(String, f64)>,
}

impl Series {
    fn get(&self, date: &str) -> Option<f64> {
        self.points
            .binary_search_by(|(d, _)| d.as_str().cmp(date))
            .ok()
            .map(|i| self.points[i].1)
    }

    /// 应用变换。找不到对应期（序列开头、缺期）的点直接略去；基期为 0 时也略去。
    pub fn transform(&self, t: Transform, freq: Frequency) -> Result<Series, String> {
        let base: fn(Frequency, &str) -> Option<String> = match t {
            Transform::Level => return Ok(self.clone()),
            Transform::Yoy => Frequency::year_ago,
            Transform::Mom | Transform::Diff => Frequency::previous,
        };
        if freq == Frequency::Daily && t != Transform::Yoy {
            return Err("日度数据没有固定的上一期，只支持原值和同比".into());
        }
        let points = self
            .points
            .iter()
            .filter_map(|(date, v)| {
                let b = self.get(&base(freq, date)?)?;
                let out = match t {
                    Transform::Diff => v - b,
                    _ if b == 0.0 => return None,
                    _ => (v / b - 1.0) * 100.0,
                };
                Some((date.clone(), out))
            })
            .collect();
        Ok(Series { points })
    }

    /// 只保留 `[start, end]` 之间的点（含两端）。
    pub fn between(mut self, start: Option<&str>, end: Option<&str>) -> Series {
        self.points
            .retain(|(d, _)| start.is_none_or(|s| d.as_str() >= s) && end.is_none_or(|e| d.as_str() <= e));
        self
    }
}

/// 输出用的数值：最多四位小数，去掉末尾的 0。
pub fn fmt_value(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.into() }
}
