//! 建表和连接设置。

use rusqlite::Connection;

pub fn configure(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )
}

pub fn configure_reader(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch("PRAGMA busy_timeout = 5000;")
}

pub fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS events (
             seq     INTEGER PRIMARY KEY AUTOINCREMENT,
             at_ms   INTEGER NOT NULL,
             kind    TEXT NOT NULL,
             payload TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS messages (
             rowid     INTEGER PRIMARY KEY,
             id        TEXT NOT NULL UNIQUE,
             thread_id TEXT NOT NULL,
             at_ms     INTEGER NOT NULL,
             text      TEXT,
             json      TEXT NOT NULL,
             seq       INTEGER NOT NULL REFERENCES events(seq)
         );
         CREATE INDEX IF NOT EXISTS messages_by_thread ON messages(thread_id, at_ms);
         CREATE TABLE IF NOT EXISTS threads (
             id            TEXT PRIMARY KEY,
             last_at_ms    INTEGER NOT NULL,
             message_count INTEGER NOT NULL,
             preview       TEXT
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS threads_by_time ON threads(last_at_ms);
         -- 早于 threads 表建立的库：从 messages 补一次。
         INSERT OR IGNORE INTO threads (id, last_at_ms, message_count, preview)
             SELECT thread_id, MAX(at_ms), COUNT(*), NULL FROM messages
             WHERE NOT EXISTS (SELECT 1 FROM threads) GROUP BY thread_id;
         CREATE TABLE IF NOT EXISTS tasks (
             id            TEXT PRIMARY KEY,
             status        TEXT NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             json          TEXT NOT NULL,
             seq           INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS tasks_by_status ON tasks(status, updated_at_ms);
         CREATE TABLE IF NOT EXISTS approvals (
             id            TEXT PRIMARY KEY,
             pending       INTEGER NOT NULL,
             created_at_ms INTEGER NOT NULL,
             json          TEXT NOT NULL,
             seq           INTEGER NOT NULL
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS approvals_by_pending ON approvals(pending, created_at_ms);
         CREATE TABLE IF NOT EXISTS settings (
             key   TEXT PRIMARY KEY,
             json  TEXT NOT NULL,
             seq   INTEGER NOT NULL
         ) WITHOUT ROWID;
         -- 外部渠道（docs/ip/m2-spec.md D-08）。绑定走事件日志（binding.put），其余三张是运行状态。
         CREATE TABLE IF NOT EXISTS channel_bindings (
             channel       TEXT NOT NULL,
             conversation  TEXT NOT NULL,
             thread_id     TEXT NOT NULL,
             agent_id      TEXT,
             requester     TEXT NOT NULL,
             created_at_ms INTEGER NOT NULL,
             seq           INTEGER NOT NULL,
             PRIMARY KEY (channel, conversation)
         ) WITHOUT ROWID;
         CREATE UNIQUE INDEX IF NOT EXISTS channel_bindings_by_thread ON channel_bindings(thread_id);
         CREATE TABLE IF NOT EXISTS channel_inbox (
             channel    TEXT NOT NULL,
             dedupe_key TEXT NOT NULL,
             at_ms      INTEGER NOT NULL,
             PRIMARY KEY (channel, dedupe_key)
         ) WITHOUT ROWID;
         -- 渠道收到、确认之前先原样存下的信封（#69）。处理完标成 done（清掉内容），重试多次仍失败的标成
         -- dropped 留着查；两种都在去重记录的保留期满后删掉。
         CREATE TABLE IF NOT EXISTS channel_envelopes (
             rowid       INTEGER PRIMARY KEY,
             channel     TEXT NOT NULL,
             id          TEXT NOT NULL,
             payload     TEXT NOT NULL,
             received_ms INTEGER NOT NULL,
             attempts    INTEGER NOT NULL DEFAULT 0,
             state       TEXT NOT NULL DEFAULT 'pending',
             error       TEXT,
             UNIQUE (channel, id)
         );
         CREATE TABLE IF NOT EXISTS channel_deliveries (
             channel      TEXT NOT NULL,
             item         TEXT NOT NULL,
             conversation TEXT NOT NULL,
             external_ref TEXT,
             state        TEXT NOT NULL,
             error        TEXT,
             at_ms        INTEGER NOT NULL,
             PRIMARY KEY (channel, item)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS channel_cursors (
             channel TEXT PRIMARY KEY,
             seq     INTEGER NOT NULL
         ) WITHOUT ROWID;
         -- 外部引擎（Codex 等）在每个对话里的会话：下一轮用它续接，只交新消息。
         -- 运行状态，不写事件日志。见 docs/ip/m2-spec.md D-23。
         CREATE TABLE IF NOT EXISTS engine_sessions (
             agent_id      TEXT NOT NULL,
             thread_id     TEXT NOT NULL,
             engine        TEXT NOT NULL,
             session_id    TEXT NOT NULL,
             last_message  TEXT NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             -- 这个会话到上一轮为止累计报告的用量；续接时引擎报的是累计值，记账只记差值（D-23）。
             used_prompt_tokens      INTEGER NOT NULL DEFAULT 0,
             used_cached_tokens      INTEGER NOT NULL DEFAULT 0,
             used_cache_write_tokens INTEGER NOT NULL DEFAULT 0,
             used_completion_tokens  INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (agent_id, thread_id)
         ) WITHOUT ROWID;
         -- 岗位记忆：一个对话在一个岗位下的摘要（docs/ip/context-memory-design.md 5.8）。
         -- 是从对话里提炼出来的，丢了可以重新提炼，所以不走事件日志。
         CREATE TABLE IF NOT EXISTS memory_thread_summaries (
             role_id           TEXT NOT NULL,
             thread_id         TEXT NOT NULL,
             source_updated_ms INTEGER NOT NULL,
             generated_ms      INTEGER NOT NULL,
             status            TEXT NOT NULL,
             slug              TEXT,
             summary           TEXT,
             attempts          INTEGER NOT NULL DEFAULT 0,
             retry_after_ms    INTEGER,
             usage_count       INTEGER NOT NULL DEFAULT 0,
             last_usage_ms     INTEGER,
             selected          INTEGER NOT NULL DEFAULT 0,
             PRIMARY KEY (role_id, thread_id)
         ) WITHOUT ROWID;
         CREATE TABLE IF NOT EXISTS memory_notes (
             id          INTEGER PRIMARY KEY AUTOINCREMENT,
             role_id     TEXT NOT NULL,
             created_ms  INTEGER NOT NULL,
             thread_id   TEXT,
             text        TEXT NOT NULL,
             applied_ms  INTEGER
         );
         CREATE INDEX IF NOT EXISTS memory_notes_by_role ON memory_notes (role_id, created_ms);
         CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
             body, content = '', contentless_delete = 1, tokenize = 'unicode61'
         );",
    )?;
    // 旧库的 engine_sessions 没有累计用量的四列，补上；默认 0，旧会话的下一轮按全量记一次。
    for column in [
        "used_prompt_tokens",
        "used_cached_tokens",
        "used_cache_write_tokens",
        "used_completion_tokens",
    ] {
        add_column(connection, "engine_sessions", column, "INTEGER NOT NULL DEFAULT 0")?;
    }
    Ok(())
}

/// 表里没有这一列时加上。表名、列名和类型都是代码里写死的。
fn add_column(connection: &Connection, table: &str, column: &str, definition: &str) -> rusqlite::Result<()> {
    let exists = connection
        .prepare(&format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?1"))?
        .exists([column])?;
    if !exists {
        connection.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition};"))?;
    }
    Ok(())
}
