//! 测试用的数据库：每个 [`TestDb`] 一个临时 schema，最后一个句柄丢掉时删掉。
//!
//! 连接串从环境变量 `IP_TEST_DATABASE_URL` 读，例如本机
//! `docker compose up -d postgres` 之后设成 `postgres://ip:ip@127.0.0.1:5432/ip`。
//! 没设时直接 panic：存储测试不能悄悄跳过。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once};

use crate::{DbConfig, Store};

pub const URL_ENV: &str = "IP_TEST_DATABASE_URL";

/// 一个临时 schema。可以开多次（测“重开以后数据还在”）。
pub struct TestDb {
    config: DbConfig,
    guard: Arc<Guard>,
}

impl TestDb {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let url = std::env::var(URL_ENV).unwrap_or_else(|_| {
            panic!("存储测试要一个 PostgreSQL：设环境变量 {URL_ENV}，见 rust/README.md 的“本地开发”")
        });
        static CLEAN: Once = Once::new();
        CLEAN.call_once(|| drop_stale_schemas(&url));
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let schema = format!(
            "ip_test_{}_{}_{nanos}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let mut config = DbConfig::new(url);
        config.schema = schema;
        config.max_connections = 4;
        Self {
            guard: Arc::new(Guard(config.clone())),
            config,
        }
    }

    pub fn config(&self) -> &DbConfig {
        &self.config
    }

    /// 打开这个 schema；返回的 `Store` 活着时 schema 不会被删。
    pub async fn open(&self) -> Store {
        let guard: Arc<dyn Send + Sync> = self.guard.clone();
        Store::connect_with_guard(&self.config, Some(guard))
            .await
            .expect("连接测试数据库")
    }
}

impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

/// 开一个只用一次的临时库。
pub async fn store() -> Store {
    TestDb::new().open().await
}

/// 按路径记住的临时库：以前用 SQLite 文件路径开库的测试改动最小，同一个路径再开就是同一个库
/// （测“重启以后还在”）。这些库到进程结束都不删，下一次跑测试时由 [`drop_stale_schemas`] 清掉。
pub async fn store_at(path: impl AsRef<Path>) -> Store {
    static BY_PATH: Mutex<Option<HashMap<PathBuf, Arc<TestDb>>>> = Mutex::new(None);
    let db = {
        let mut map = BY_PATH.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        map.get_or_insert_with(HashMap::new)
            .entry(path.as_ref().to_path_buf())
            .or_insert_with(|| Arc::new(TestDb::new()))
            .clone()
    };
    db.open().await
}

/// 删掉已经退出的测试进程留下的 schema（名字里带进程号）。测试中途 panic 或用了 [`store_at`] 都会留下。
fn drop_stale_schemas(url: &str) {
    blocking(
        move |client| async move {
            let Ok(rows) = client
                .query(
                    "SELECT nspname FROM pg_namespace WHERE nspname LIKE 'ip\\_test\\_%'",
                    &[],
                )
                .await
            else {
                return;
            };
            for row in rows {
                let name: String = row.get(0);
                let pid = name.split('_').nth(2).and_then(|p| p.parse::<u32>().ok());
                let alive =
                    pid.is_some_and(|pid| pid == std::process::id() || Path::new(&format!("/proc/{pid}")).exists());
                if !alive {
                    let _ = client
                        .batch_execute(&format!("DROP SCHEMA IF EXISTS {name} CASCADE"))
                        .await;
                }
            }
        },
        DbConfig::new(url),
    );
}

/// 在另一个线程里用自己的运行时连库做一件事：调用方可能正处在测试的运行时里，不能在这里 `block_on`。
fn blocking<F, Fut>(work: F, config: DbConfig)
where
    F: FnOnce(tokio_postgres::Client) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()>,
{
    let _ = std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
            return;
        };
        runtime.block_on(async move {
            let Ok((pg, tls, _)) = config.pg_config() else {
                return;
            };
            let Ok((client, connection)) = pg.connect(tls).await else {
                return;
            };
            tokio::spawn(connection);
            work(client).await;
        });
    })
    .join();
}

struct Guard(DbConfig);

impl Drop for Guard {
    fn drop(&mut self) {
        let schema = self.0.schema.clone();
        blocking(
            move |client| async move {
                let _ = client
                    .batch_execute(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
                    .await;
            },
            self.0.clone(),
        );
    }
}
