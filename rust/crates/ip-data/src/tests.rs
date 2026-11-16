use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use ip_core::RoleId;

use crate::csv_dir::parse_csv;
use crate::series::{Frequency, Transform, fmt_value};
use crate::{DataHub, SourceConfig, SourceKind};

fn sample_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/macro-data")
}

#[test]
fn transforms_match_by_date() {
    // 2025-03 缺期：2025-04 没有环比，但同比照样能算。
    let s = parse_csv(
        "date,value\n2024-02,100\n2024-04,110\n2025-01,90\n2025-02,105\n2025-04,121\n",
        Frequency::Monthly,
    )
    .unwrap();
    let yoy = s.transform(Transform::Yoy, Frequency::Monthly).unwrap();
    assert_eq!(
        yoy.points
            .iter()
            .map(|(d, v)| (d.as_str(), fmt_value(*v)))
            .collect::<Vec<_>>(),
        [("2025-02", "5".to_string()), ("2025-04", "10".to_string())]
    );
    let mom = s.transform(Transform::Mom, Frequency::Monthly).unwrap();
    assert_eq!(mom.points.len(), 1);
    assert_eq!(mom.points[0].0, "2025-02");
    let diff = s.transform(Transform::Diff, Frequency::Monthly).unwrap();
    assert_eq!(diff.points, [("2025-02".to_string(), 15.0)]);

    // 跨年的上一期。
    let q = parse_csv("2024-Q4,5.0\n2025-Q1,5.4\n", Frequency::Quarterly).unwrap();
    let d = q.transform(Transform::Diff, Frequency::Quarterly).unwrap();
    assert_eq!(fmt_value(d.points[0].1), "0.4");
    let m = parse_csv("2024-12,1\n2025-01,3\n", Frequency::Monthly).unwrap();
    assert_eq!(
        m.transform(Transform::Mom, Frequency::Monthly).unwrap().points[0].1,
        200.0
    );

    assert!(q.clone().between(Some("2025-Q1"), None).points.len() == 1);
    let daily = parse_csv("2025-01-02,1\n", Frequency::Daily).unwrap();
    assert!(daily.transform(Transform::Mom, Frequency::Daily).is_err());
}

#[test]
fn csv_errors_name_the_line() {
    assert!(
        parse_csv("2025-13,1\n", Frequency::Monthly)
            .unwrap_err()
            .contains("第 1 行")
    );
    assert!(
        parse_csv("date,value\n2025-01,abc\n", Frequency::Monthly)
            .unwrap_err()
            .contains("第 2 行")
    );
    assert!(
        parse_csv("2025-01,1\n2025-01,2\n", Frequency::Monthly)
            .unwrap_err()
            .contains("重复")
    );
    // 空值是缺失，# 开头是注释。
    let s = parse_csv("# 注释\n2025-01,\n2025-02,1.5\n", Frequency::Monthly).unwrap();
    assert_eq!(s.points.len(), 1);
}

fn hub(roles: Vec<RoleId>) -> Arc<DataHub> {
    let mut configs = BTreeMap::new();
    configs.insert(
        "demo".to_string(),
        SourceConfig {
            kind: SourceKind::CsvDir,
            path: sample_dir(),
            roles,
        },
    );
    Arc::new(DataHub::open(&configs, std::path::Path::new("/")).unwrap())
}

#[tokio::test]
async fn sample_source_answers_catalog_and_series() {
    let hub = hub(vec!["macro".into()]);
    let macro_role: RoleId = "macro".into();

    assert_eq!(hub.specs_for(&macro_role).len(), 2);
    assert!(hub.specs_for(&"ops".into()).is_empty(), "其他岗位看不到数据工具");
    assert!(hub.call(&macro_role, "shell", "{}").await.is_none());

    let all = hub.call(&macro_role, "data_catalog", "").await.unwrap();
    assert!(all.ok, "{}", all.output);
    assert!(all.output.contains("共 6 个序列"), "{}", all.output);
    assert!(
        all.output
            .contains("demo/cn_cpi_yoy：中国 CPI 同比（%，月度）。来源：示例数据（虚构")
    );
    let pmi = hub
        .call(&macro_role, "data_catalog", r#"{"query":"PMI"}"#)
        .await
        .unwrap();
    assert!(
        pmi.output.contains("共 1 个序列") && pmi.output.contains("50 为荣枯线"),
        "{}",
        pmi.output
    );

    let m2 = hub
        .call(
            &macro_role,
            "data_series",
            r#"{"series":"demo/cn_m2","start":"2026-07","transform":"yoy"}"#,
        )
        .await
        .unwrap();
    assert!(m2.ok, "{}", m2.output);
    assert!(m2.output.contains("同比（%）"), "{}", m2.output);
    assert!(m2.output.contains("数据截至 2026-10"), "{}", m2.output);
    assert_eq!(m2.output.matches("\n2026-").count(), 4, "{}", m2.output);
    assert!(m2.output.contains("最新：2026-10 = "), "{}", m2.output);

    let errors = [
        (r#"{"series":"cn_m2"}"#, "<数据源>/<序列>"),
        (r#"{"series":"demo/nope"}"#, "没有序列 nope"),
        (r#"{"series":"other/cn_m2"}"#, "没有数据源 other"),
        (r#"{"series":"demo/cn_m2","start":"2026"}"#, "不符合月度格式"),
        (r#"{"series":"demo/cn_m2","transform":"qoq"}"#, "参数不对"),
    ];
    for (args, want) in errors {
        let out = hub.call(&macro_role, "data_series", args).await.unwrap();
        assert!(!out.ok && out.output.contains(want), "{args}: {}", out.output);
    }
    let out = hub
        .call(&"ops".into(), "data_series", r#"{"series":"demo/cn_m2"}"#)
        .await
        .unwrap();
    assert!(!out.ok, "岗位受限的数据源不能取数");
}

#[test]
fn bad_sources_are_rejected_at_startup() {
    let mut configs = BTreeMap::new();
    configs.insert(
        "demo".to_string(),
        SourceConfig {
            kind: SourceKind::CsvDir,
            path: "does-not-exist".into(),
            roles: vec![],
        },
    );
    let dir = tempfile::tempdir().unwrap();
    assert!(DataHub::open(&configs, dir.path()).is_err());

    std::fs::write(
        dir.path().join("catalog.yaml"),
        "source: x\nseries:\n  - { id: ../etc, name: x, frequency: monthly }\n",
    )
    .unwrap();
    configs.get_mut("demo").unwrap().path = dir.path().to_path_buf();
    let err = DataHub::open(&configs, dir.path()).err().unwrap().to_string();
    assert!(err.contains("只能用字母"), "{err}");
}

mod fetching {
    use std::collections::BTreeMap;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::fetch::{Fetch, Fetcher, parse_fred};
    use crate::series::Frequency;
    use crate::{SourceConfig, SourceKind, pull};

    #[test]
    fn parses_fred_csv() {
        let csv = "observation_date,DGS10\n2024-07-01,4.25\n2024-10-01,.\n2025-01-01,\n2025-04-01,4.4\n";
        let q = parse_fred(csv, Frequency::Quarterly).unwrap();
        assert_eq!(q.points, [("2024-Q3".into(), 4.25), ("2025-Q2".into(), 4.4)]);
        let m = parse_fred(csv, Frequency::Monthly).unwrap();
        assert_eq!(m.points[0].0, "2024-07");
        assert!(parse_fred("observation_date,X\n", Frequency::Monthly).is_err());
        assert!(parse_fred("h\n2024/01/01,1\n", Frequency::Monthly).is_err());
        let spec: Fetch = serde_saphyr::from_str("{ fred: { id: DGS10, aggregate: eop } }").unwrap();
        assert!(matches!(spec, Fetch::Fred(ref f) if f.id == "DGS10"));
        // 国家统计局改由 macro-mcp 的工具直接取，旧写法要报错，不能悄悄不拉。
        assert!(serde_saphyr::from_str::<Fetch>("{ nbs: { code: A01010101 } }").is_err());
    }

    /// 本地假服务：按路径前缀返回固定内容，记录请求行。
    async fn fake_server(
        routes: Vec<(&'static str, String)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = req.lines().next().unwrap_or_default().to_string();
                log.lock().unwrap().push(line.clone());
                let body = routes.iter().find(|(p, _)| line.contains(p)).map(|(_, b)| b.clone());
                let (status, body) = match body {
                    Some(b) => ("200 OK", b),
                    None => ("404 Not Found", "<h1>not found</h1>".to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });
        (base, seen)
    }

    #[tokio::test]
    async fn pull_merges_into_csv() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("catalog.yaml"),
            r#"source: 测试
series:
  - { id: us_cpi, name: 美国 CPI, frequency: monthly, fetch: { fred: { id: CPIAUCSL } } }
  - { id: bad, name: 不存在, frequency: monthly, fetch: { fred: { id: NOPE } } }
  - { id: manual, name: 手工维护, frequency: monthly }
"#,
        )
        .unwrap();
        // 已有的历史数据保留，同一日期以新数据为准。
        std::fs::write(dir.path().join("us_cpi.csv"), "date,value\n2000-01,1\n2024-09,999\n").unwrap();
        let fred = "observation_date,CPIAUCSL\n2024-09-01,314.1\n2024-10-01,314.7\n".to_string();
        let (base, seen) = fake_server(vec![("id=CPIAUCSL", fred)]).await;
        let fetcher = Fetcher::new().unwrap().with_fred_base(&base);
        let mut configs = BTreeMap::new();
        configs.insert(
            "pub".to_string(),
            SourceConfig {
                kind: SourceKind::CsvDir,
                path: dir.path().to_path_buf(),
                roles: vec![],
            },
        );

        let reports = pull(&configs, dir.path(), &fetcher, None).await.unwrap();
        let ids: Vec<&str> = reports.iter().map(|r| r.series.as_str()).collect();
        assert_eq!(ids, ["pub/us_cpi", "pub/bad"], "没写 fetch 的序列不拉");
        let ok = reports[0].outcome.as_ref().unwrap();
        assert_eq!((ok.fetched, ok.total, ok.latest.as_str()), (2, 3, "2024-10"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("us_cpi.csv")).unwrap(),
            "date,value\n2000-01,1\n2024-09,314.1\n2024-10,314.7\n"
        );
        let err = reports[1].outcome.as_ref().unwrap_err();
        assert!(
            err.contains("404") && err.contains("not found") && !err.contains("<h1>"),
            "{err}"
        );

        let requests = seen.lock().unwrap().clone();
        assert!(
            requests.iter().any(|r| r.contains("fq=Monthly&fam=avg")),
            "{requests:?}"
        );

        // 只拉一个序列。
        let one = pull(&configs, dir.path(), &fetcher, Some("pub/us_cpi")).await.unwrap();
        assert_eq!(one.len(), 1);
    }

    #[tokio::test]
    async fn series_without_data_says_how_to_get_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("catalog.yaml"),
            "source: 测试\nseries:\n  - { id: x, name: X, frequency: monthly, fetch: { fred: { id: X } } }\n",
        )
        .unwrap();
        let mut configs = BTreeMap::new();
        configs.insert(
            "pub".to_string(),
            SourceConfig {
                kind: SourceKind::CsvDir,
                path: dir.path().to_path_buf(),
                roles: vec![],
            },
        );
        let hub = std::sync::Arc::new(crate::DataHub::open(&configs, dir.path()).unwrap());
        let role = "any".into();
        let cat = hub.call(&role, "data_catalog", "{}").await.unwrap();
        assert!(cat.output.contains("暂无数据（还没拉取）"), "{}", cat.output);
        let s = hub.call(&role, "data_series", r#"{"series":"pub/x"}"#).await.unwrap();
        assert!(s.ok && s.output.contains("ip data pull"), "{}", s.output);
    }
}
