-- I.P. 的全部表，放在自己的 schema 里（连接时 search_path 指向它，见 docs/ip/postgres-migration-plan.md 第 3 节）。
-- 从 SQLite 版的 schema.rs 照搬，类型换成 PostgreSQL 的：时间和计数 BIGINT，JSON 用 JSONB，0/1 用 BOOLEAN。

-- 只追加的事件日志。seq 按提交顺序递增：写入方在事务开头取同一把事务级 advisory lock（writer.rs）。
CREATE TABLE events (
    seq     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at_ms   BIGINT NOT NULL,
    kind    TEXT NOT NULL,
    payload JSONB NOT NULL
);

-- pos 只用来排先后（同一毫秒的消息按写入顺序）；search 是二字切分后的全文索引，没有文字时为空。
CREATE TABLE messages (
    pos       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    id        TEXT NOT NULL UNIQUE,
    thread_id TEXT NOT NULL,
    at_ms     BIGINT NOT NULL,
    text      TEXT,
    json      JSONB NOT NULL,
    seq       BIGINT NOT NULL REFERENCES events (seq),
    search    TSVECTOR
);
CREATE INDEX messages_by_thread ON messages (thread_id, at_ms);
-- fastupdate 关掉：开着时新写入先进一个待合并列表，每次搜索都要线性扫一遍它，刚写完一批消息时搜索会慢好几倍。
CREATE INDEX messages_search ON messages USING GIN (search) WITH (fastupdate = off);

CREATE TABLE threads (
    id            TEXT PRIMARY KEY,
    last_at_ms    BIGINT NOT NULL,
    message_count BIGINT NOT NULL,
    preview       TEXT
);
CREATE INDEX threads_by_time ON threads (last_at_ms);

CREATE TABLE tasks (
    id            TEXT PRIMARY KEY,
    status        TEXT NOT NULL,
    updated_at_ms BIGINT NOT NULL,
    json          JSONB NOT NULL,
    seq           BIGINT NOT NULL
);
CREATE INDEX tasks_by_status ON tasks (status, updated_at_ms);

CREATE TABLE approvals (
    id            TEXT PRIMARY KEY,
    pending       BOOLEAN NOT NULL,
    created_at_ms BIGINT NOT NULL,
    json          JSONB NOT NULL,
    seq           BIGINT NOT NULL
);
CREATE INDEX approvals_by_pending ON approvals (pending, created_at_ms);

CREATE TABLE settings (
    key  TEXT PRIMARY KEY,
    json JSONB NOT NULL,
    seq  BIGINT NOT NULL
);

-- 外部渠道（docs/ip/m2-spec.md D-08）。绑定走事件日志（binding.put），其余是运行状态。
CREATE TABLE channel_bindings (
    channel       TEXT NOT NULL,
    conversation  TEXT NOT NULL,
    thread_id     TEXT NOT NULL UNIQUE,
    agent_id      TEXT,
    requester     TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    seq           BIGINT NOT NULL,
    PRIMARY KEY (channel, conversation)
);

CREATE TABLE channel_inbox (
    channel    TEXT NOT NULL,
    dedupe_key TEXT NOT NULL,
    at_ms      BIGINT NOT NULL,
    PRIMARY KEY (channel, dedupe_key)
);

-- 渠道收到、确认之前先原样存下的信封（#69）。pos 记收到的先后。
CREATE TABLE channel_envelopes (
    pos         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    channel     TEXT NOT NULL,
    id          TEXT NOT NULL,
    payload     TEXT NOT NULL,
    received_ms BIGINT NOT NULL,
    attempts    BIGINT NOT NULL DEFAULT 0,
    state       TEXT NOT NULL DEFAULT 'pending',
    error       TEXT,
    UNIQUE (channel, id)
);

CREATE TABLE channel_deliveries (
    channel      TEXT NOT NULL,
    item         TEXT NOT NULL,
    conversation TEXT NOT NULL,
    external_ref TEXT,
    state        TEXT NOT NULL,
    error        TEXT,
    at_ms        BIGINT NOT NULL,
    PRIMARY KEY (channel, item)
);

CREATE TABLE channel_cursors (
    channel TEXT PRIMARY KEY,
    seq     BIGINT NOT NULL
);

-- 外部引擎（Codex 等）在每个对话里的会话（D-23）。运行状态，不写事件日志。
CREATE TABLE engine_sessions (
    agent_id                TEXT NOT NULL,
    thread_id               TEXT NOT NULL,
    engine                  TEXT NOT NULL,
    session_id              TEXT NOT NULL,
    last_message            TEXT NOT NULL,
    updated_at_ms           BIGINT NOT NULL,
    used_prompt_tokens      BIGINT NOT NULL DEFAULT 0,
    used_cached_tokens      BIGINT NOT NULL DEFAULT 0,
    used_cache_write_tokens BIGINT NOT NULL DEFAULT 0,
    used_completion_tokens  BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, thread_id)
);

-- 岗位记忆（docs/ip/context-memory-design.md 5.8）。从对话里提炼出来的，不走事件日志。
CREATE TABLE memory_thread_summaries (
    role_id           TEXT NOT NULL,
    thread_id         TEXT NOT NULL,
    source_updated_ms BIGINT NOT NULL,
    generated_ms      BIGINT NOT NULL,
    status            TEXT NOT NULL,
    slug              TEXT,
    summary           TEXT,
    attempts          BIGINT NOT NULL DEFAULT 0,
    retry_after_ms    BIGINT,
    usage_count       BIGINT NOT NULL DEFAULT 0,
    last_usage_ms     BIGINT,
    PRIMARY KEY (role_id, thread_id)
);

CREATE TABLE memory_notes (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    role_id    TEXT NOT NULL,
    created_ms BIGINT NOT NULL,
    thread_id  TEXT,
    text       TEXT NOT NULL,
    applied_ms BIGINT
);
CREATE INDEX memory_notes_by_role ON memory_notes (role_id, created_ms);
