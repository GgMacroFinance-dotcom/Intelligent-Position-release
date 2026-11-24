//! 从 SQLite 版的 `ip.db` 导入（`ip store import-sqlite`，`docs/ip/postgres-migration-plan.md` 第 6 节）。
//!
//! - 目标 schema 里不能已经有数据，否则拒绝：避免导两遍。
//! - 全部在一个事务里：按依赖顺序逐表拷，事件保留原来的 seq，更正笔记保留原来的编号；
//!   消息的全文索引按新的分词重新算，不拷 FTS5 的索引。
//! - 提交前逐表核对行数，有一张对不上就整个回滚。
//! - 旧库只读打开，不改不删。

use std::path::Path;

use rusqlite::OpenFlags;
use rusqlite::types::ValueRef;
use tokio_postgres::types::ToSql;

use crate::writer::{clean_text, jsonb_safe};
use crate::{DbConfig, Result, Store, StoreError, schema, segment};

/// 一列在 PostgreSQL 里存成什么。
#[derive(Clone, Copy)]
enum Col {
    Text,
    Int,
    Bool,
    Json,
}

/// 一张表怎么拷。`columns` 是旧库里的列，`defaults` 是旧库没有这一列时用的值（只有整数列会缺）。
struct Table {
    name: &'static str,
    columns: &'static [(&'static str, Col)],
    order_by: &'static str,
    /// 自增列要保留原值时加 `OVERRIDING SYSTEM VALUE`。
    keep_identity: bool,
    /// 消息表另外算一列全文索引。
    with_search: bool,
}

const TABLES: &[Table] = &[
    Table {
        name: "events",
        columns: &[
            ("seq", Col::Int),
            ("at_ms", Col::Int),
            ("kind", Col::Text),
            ("payload", Col::Json),
        ],
        order_by: "seq",
        keep_identity: true,
        with_search: false,
    },
    Table {
        name: "messages",
        columns: &[
            ("id", Col::Text),
            ("thread_id", Col::Text),
            ("at_ms", Col::Int),
            ("text", Col::Text),
            ("json", Col::Json),
            ("seq", Col::Int),
        ],
        order_by: "rowid",
        keep_identity: false,
        with_search: true,
    },
    Table {
        name: "threads",
        columns: &[
            ("id", Col::Text),
            ("last_at_ms", Col::Int),
            ("message_count", Col::Int),
            ("preview", Col::Text),
        ],
        order_by: "id",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "tasks",
        columns: &[
            ("id", Col::Text),
            ("status", Col::Text),
            ("updated_at_ms", Col::Int),
            ("json", Col::Json),
            ("seq", Col::Int),
        ],
        order_by: "id",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "approvals",
        columns: &[
            ("id", Col::Text),
            ("pending", Col::Bool),
            ("created_at_ms", Col::Int),
            ("json", Col::Json),
            ("seq", Col::Int),
        ],
        order_by: "id",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "settings",
        columns: &[("key", Col::Text), ("json", Col::Json), ("seq", Col::Int)],
        order_by: "key",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "channel_bindings",
        columns: &[
            ("channel", Col::Text),
            ("conversation", Col::Text),
            ("thread_id", Col::Text),
            ("agent_id", Col::Text),
            ("requester", Col::Text),
            ("created_at_ms", Col::Int),
            ("seq", Col::Int),
        ],
        order_by: "channel, conversation",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "channel_inbox",
        columns: &[("channel", Col::Text), ("dedupe_key", Col::Text), ("at_ms", Col::Int)],
        order_by: "channel, dedupe_key",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "channel_envelopes",
        columns: &[
            ("channel", Col::Text),
            ("id", Col::Text),
            ("payload", Col::Text),
            ("received_ms", Col::Int),
            ("attempts", Col::Int),
            ("state", Col::Text),
            ("error", Col::Text),
        ],
        order_by: "rowid",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "channel_deliveries",
        columns: &[
            ("channel", Col::Text),
            ("item", Col::Text),
            ("conversation", Col::Text),
            ("external_ref", Col::Text),
            ("state", Col::Text),
            ("error", Col::Text),
            ("at_ms", Col::Int),
        ],
        order_by: "channel, item",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "channel_cursors",
        columns: &[("channel", Col::Text), ("seq", Col::Int)],
        order_by: "channel",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "engine_sessions",
        columns: &[
            ("agent_id", Col::Text),
            ("thread_id", Col::Text),
            ("engine", Col::Text),
            ("session_id", Col::Text),
            ("last_message", Col::Text),
            ("updated_at_ms", Col::Int),
            ("used_prompt_tokens", Col::Int),
            ("used_cached_tokens", Col::Int),
            ("used_cache_write_tokens", Col::Int),
            ("used_completion_tokens", Col::Int),
        ],
        order_by: "agent_id, thread_id",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "memory_thread_summaries",
        columns: &[
            ("role_id", Col::Text),
            ("thread_id", Col::Text),
            ("source_updated_ms", Col::Int),
            ("generated_ms", Col::Int),
            ("status", Col::Text),
            ("slug", Col::Text),
            ("summary", Col::Text),
            ("attempts", Col::Int),
            ("retry_after_ms", Col::Int),
            ("usage_count", Col::Int),
            ("last_usage_ms", Col::Int),
        ],
        order_by: "role_id, thread_id",
        keep_identity: false,
        with_search: false,
    },
    Table {
        name: "memory_notes",
        columns: &[
            ("id", Col::Int),
            ("role_id", Col::Text),
            ("created_ms", Col::Int),
            ("thread_id", Col::Text),
            ("text", Col::Text),
            ("applied_ms", Col::Int),
        ],
        order_by: "id",
        keep_identity: true,
        with_search: false,
    },
];

/// 一个值。旧库里没有的整数列按 0 算（`engine_sessions` 早期没有用量四列）。
enum Value {
    Text(Option<String>),
    Int(Option<i64>),
    Bool(bool),
}

impl Value {
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Value::Text(v) => v,
            Value::Int(v) => v,
            Value::Bool(v) => v,
        }
    }
}

/// 每张表导了几行。旧库里没有这张表时是 `None`。
#[derive(Debug, Default)]
pub struct ImportReport {
    pub tables: Vec<(&'static str, Option<u64>)>,
}

impl Store {
    /// 把 SQLite 版的 `ip.db` 导进 `config` 指的 schema。目标里已经有数据时拒绝。
    pub async fn import_sqlite(config: &DbConfig, path: &Path) -> Result<ImportReport> {
        if !path.is_file() {
            return Err(StoreError::Config(format!("找不到旧库 {}", path.display())));
        }
        // 建 schema、跑迁移；拿到的 Store 用完就丢，导入用自己的连接和事务。
        drop(Store::connect(config).await?);
        let (pg, tls, _) = config.pg_config()?;
        let (mut client, connection) = pg.connect(tls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "导入用的数据库连接断开");
            }
        });
        let transaction = client.transaction().await?;
        // 和写任务取同一把锁：导入期间服务（如果开着）的写入排在后面。
        transaction
            .batch_execute(&format!(
                "SELECT pg_advisory_xact_lock({}, {})",
                schema::LOCK_SPACE,
                schema::lock_key(&config.schema, schema::LOCK_EVENTS)
            ))
            .await?;
        for table in TABLES {
            let n: i64 = transaction
                .query_one(&format!("SELECT COUNT(*) FROM {}", table.name), &[])
                .await?
                .get(0);
            if n > 0 {
                return Err(StoreError::Conflict(format!(
                    "schema {} 的表 {} 里已经有 {n} 行，不导入（避免导两遍）。要重导就先清空这个 schema",
                    config.schema, table.name
                )));
            }
        }

        let mut report = ImportReport::default();
        for table in TABLES {
            let path = path.to_path_buf();
            let rows = tokio::task::spawn_blocking(move || read_table(&path, table))
                .await
                .map_err(|_| StoreError::Closed)??;
            let Some(rows) = rows else {
                report.tables.push((table.name, None));
                continue;
            };
            let statement = transaction.prepare(&insert_sql(table)).await?;
            for row in &rows {
                let mut params: Vec<&(dyn ToSql + Sync)> = row.iter().map(Value::as_sql).collect();
                let search = if table.with_search {
                    match &row[3] {
                        Value::Text(Some(text)) => segment::index_text(text),
                        _ => None,
                    }
                } else {
                    None
                };
                if table.with_search {
                    params.push(&search);
                }
                transaction.execute(&statement, &params).await?;
            }
            let copied: i64 = transaction
                .query_one(&format!("SELECT COUNT(*) FROM {}", table.name), &[])
                .await?
                .get(0);
            if u64::try_from(copied).ok() != Some(rows.len() as u64) {
                return Err(StoreError::Batch(format!(
                    "表 {} 旧库 {} 行，导入后 {copied} 行，已回滚",
                    table.name,
                    rows.len()
                )));
            }
            report.tables.push((table.name, Some(rows.len() as u64)));
        }
        // 保留了原值的自增列，把序列调到最大值之后，新写入接着往下编号。
        for (table, column) in [("events", "seq"), ("memory_notes", "id")] {
            transaction
                .batch_execute(&format!(
                    "SELECT setval(pg_get_serial_sequence('{table}', '{column}'), \
                     GREATEST(COALESCE(MAX({column}), 0), 1), MAX({column}) IS NOT NULL) FROM {table}"
                ))
                .await?;
        }
        transaction.commit().await?;
        client.batch_execute("ANALYZE").await?;
        Ok(report)
    }
}

fn insert_sql(table: &Table) -> String {
    let mut names: Vec<&str> = table.columns.iter().map(|(name, _)| *name).collect();
    let mut values: Vec<String> = table
        .columns
        .iter()
        .enumerate()
        .map(|(i, (_, col))| match col {
            Col::Json => format!("${}::text::jsonb", i + 1),
            _ => format!("${}", i + 1),
        })
        .collect();
    if table.with_search {
        names.push("search");
        values.push(format!("${}::text::tsvector", values.len() + 1));
    }
    let overriding = if table.keep_identity {
        " OVERRIDING SYSTEM VALUE"
    } else {
        ""
    };
    format!(
        "INSERT INTO {} ({}){overriding} VALUES ({})",
        table.name,
        names.join(", "),
        values.join(", ")
    )
}

/// 读旧库的一张表。没有这张表时返回 `None`；没有某个整数列时按 0。
fn read_table(path: &Path, table: &Table) -> Result<Option<Vec<Vec<Value>>>> {
    let db = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| StoreError::Config(format!("打不开旧库 {}：{e}", path.display())))?;
    let sqlite = |e: rusqlite::Error| StoreError::Config(format!("读旧库的表 {} 出错：{e}", table.name));
    let present: Vec<String> = db
        .prepare(&format!("SELECT name FROM pragma_table_info('{}')", table.name))
        .map_err(sqlite)?
        .query_map([], |row| row.get(0))
        .map_err(sqlite)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sqlite)?;
    if present.is_empty() {
        return Ok(None);
    }
    let select: Vec<String> = table
        .columns
        .iter()
        .map(|(name, col)| match (present.iter().any(|p| p == name), col) {
            (true, _) => (*name).to_string(),
            (false, Col::Int) => "0".to_string(),
            (false, _) => "NULL".to_string(),
        })
        .collect();
    let sql = format!(
        "SELECT {} FROM {} ORDER BY {}",
        select.join(", "),
        table.name,
        table.order_by
    );
    let mut statement = db.prepare(&sql).map_err(sqlite)?;
    let mut rows = statement.query([]).map_err(sqlite)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().map_err(sqlite)? {
        let mut values = Vec::with_capacity(table.columns.len());
        for (i, (name, col)) in table.columns.iter().enumerate() {
            let raw = row.get_ref(i).map_err(sqlite)?;
            values.push(
                convert(raw, *col)
                    .map_err(|e| StoreError::Config(format!("表 {} 列 {name} 的值转不了：{e}", table.name)))?,
            );
        }
        out.push(values);
    }
    Ok(Some(out))
}

fn convert(raw: ValueRef<'_>, col: Col) -> std::result::Result<Value, String> {
    let text = |raw: ValueRef<'_>| -> std::result::Result<Option<String>, String> {
        match raw {
            ValueRef::Null => Ok(None),
            ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
                Ok(Some(clean_text(&String::from_utf8_lossy(bytes)).into_owned()))
            }
            ValueRef::Integer(n) => Ok(Some(n.to_string())),
            ValueRef::Real(x) => Ok(Some(x.to_string())),
        }
    };
    Ok(match col {
        Col::Text => Value::Text(text(raw)?),
        Col::Json => Value::Text(text(raw)?.map(jsonb_safe)),
        Col::Int => Value::Int(match raw {
            ValueRef::Null => None,
            ValueRef::Integer(n) => Some(n),
            other => return Err(format!("应是整数，实际是 {:?}", other.data_type())),
        }),
        Col::Bool => Value::Bool(match raw {
            ValueRef::Integer(n) => n != 0,
            other => return Err(format!("应是 0 或 1，实际是 {:?}", other.data_type())),
        }),
    })
}

#[cfg(test)]
mod tests {
    use ip_core::{Author, Message, ThreadId};
    use rusqlite::params;

    use crate::Store;
    use crate::testing::TestDb;

    const OLD_SCHEMA: &str = include_str!("../tests/fixtures/sqlite-schema.sql");

    /// 照 SQLite 版写入的样子造一个旧库：两条消息、一项设置、一条更正、一个早期没有用量列的引擎会话。
    fn old_db(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("ip.db");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(OLD_SCHEMA).unwrap();
        // 早期的 engine_sessions 没有累计用量四列。
        db.execute_batch(
            "DROP TABLE engine_sessions;
             CREATE TABLE engine_sessions (
                 agent_id TEXT NOT NULL, thread_id TEXT NOT NULL, engine TEXT NOT NULL,
                 session_id TEXT NOT NULL, last_message TEXT NOT NULL, updated_at_ms INTEGER NOT NULL,
                 PRIMARY KEY (agent_id, thread_id)
             ) WITHOUT ROWID;
             INSERT INTO engine_sessions VALUES ('ops-1', 't1', 'codex', 'sess-old', 'm9', 7);",
        )
        .unwrap();
        let mut seq = 0;
        for (i, body) in ["请帮我整理一下本周的销售报表", "工具输出\0带一个 NUL"]
            .iter()
            .enumerate()
        {
            let mut m = Message::text(ThreadId::from("t1"), Author::human("张三"), *body);
            m.at_ms = i as i64 + 1;
            let json = serde_json::to_string(&m).unwrap();
            db.execute(
                "INSERT INTO events (at_ms, kind, payload) VALUES (?1, 'message.appended', ?2)",
                params![m.at_ms, json],
            )
            .unwrap();
            seq = db.last_insert_rowid();
            db.execute(
                "INSERT INTO messages (id, thread_id, at_ms, text, json, seq) VALUES (?1, 't1', ?2, ?3, ?4, ?5)",
                params![m.id.as_str(), m.at_ms, body, json, seq],
            )
            .unwrap();
        }
        db.execute_batch(
            "INSERT INTO threads VALUES ('t1', 2, 2, '工具输出');
             INSERT INTO events (at_ms, kind, payload) VALUES (3, 'setting.put', '{\"key\":\"rules\",\"value\":{\"a\":1}}');",
        )
        .unwrap();
        db.execute(
            "INSERT INTO settings VALUES ('rules', '{\"a\":1}', ?1)",
            params![seq + 1],
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO memory_notes (id, role_id, created_ms, thread_id, text) VALUES (7, 'macro', 5, 't1', '以后先看 CPI');
             INSERT INTO approvals VALUES ('a1', 1, 4, '{}', 1);
             INSERT INTO channel_cursors VALUES ('slack', 3);",
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn imports_an_old_database_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = old_db(dir.path());
        let db = TestDb::new();
        let report = Store::import_sqlite(db.config(), &path).await.unwrap();
        let count = |name: &str| report.tables.iter().find(|(t, _)| *t == name).and_then(|(_, n)| *n);
        assert_eq!(count("events"), Some(3));
        assert_eq!(count("messages"), Some(2));
        assert_eq!(count("memory_notes"), Some(1));
        assert_eq!(count("engine_sessions"), Some(1));

        let store = db.open().await;
        // 全文索引按新分词重算了；NUL 换成了替换字符。
        assert_eq!(store.search("销售报表", None, 10).await.unwrap().len(), 1);
        let messages = store.thread_messages(&"t1".into(), None, 10).await.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(store.recent_threads(10).await.unwrap()[0].message_count, 2);
        assert_eq!(
            store.setting("rules").await.unwrap(),
            Some(serde_json::json!({ "a": 1 }))
        );
        assert_eq!(store.cursor("slack").await.unwrap(), Some(3));
        let session = store
            .engine_session(&"ops-1".into(), &"t1".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.session_id, "sess-old");
        assert_eq!(session.used, crate::SessionUsage::default());

        // 新写入接着原来的编号往下走。
        let seq = store
            .append_message(Message::text(
                ThreadId::from("t1"),
                Author::human("张三"),
                "导入后的第一条",
            ))
            .await
            .unwrap();
        assert!(seq > 3, "{seq}");
        let note = store
            .add_memory_note(&"macro".into(), None, "新的更正", 9)
            .await
            .unwrap();
        assert!(note > 7, "{note}");

        // 再导一次：目标已经有数据，拒绝，原有数据不动。
        let again = Store::import_sqlite(db.config(), &path).await;
        assert!(matches!(again, Err(crate::StoreError::Conflict(_))), "{again:?}");
        assert_eq!(store.thread_messages(&"t1".into(), None, 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn missing_file_is_an_error() {
        let db = TestDb::new();
        let result = Store::import_sqlite(db.config(), std::path::Path::new("/nonexistent/ip.db")).await;
        assert!(matches!(result, Err(crate::StoreError::Config(_))));
    }
}
