//! 数据源：给数字员工查数据用的两个工具。
//!
//! - `data_catalog`：列出能用的序列（名称、单位、频率、来源、数据区间）。
//! - `data_series`：取一个序列，可截取时间段、算同比 / 环比 / 差值，结果带来源和数据截至日期。
//!
//! 数据源在 `ip.yaml` 的 `data_sources` 里配置，可以限定只给某些岗位用。现在支持的类型是 `csv_dir`
//! （一个目录放 `catalog.yaml` 和若干 CSV，见 [`csv_dir`]），数据管道定时把官方数据写成 CSV 即可。
//! 需要实时接口（行情、数据库）时，可以另写一种数据源，或者用 MCP 服务器接入。
//!
//! 工具只读：员工不能通过它们改数据。

pub mod csv_dir;
pub mod fetch;
pub mod series;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ip_core::RoleId;
use ip_models::ToolSpec;
use ip_tools::ToolOutput;
use serde::Deserialize;
use serde_json::json;

use crate::csv_dir::{CsvDir, SeriesInfo};
use crate::series::{Transform, fmt_value};

pub const CATALOG_TOOL: &str = "data_catalog";
pub const SERIES_TOOL: &str = "data_series";
/// 一次最多返回的数据行数，多了只留最近的。
const MAX_ROWS: usize = 240;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    #[default]
    CsvDir,
}

/// `ip.yaml` 里 `data_sources` 的一项。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    #[serde(default)]
    pub kind: SourceKind,
    /// 数据目录；相对路径以配置文件所在目录为准。
    pub path: PathBuf,
    /// 只给这些岗位用；空表示所有岗位。
    #[serde(default)]
    pub roles: Vec<RoleId>,
}

#[derive(Debug, thiserror::Error)]
pub enum DataError {
    #[error("数据源名 {0:?} 只能用字母、数字、_ 和 -")]
    BadName(String),
    #[error("数据源 {name} 不可用：{error}")]
    Open { name: String, error: String },
}

struct Source {
    dir: CsvDir,
    roles: Vec<RoleId>,
}

/// 所有数据源。只读，放进 `Arc` 在员工之间共享。
#[derive(Default)]
pub struct DataHub {
    sources: BTreeMap<String, Source>,
}

pub type SharedData = Arc<DataHub>;

/// 一个数据源的状况（[`DataHub::health`]）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceHealth {
    pub name: String,
    pub series: usize,
    pub readable: usize,
    pub error: Option<String>,
}

impl DataHub {
    /// 打开并检查所有数据源；`base` 用来换算相对路径。
    pub fn open(configs: &BTreeMap<String, SourceConfig>, base: &Path) -> Result<Self, DataError> {
        let mut hub = Self::default();
        for (name, config) in configs {
            let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(DataError::BadName(name.clone()));
            }
            let path = if config.path.is_relative() {
                base.join(&config.path)
            } else {
                config.path.clone()
            };
            let dir = match config.kind {
                SourceKind::CsvDir => CsvDir::open(path).map_err(|error| DataError::Open {
                    name: name.clone(),
                    error,
                })?,
            };
            hub.sources.insert(
                name.clone(),
                Source {
                    dir,
                    roles: config.roles.clone(),
                },
            );
        }
        Ok(hub)
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// 每个数据源目录里登记了几个序列、其中几个读得出至少一个数据点；目录本身读不了时给出原因。`ip check` 用。
    pub fn health(&self) -> Vec<SourceHealth> {
        self.sources
            .iter()
            .map(|(name, s)| match s.dir.catalog() {
                Ok(catalog) => SourceHealth {
                    name: name.clone(),
                    series: catalog.series.len(),
                    // 还没拉取的序列读出来是空的，不算有数据。
                    readable: catalog
                        .series
                        .iter()
                        .filter(|i| s.dir.read(i).is_ok_and(|series| !series.points.is_empty()))
                        .count(),
                    error: None,
                },
                Err(e) => SourceHealth {
                    name: name.clone(),
                    series: 0,
                    readable: 0,
                    error: Some(e),
                },
            })
            .collect()
    }

    fn visible<'a>(&'a self, role: &'a RoleId) -> impl Iterator<Item = (&'a String, &'a Source)> + 'a {
        self.sources
            .iter()
            .filter(move |(_, s)| s.roles.is_empty() || s.roles.contains(role))
    }

    /// 某个岗位能用的工具；没有可用的数据源时为空。
    pub fn specs_for(&self, role: &RoleId) -> Vec<ToolSpec> {
        if self.visible(role).next().is_none() {
            return Vec::new();
        }
        vec![
            ToolSpec {
                name: CATALOG_TOOL.into(),
                description:
                    "列出能查的数据序列：名称、单位、频率、来源和数据区间。先用它找到序列 id，再用 data_series 取数。"
                        .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "按名称或 id 过滤，可省略" }
                    }
                }),
            },
            ToolSpec {
                name: SERIES_TOOL.into(),
                description: "取一个数据序列，可截取时间段、计算同比/环比/差值。结果带来源和数据截至日期，引用时照写。"
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "series": { "type": "string", "description": "序列 id，写成 <数据源>/<序列>，见 data_catalog" },
                        "start": { "type": "string", "description": "起始日期（含），如 2025-01、2025-Q1；可省略" },
                        "end": { "type": "string", "description": "结束日期（含）；可省略" },
                        "transform": {
                            "type": "string",
                            "enum": ["level", "yoy", "mom", "diff"],
                            "description": "level 原值（默认）；yoy 同比 %；mom 环比 %；diff 较上期变化（原值是百分比时即百分点）"
                        }
                    },
                    "required": ["series"]
                }),
            },
        ]
    }

    /// 执行数据工具；名字不是数据工具时返回 `None`。
    pub async fn call(self: &Arc<Self>, role: &RoleId, name: &str, arguments: &str) -> Option<ToolOutput> {
        if name != CATALOG_TOOL && name != SERIES_TOOL {
            return None;
        }
        let hub = Arc::clone(self);
        let (role, name, arguments) = (role.clone(), name.to_string(), arguments.to_string());
        let out = tokio::task::spawn_blocking(move || {
            let result = if name == CATALOG_TOOL {
                hub.catalog(&role, &arguments)
            } else {
                hub.series(&role, &arguments)
            };
            match result {
                Ok(text) => ToolOutput::ok(text),
                Err(e) => ToolOutput::err(e),
            }
        })
        .await
        .unwrap_or_else(|e| ToolOutput::err(format!("数据工具出错：{e}")));
        Some(out)
    }

    fn catalog(&self, role: &RoleId, arguments: &str) -> Result<String, String> {
        #[derive(Deserialize, Default)]
        struct Args {
            #[serde(default)]
            query: Option<String>,
        }
        let args: Args = parse_args(arguments)?;
        let query = args.query.map(|q| q.trim().to_lowercase()).filter(|q| !q.is_empty());
        let mut lines = Vec::new();
        for (name, source) in self.visible(role) {
            let catalog = source.dir.catalog()?;
            for info in &catalog.series {
                let id = format!("{name}/{}", info.id);
                if let Some(q) = &query
                    && !id.to_lowercase().contains(q)
                    && !info.name.to_lowercase().contains(q)
                {
                    continue;
                }
                let range = match source.dir.read(info) {
                    Ok(s) => match (s.points.first(), s.points.last()) {
                        (Some(a), Some(b)) => format!("数据 {} 至 {}", a.0, b.0),
                        _ => "暂无数据（还没拉取）".into(),
                    },
                    Err(e) => format!("读取失败：{e}"),
                };
                let source_name = info.source.as_deref().unwrap_or(&catalog.source);
                let note = info.note.as_deref().map(|n| format!("；{n}")).unwrap_or_default();
                lines.push(format!(
                    "- {id}：{}（{}）。来源：{source_name}；{range}{note}",
                    info.name,
                    describe_unit(info)
                ));
            }
        }
        if lines.is_empty() {
            return Ok(match query {
                Some(q) => format!("没有名称或 id 含“{q}”的序列"),
                None => "没有可用的数据序列".into(),
            });
        }
        Ok(format!("共 {} 个序列：\n{}", lines.len(), lines.join("\n")))
    }

    fn series(&self, role: &RoleId, arguments: &str) -> Result<String, String> {
        #[derive(Deserialize)]
        struct Args {
            series: String,
            #[serde(default)]
            start: Option<String>,
            #[serde(default)]
            end: Option<String>,
            #[serde(default)]
            transform: Transform,
        }
        let args: Args = parse_args(arguments)?;
        let (source_name, id) = args
            .series
            .split_once('/')
            .ok_or_else(|| format!("序列 id {} 应写成 <数据源>/<序列>，先用 data_catalog 查", args.series))?;
        let source = self
            .visible(role)
            .find(|(n, _)| n.as_str() == source_name)
            .map(|(_, s)| s)
            .ok_or_else(|| format!("没有数据源 {source_name}，或你的岗位不能用它"))?;
        let catalog = source.dir.catalog()?;
        let info = catalog
            .series
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("数据源 {source_name} 里没有序列 {id}，先用 data_catalog 查"))?;
        for date in [&args.start, &args.end].into_iter().flatten() {
            if !info.frequency.valid(date) {
                return Err(format!("日期 {date} 不符合{}格式", info.frequency.label()));
            }
        }
        let raw = source.dir.read(info)?;
        let latest = raw.points.last().map(|p| p.0.clone()).unwrap_or_default();
        let data = raw
            .transform(args.transform, info.frequency)?
            .between(args.start.as_deref(), args.end.as_deref());
        let source_text = info.source.as_deref().unwrap_or(&catalog.source);
        let unit = match args.transform {
            Transform::Level | Transform::Diff => describe_unit(info),
            Transform::Yoy | Transform::Mom => format!("%，{}", info.frequency.label()),
        };
        let mut out = format!(
            "{}/{}：{}，{}（{unit}）\n来源：{source_text}；数据截至 {latest}\n",
            source_name,
            info.id,
            info.name,
            args.transform.label()
        );
        if let Some(note) = &info.note {
            out.push_str(&format!("说明：{note}\n"));
        }
        if raw.points.is_empty() {
            out.push_str("这个序列还没有数据：需要先运行 ip data pull 拉取，或等数据管道写入");
            return Ok(out);
        }
        if data.points.is_empty() {
            out.push_str("这个时间段没有数据");
            return Ok(out);
        }
        let skip = data.points.len().saturating_sub(MAX_ROWS);
        if skip > 0 {
            out.push_str(&format!("（共 {} 行，只列出最近 {MAX_ROWS} 行）\n", data.points.len()));
        }
        out.push_str("date,value\n");
        for (d, v) in &data.points[skip..] {
            out.push_str(&format!("{d},{}\n", fmt_value(*v)));
        }
        let n = data.points.len();
        let (last_d, last_v) = &data.points[n - 1];
        out.push_str(&format!("最新：{last_d} = {}", fmt_value(*last_v)));
        if n >= 2 {
            let (prev_d, prev_v) = &data.points[n - 2];
            out.push_str(&format!(
                "；前一期 {prev_d} = {}；变化 {}",
                fmt_value(*prev_v),
                fmt_value(last_v - prev_v)
            ));
        }
        Ok(out)
    }
}

/// `ip data pull` 的一行结果。
#[derive(Debug)]
pub struct PullReport {
    /// `<数据源>/<序列>`。
    pub series: String,
    pub outcome: Result<PullOk, String>,
}

#[derive(Debug)]
pub struct PullOk {
    pub fetched: usize,
    pub total: usize,
    pub latest: String,
    pub upstream_name: Option<String>,
}

/// 拉取所有写了 `fetch` 的序列，写进 CSV。`only` 是数据源名或 `<数据源>/<序列>`，限定只拉这些。
/// 单个序列失败不影响其他序列，失败原因写在对应的结果里。
pub async fn pull(
    configs: &BTreeMap<String, SourceConfig>,
    base: &Path,
    fetcher: &fetch::Fetcher,
    only: Option<&str>,
) -> Result<Vec<PullReport>, DataError> {
    let hub = DataHub::open(configs, base)?;
    let mut reports = Vec::new();
    for (name, source) in &hub.sources {
        let catalog = source.dir.catalog().map_err(|error| DataError::Open {
            name: name.clone(),
            error,
        })?;
        for info in &catalog.series {
            let id = format!("{name}/{}", info.id);
            if only.is_some_and(|o| o != name && o != id) {
                continue;
            }
            let Some(spec) = &info.fetch else { continue };
            let outcome = async {
                let pulled = fetcher.pull(spec, info.frequency).await?;
                let total = fetch::merge_into(&source.dir, info, &pulled.series)?;
                Ok(PullOk {
                    fetched: pulled.series.points.len(),
                    total,
                    latest: pulled.series.points.last().map(|p| p.0.clone()).unwrap_or_default(),
                    upstream_name: pulled.upstream_name,
                })
            }
            .await;
            reports.push(PullReport { series: id, outcome });
        }
    }
    Ok(reports)
}

fn describe_unit(info: &SeriesInfo) -> String {
    if info.unit.is_empty() {
        info.frequency.label().to_string()
    } else {
        format!("{}，{}", info.unit, info.frequency.label())
    }
}

fn parse_args<T: serde::de::DeserializeOwned>(arguments: &str) -> Result<T, String> {
    let text = if arguments.trim().is_empty() { "{}" } else { arguments };
    serde_json::from_str(text).map_err(|e| format!("参数不对：{e}"))
}

#[cfg(test)]
mod tests;
