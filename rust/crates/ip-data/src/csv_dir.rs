//! 一个目录就是一个数据源：`catalog.yaml` 列出序列，每个序列一个 `<id>.csv`（两列：日期、数值）。
//!
//! ```yaml
//! source: 国家统计局            # 默认来源，写进每次取数的结果
//! series:
//!   - id: cn_cpi_yoy
//!     name: 中国 CPI 同比
//!     unit: "%"
//!     frequency: monthly       # daily / monthly / quarterly
//!     note: 上年同月 = 100     # 可选
//! ```
//!
//! 每次调用都重新读文件，数据管道更新了 CSV 不用重启服务。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::series::{Frequency, Series};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    /// 默认来源说明。
    pub source: String,
    pub series: Vec<SeriesInfo>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub unit: String,
    pub frequency: Frequency,
    /// 覆盖目录的默认来源。
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    /// 从哪里拉取，见 [`crate::fetch`]；不写表示 CSV 由别的程序维护。
    #[serde(default)]
    pub fetch: Option<crate::fetch::Fetch>,
}

#[derive(Clone, Debug)]
pub struct CsvDir {
    dir: PathBuf,
}

impl CsvDir {
    /// 检查目录和目录文件可读、序列 id 合法且不重复。
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let source = Self { dir: dir.into() };
        source.catalog()?;
        Ok(source)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn catalog(&self) -> Result<Catalog, String> {
        let path = self.dir.join("catalog.yaml");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        let catalog: Catalog =
            serde_saphyr::from_str(&text).map_err(|e| format!("{} 格式不对：{e}", path.display()))?;
        let mut seen = std::collections::HashSet::new();
        for s in &catalog.series {
            let ok = !s.id.is_empty() && s.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(format!("序列 id {:?} 只能用字母、数字、_ 和 -", s.id));
            }
            if !seen.insert(s.id.as_str()) {
                return Err(format!("序列 id {} 重复", s.id));
            }
        }
        Ok(catalog)
    }

    /// 读一个序列的全部数据，按日期升序。CSV 还不存在（还没拉取过）时是空序列。
    pub fn read(&self, info: &SeriesInfo) -> Result<Series, String> {
        let path = self.dir.join(format!("{}.csv", info.id));
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Series::default()),
            Err(e) => return Err(format!("读取 {} 失败：{e}", path.display())),
        };
        parse_csv(&text, info.frequency).map_err(|e| format!("{}：{e}", path.display()))
    }
}

/// 两列 `日期,数值`。第一行的数值不是数字时当表头跳过；空行和 `#` 开头的行跳过；数值为空表示缺失。
pub fn parse_csv(text: &str, freq: Frequency) -> Result<Series, String> {
    let mut points: Vec<(String, f64)> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (date, value) = line
            .split_once(',')
            .map(|(d, v)| (d.trim(), v.trim()))
            .ok_or_else(|| format!("第 {} 行应为“日期,数值”", i + 1))?;
        if !freq.valid(date) {
            if i == 0 && value.parse::<f64>().is_err() {
                continue; // 表头
            }
            return Err(format!("第 {} 行的日期 {date} 不符合{}格式", i + 1, freq.label()));
        }
        if value.is_empty() {
            continue;
        }
        let v: f64 = value
            .parse()
            .map_err(|_| format!("第 {} 行的数值 {value} 不是数字", i + 1))?;
        if !v.is_finite() {
            return Err(format!("第 {} 行的数值 {value} 无效", i + 1));
        }
        points.push((date.to_string(), v));
    }
    points.sort_by(|a, b| a.0.cmp(&b.0));
    if let Some(w) = points.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(format!("日期 {} 重复", w[0].0));
    }
    Ok(Series { points })
}
