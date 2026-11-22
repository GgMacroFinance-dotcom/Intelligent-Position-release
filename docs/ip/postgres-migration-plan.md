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
- 单一写入者：保留“一个写任务、同时到达的写合并成一个事务、每个请求一个保存点”的做法，用一条专用连接。
- 事件顺序：`ip context`、`ip device pair` 会在服务运行时另开进程写同一个库（SQLite 时就是这样），所以不能用“一个库只许一个进程”的锁。改为每个写事务开头取一把事务级 advisory lock（`pg_advisory_xact_lock`，提交时释放），所有写事件的事务排队提交，`events.seq` 按提交顺序递增，渠道游标的语义不变。seq 可能跳号（回滚的事务不收回序号），游标只要求递增，不受影响。

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
- 实测（2026-11-22，`ip bench store`，PostgreSQL 16 和基准在同一台机器上）：照搬 SQLite 的写法 p95 是 41 ms。做了这几件以后 p95 7–8.4 ms、p50 3–3.6 ms，达到 10 ms 的目标：
  - 先只搜最新的 1 万条消息，命中凑不够排序窗口再把范围放大四倍，直到覆盖全部历史。短语匹配要逐条读出命中消息的位置信息，常见词在 5 万条里命中 1 万条时光这一步就要十几毫秒。
  - 排序窗口 `RANK_WINDOW` 从 2000 减到 1000：`ts_rank` 逐条解析位置信息，2000 条要 7 ms 左右。
  - 连接设 `plan_cache_mode=force_custom_plan`：预编译语句执行几次后 PostgreSQL 改用通用计划，范围条件会选错索引。
  - GIN 索引 `fastupdate = off`：开着时新写入先进待合并列表，刚写完一批消息时每次搜索都要线性扫它。代价是写入慢一些。
  - 基准灌完数据后手动 `ANALYZE` 一次，量的是平稳状态（长期运行的库由 autovacuum 做）。
- 写入吞吐从 SQLite 的约 1 万条/秒降到约 1,600 条/秒（每条消息几次网络往返）。员工的消息量远低于这个数，暂时不优化。
- 岗位记忆搜索（`memory_search`）用 C 排序规则的 `lower()` 加 `strpos`，2000 份时 p95 约 6–7 ms（SQLite 时约 8 ms）。

### 4.5 迁移文件

`ip-store/migrations/0001_init.sql` 起，编译进二进制（`include_str!`），启动时在 advisory lock 下按版本号执行没跑过的，记进 `ip.schema_migrations`。不引入迁移框架，几十行代码。

### 4.6 配置

```yaml
database:
  url_env: IP_DATABASE_URL   # 连接串从环境变量读，密码不进 ip.yaml
  schema: ip
  max_connections: 8
```

第一版只支持不加密的连接（`NoTls`），够连同一台机器或同一个 compose 网络里的库；连 RDS、Supabase 这类要 TLS 的外部库，后面单独加（`tokio-postgres-rustls`）。

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

一次性命令 `ip store import-sqlite --config ip.yaml [--from <旧库>]`（省略 `--from` 时读 `<data_dir>/ip.db`，2026-11-22 已做）：

1. 目标 schema 必须是空的（只跑过建表迁移），否则拒绝，避免重复导入。
2. 一个事务里按依赖顺序拷：`events`（保留原 `seq`）→ 各投影表 → 渠道表 → `engine_sessions` → 记忆表。`messages.search` 由导入时重新计算，不拷 FTS5 的索引。
3. 把 `events`、`memory_notes` 的自增起点调到最大值之后。
4. 校验：提交前逐表核对行数，有一张对不上就整个回滚。（原计划还要抽查搜索结果两边一致；排序从 bm25 换成了 `ts_rank`，结果顺序本来就会不同，改成在测试里查导入后能搜到。）
5. 旧库只读打开，不改不删、不改名；确认服务正常后由人自己挪走。
6. 旧库没有的表跳过；早期 `engine_sessions` 没有的用量列按 0。NUL 字符换成替换字符。导入完 `ANALYZE` 一次。

## 7. mobius 怎么迁

mobius 上 I.P. 只跑运维员工小维，数据量小，停机几分钟可以接受。

1. 在 mobius 上单独起一个 `postgres:17` 容器给 I.P. 和 TCMS 用，**不和 Ai4sci 中转站的 PostgreSQL 共用**：那是另一个产品，有自己的升级、回滚演练和备份，混在一起出事会互相拖累。
2. 停 `ip-harness`，备份 `data/ip.db`。
3. 换新版二进制，`ip.yaml` 加 `database`，环境变量文件里放连接串。
4. 跑 `ip store import-sqlite --config ip.yaml`，看每张表的行数。
5. 启动服务，在控制台搜一条历史消息、看一眼待办和审批。
6. 回滚：换回旧二进制和旧 `ip.yaml`，`ip.db` 还在。

这一步是改生产主机，到时候单独请维护者点头再做。

## 8. 拆成几个 PR

1. 本方案、backlog 加一条、`project-spec.md` 记下决定（只改文档，#55）。
2. `ip-store` 换成 PostgreSQL，连同配置 `database`、docker-compose 和初始化脚本、CI 的 PostgreSQL、README 和部署文档。原计划把配置和部署单拆一个 PR，但不一起改的话 main 上用 Docker 部署会起不来，所以合在一起。删掉 `rusqlite`。
3. `ip store import-sqlite` 和导入测试（`ip-store` 的 `import-sqlite` 功能，只有 `ip-cli` 打开，`rusqlite` 只用来读旧库）。
4. 连外部库的 TLS。
5. mobius 上执行（不是 PR，单独确认）。

## 9. 已定的（维护者 2026-11-22）

1. **不留 SQLite。** 只有 PostgreSQL 一套后端，`rusqlite` 在第 2 个 PR 里删掉。单机试用用 `docker compose up` 一起起 PostgreSQL。
2. **mobius 上 PostgreSQL 单独起一个容器**，不和 Ai4sci 中转站的共用。
3. **JSON 列用 `JSONB`。**
