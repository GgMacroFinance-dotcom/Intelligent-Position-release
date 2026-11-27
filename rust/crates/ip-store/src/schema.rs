//! 连接设置和建表。表结构写在 `migrations/` 下按编号排的 SQL 文件里，编译进二进制。

use tokio_postgres::Client;

use crate::Result;

/// advisory lock 的第一个键，“IP”两个字母。和 TCMS（sqlx 用单键锁）不在同一个键空间，见方案第 3 节。
pub const LOCK_SPACE: i32 = 0x4950;
/// 第二个键：跑迁移。
pub const LOCK_MIGRATE: &str = "migrate";
/// 第二个键：写事件日志，保证 seq 按提交顺序递增。
pub const LOCK_EVENTS: &str = "events";

/// 按编号排好的迁移。已经发布的文件不要改，加新文件。
pub(crate) const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/0001_init.sql")),
    (2, include_str!("../migrations/0002_thread_agent.sql")),
];

/// 第二个键按 schema 名区分，同一个库里多个 schema（测试时每个测试一个）互不影响。
/// schema 名在 `DbConfig` 里校验过，只有小写字母、数字和下划线，可以直接拼进 SQL。
pub fn lock_key(schema: &str, kind: &str) -> String {
    format!("hashtext('{schema}/{kind}')")
}

/// 在 advisory lock 下建 schema（还没有时）、执行没跑过的迁移。几个进程同时启动时只有一个在跑，其余等它跑完。
pub async fn migrate(client: &mut Client, schema: &str) -> Result<()> {
    let lock = format!(
        "SELECT pg_advisory_lock({LOCK_SPACE}, {})",
        lock_key(schema, LOCK_MIGRATE)
    );
    let unlock = format!(
        "SELECT pg_advisory_unlock({LOCK_SPACE}, {})",
        lock_key(schema, LOCK_MIGRATE)
    );
    client.batch_execute(&lock).await?;
    let result = async {
        // 先查再建：客户给的账号可能没有建 schema 的权限，schema 已经由管理员建好时不该报错。
        let exists = client
            .query_opt("SELECT 1 FROM pg_namespace WHERE nspname = $1", &[&schema])
            .await?
            .is_some();
        if !exists {
            client.batch_execute(&format!("CREATE SCHEMA {schema}")).await?;
        }
        run_pending(client).await
    }
    .await;
    // 解锁失败也不要紧：连接关掉时 PostgreSQL 会自动释放会话级锁。
    let _ = client.batch_execute(&unlock).await;
    result
}

async fn run_pending(client: &mut Client) -> Result<()> {
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version    BIGINT PRIMARY KEY,
                 applied_ms BIGINT NOT NULL
             )",
        )
        .await?;
    let done: Vec<i64> = client
        .query("SELECT version FROM schema_migrations", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    for (version, sql) in MIGRATIONS {
        if done.contains(version) {
            continue;
        }
        let transaction = client.transaction().await?;
        transaction.batch_execute(sql).await?;
        transaction
            .execute(
                "INSERT INTO schema_migrations (version, applied_ms) VALUES ($1, $2)",
                &[version, &ip_core::now_ms()],
            )
            .await?;
        transaction.commit().await?;
        tracing::info!(version, "数据库迁移完成");
    }
    Ok(())
}
