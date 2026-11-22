# I.P. 存储层改用 PostgreSQL：迁移方案

状态：维护者 2026-11-22 确认，第 9 节三条都按建议定。

## 1. 为什么改

维护者 2026-11-22 定：新的团队上下文系统（仓库 `Team-context-management-system`，下称 TCMS）和 I.P. 用同一个 PostgreSQL。TCMS 只依赖标准 PostgreSQL（兼容 Supabase，但不依赖 Supabase 特有的东西），I.P. 跟着走同一条线。I.P. 部署在打包卖出的智能岗位云服务器上，客户团队可以自己改造部署，所以选型要满足：

- 只用标准 PostgreSQL，不装扩展。客户可以换成自己的 PostgreSQL、RDS 或 Supabase。
- 一台服务器上一个 PostgreSQL 进程，两个系统各用一个 schema，互不碰表。
- 依赖许可证和 Apache-2.0 兼容。

## 2. 现状（2026-11-22 读的 main，312a76b）

- 存储全在 `rust/crates/ip-store`，用 `rusqlite`（内置 SQLite）。别的 crate 不直接用 `rusqlite`，只用 `Store` 的约 50 个异步方法。这是好消息：对外接口可以不变，改动集中在一个 crate。
- `Store::open(<data_dir>/ip.db)` 在 `ip-server/src/app.rs`、`check.rs`，`ip-cli` 的 `main.rs` 和两个基准里调用；测试里用临时目录开库。
- 写：一个专用写线程，把同时到达的写请求合并进一个事务（每个请求一个保存点）。读：`spawn_blocking` 加只读连接池，WAL 模式。
- 表（`schema.rs`）：`events`（只追加事件日志，`seq` 自增）、投影表 `messages`、`threads`、`tasks`、`approvals`、`settings`；渠道运行状态 `channel_bindings`、`channel_inbox`、`channel_envelopes`、`channel_deliveries`、`channel_cursors`；`engine_sessions`；记忆 `memory_thread_summaries`、`memory_notes`；全文索引 `messages_fts`（FTS5，contentless）。
- 中文搜索：`segment.rs` 把中日韩文字切成重叠二字词再交给 FTS5 `unicode61`；单个汉字用前缀查询。全库搜索只在最近 2000 条命中里排序（`RANK_WINDOW`）。岗位记忆搜索用 `instr(lower(...))` 子串匹配。
- 依赖顺序的地方：渠道出站按 `events.seq` 游标读（`events_after`、`channel_cursors`），要求 seq 按提交顺序单调递增。现在靠单一写线程保证。
- 基准：`ip bench store`（5 万条写入、搜索 p95 目标 10 ms，现在 5.4 ms；每轮读岗位上下文 p95 目标 5 ms）、`ip bench agents`、`ip bench startup`。
- 部署：`rust/docker-compose.yml` 一个 harness 容器加数据卷；mobius 上是 systemd 服务 `ip-harness`，`data_dir: /home/ubuntu/ip/data`，`MemoryMax=2G`。mobius 上已有 Ai4sci 中转站自己的 PostgreSQL 容器（`newapi-sidecar-postgres-1`）。
- 敏感路径：`ip-store` 不在 `rust/CONTRIBUTING.md` 的清单里；但新增的数据库密码读取落在 `config.rs` 读密钥的部分，而且迁移会动到所有历史数据，按“拿不准就按敏感路径处理”，这几个 PR 都请一位工程师读过再合并。

## 3. schema 划分

- 一个库（建议库名 `ip`），两个 schema：`ip` 放 I.P. 的表，`tcms` 放团队上下文的表。
- 两个登录角色：`ip_app` 拥有 `ip` schema，`tcms_app` 拥有 `tcms` schema；互相没有任何权限（`REVOKE ALL ON SCHEMA ... FROM PUBLIC`）。跨系统只走 TCMS 的 MCP 接口，不写跨 schema 的 SQL。
- I.P. 连接时设 `search_path = ip`，代码里不写 schema 前缀，schema 名可配置（客户放在已有库里时可以改名）。
- 各自的迁移记录表在自己 schema 里：I.P. 是 `ip.schema_migrations`；TCMS 用 sqlx，落在 `tcms._sqlx_migrations`。两边可以各自升级。
- advisory lock：sqlx 的迁移锁键按库名算（`crc32(current_database())` 乘一个常数），I.P. 用自己固定的两键形式 `pg_advisory_lock(0x4950, n)`（0x4950 是 “IP”），和 sqlx 的单键锁不在同一个键空间，不会撞。
- LISTEN/NOTIFY 的频道名是库级的：I.P. 以后若用，一律 `ip_` 前缀；TCMS 用 `tcms_` 前缀。
- 第一版两边都不装扩展。TCMS 以后语义检索可能要 pgvector，到时镜像换成 `pgvector/pgvector:pg17`，是可选项。TCMS 的全文检索用进程内的 tantivy，不受 I.P. 分词方案影响。
- 以上 2026-11-22 和 TCMS 线程对过，没有冲突。

## 4. 代码怎么改

### 4.1 驱动

建议 `tokio-postgres` + `deadpool-postgres`（都是 MIT/Apache-2.0），TLS 用 `tokio-postgres-rustls`（连 Supabase、RDS 等远端库要 TLS）。不选 `sqlx`：它的编译期查SQL要么要连库、要么要提交离线缓存，编译也重；我们的 SQL 都手写，用不上它的宏。`Store` 的方法本来就是 `async`，换成真正的异步驱动后读不再需要 `spawn_blocking`。

### 4.2 保持不变的

- `Store` 的公开方法、`StoreError` 以外的类型都不变，调用方只改 `Store::open` 一处（改成接收数据库配置）。`StoreError::Sqlite` 换成 `StoreError::Db`。
- 单一写入者：保留“一个写任务、同时到达的写合并成一个事务、每个请求一个保存点”的做法，用一条专用连接。这同时保证了 `events.seq` 按提交顺序递增，渠道游标的语义不变。代价是同一个库只能有一个 I.P. 进程在写，这和现在一样；启动时用 `pg_try_advisory_lock` 占住，第二个进程报错退出，不会悄悄乱序。

### 4.3 表结构对照

| SQLite | PostgreSQL |
| --- | --- |
| `INTEGER`（时间、计数） | `BIGINT` |
| `INTEGER PRIMARY KEY AUTOINCREMENT` | `BIGINT GENERATED ALWAYS AS IDENTITY` |
| `WITHOUT ROWID` | 普通表 |
| `TEXT` 存的 JSON（`payload`、`json`） | `JSONB`（读回时由 serde 反序列化，键顺序变化不影响） |
| 0/1 整数当布尔（`pending`、`selected`） | `BOOLEAN` |
| `INSERT OR IGNORE` / `INSERT OR REPLACE` | `ON CONFLICT DO NOTHING` / `ON CONFLICT ... DO UPDATE` |
| `instr(lower(x), ?)` | `strpos(lower(x), $n) > 0` |
| `add_column` 查 `pragma_table_info` | 版本化迁移文件（见 4.5） |

### 4.4 中文全文搜索

不装 `zhparser`、`pg_jieba` 之类的扩展（客户的库、Supabase 不一定有），继续用我们自己的二字切分：

- `messages` 加一列 `search tsvector`，写入时在 Rust 里算 `index_text(text)`，用 `to_tsvector('simple', $1)` 存；建 GIN 索引。
- `match_query` 改为生成 `tsquery`：每个词内部的二字词用 `<->` 连成短语，词之间 `&`；单个汉字用前缀 `字:*`。这和现在 FTS5 的语义一一对应。
- 排序：FTS5 的 bm25 换成 `ts_rank`，排序结果会有细微差别。继续只在最近 2000 条命中里排序。
- 要重新量：`ip bench store` 的搜索 p95 目标仍是 10 ms。PostgreSQL 走本机套接字，每次查询多一次往返；GIN 上的短语查询要回表核对位置，常见词可能变慢。量出来超标再考虑把短语匹配改成只用 `&`（多搜到一点，精度略降）。

### 4.5 迁移文件

`ip-store/migrations/0001_init.sql` 起，编译进二进制（`include_str!`），启动时在 advisory lock 下按版本号执行没跑过的，记进 `ip.schema_migrations`。不引入迁移框架，几十行代码。

### 4.6 配置

```yaml
database:
  url_env: IP_DATABASE_URL   # 连接串从环境变量读，密码不进 ip.yaml
  schema: ip
  max_connections: 8
```

连 Supabase 时用直连或 session 模式的连接池地址，不能用 transaction 模式（6543 端口），因为 `tokio-postgres` 用预编译语句。

### 4.7 测试和 CI

- 测试需要一个 PostgreSQL：读 `IP_TEST_DATABASE_URL`，每个测试建一个随机名字的 schema、结束时删掉，测试之间互不影响、可以并行。没设这个变量时，存储相关测试报错而不是跳过（不能静默不测）。
- `rust` 工作流加一个 `postgres:17` service 容器。基准也在这个容器上跑。
- 测试辅助函数集中在 `ip-store` 里，其他 crate 的测试（`ip-server`、`ip-runtime` 里开 `Store` 的那些）改成调用它。

### 4.8 性能和内存

harness 自身的内存目标（40 MB）不变，PostgreSQL 是另一个进程。打包服务器的总内存要多算 PostgreSQL 的一份（默认配置约 100–200 MB）。`ip bench startup` 的启动时间要重新量，多了连库和跑迁移。

## 5. 部署

- **打包服务器**：`rust/docker-compose.yml` 加 `postgres:17` 服务和数据卷，`deploy/postgres-init/` 放初始化脚本（建 `ip_app`、`tcms_app` 两个角色和两个 schema），密码从 `.env` 读（`deploy/env.example` 里只放占位）。PostgreSQL 只监听容器网络，不映射到主机端口。TCMS 的容器连同一个服务。
- **备份**：每天 `pg_dump -Fc` 一次，按 schema 分开导（`-n ip`、`-n tcms`），保留 30 天，做法照 Ai4sci 中转站已有的备份。替代现在“拷 `ip.db` 文件”。
- **客户自己的库**：给一段手工建角色、建 schema 的 SQL，客户改连接串就能用。
- **本地开发**：`docker compose up postgres` 起一个；README 写清楚。

## 6. 数据迁移

新增一次性命令 `ip store import-sqlite --from <data_dir>/ip.db`：

1. 目标 schema 必须是空的（只跑过建表迁移），否则拒绝，避免重复导入。
2. 一个事务里按依赖顺序拷：`events`（保留原 `seq`）→ 各投影表 → 渠道表 → `engine_sessions` → 记忆表。`messages.search` 由导入时重新计算，不拷 FTS5 的索引。
3. 把 `events`、`memory_notes` 的自增起点调到最大值之后。
4. 校验：每张表行数、`last_seq`、抽几条搜索在两边结果一致，有一项不对就回滚。
5. 原 `ip.db` 不删，改名留作备份。

## 7. mobius 怎么迁

mobius 上 I.P. 只跑运维员工小维，数据量小，停机几分钟可以接受。

1. 在 mobius 上单独起一个 `postgres:17` 容器给 I.P. 和 TCMS 用，**不和 Ai4sci 中转站的 PostgreSQL 共用**：那是另一个产品，有自己的升级、回滚演练和备份，混在一起出事会互相拖累。
2. 停 `ip-harness`，备份 `data/ip.db`。
3. 换新版二进制，`ip.yaml` 加 `database`，环境变量文件里放连接串。
4. 跑 `ip store import-sqlite`，看校验结果。
5. 启动服务，在控制台搜一条历史消息、看一眼待办和审批。
6. 回滚：换回旧二进制和旧 `ip.yaml`，`ip.db` 还在。

这一步是改生产主机，到时候单独请维护者点头再做。

## 8. 拆成几个 PR

1. 本方案、backlog 加一条、`project-spec.md` 记下决定（只改文档）。
2. `ip-store` 换成 PostgreSQL：迁移文件、写任务、读池、搜索；测试和 CI 加 PostgreSQL；`ip bench store` 重新量。`Store::open` 的调用方跟着改。删掉 `rusqlite`。
3. 配置 `database`、docker-compose 和初始化脚本、README、部署文档。
4. `ip store import-sqlite` 和导入测试。
5. mobius 上执行（不是 PR，单独确认）。

2 和 3 可能要合在一起才能编译通过，到时候看。

## 9. 已定的（维护者 2026-11-22）

1. **不留 SQLite。** 只有 PostgreSQL 一套后端，`rusqlite` 在第 2 个 PR 里删掉。单机试用用 `docker compose up` 一起起 PostgreSQL。
2. **mobius 上 PostgreSQL 单独起一个容器**，不和 Ai4sci 中转站的共用。
3. **JSON 列用 `JSONB`。**
