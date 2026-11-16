# 参与开发

## 流程

1. 开工前看一眼[未完成事项](../docs/ip/backlog.md)和开放的 PR，避免两个人（或两个 Claude）做同一件事。
2. 从 `main` 拉分支，名字用 `<你的名字>/<做什么>`，例如 `alice/store-event-log`。不要直接推 `main`。
3. 改动尽量小，一个 PR 只做一件事；PR 描述写清楚改了什么、怎么验证的、哪些还没验。
4. 合并规则（2026-11-11 定）：
   - **动到敏感路径的 PR，必须有一位工程师读过代码再合并。** 在 PR 描述里写明动了哪一类。
   - 其他 PR 本地检查通过后可以直接合并。

敏感路径是出了错会越权、泄密或者在主机上乱执行的那部分：

| 类别 | 路径 |
| --- | --- |
| 工具和沙箱 | `crates/ip-tools/` |
| 令牌与鉴权 | `crates/ip-server/src/ws.rs`、`crates/ip-server/src/mcp.rs`、`crates/ip-runtime/src/gateway.rs`，以及 `crates/ip-server/src/app.rs`、`config.rs` 里读密钥和校验令牌的部分 |
| MCP | `crates/ip-mcp/` |
| 外部渠道 | `crates/ip-connectors/`、`crates/ip-server/src/channels.rs` |
| 外部引擎 | `crates/ip-engines/`、`crates/ip-runtime/src/engine.rs` |

拿不准算不算，就按敏感路径处理。

CI 已恢复（2026-11-13）。改到 `rust/**` 或 `.github/workflows/rust.yml` 的 PR 会自动跑 `rust` 工作流，合并前要看到它通过，PR 描述里写明 CI 结果。提交信息不要加 `[skip ci]`，否则这次推送不跑 CI。只改文档的 PR 不触发 CI。仓库还是私有的，Actions 分钟数有限，提 PR 前最好先在本地跑一遍下面的检查。

## 本地检查（和 CI 一致）

```sh
cd rust
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release && ./target/release/ip bench startup --check
```

改了依赖（`Cargo.lock`、任何 `Cargo.toml`）或 `deny.toml` 的 PR，CI 另外跑 `licenses` 工作流检查依赖的许可证。本地跑同一项检查：

```sh
cargo install cargo-deny --locked    # 只需装一次
cargo deny --locked --all-features check licenses
```

## 约定

- **性能是功能的一部分。** 新增的热路径要在 `ip bench` 里有对应基准和目标值；PR 让某项基准变差时，在描述里写明原因。
- **不阻塞异步运行时。** 数据库、搜索、图像处理放到专用线程；tokio 任务里不做同步 IO。
- **数据只追加。** 不整文件重写状态；需要改动就追加事件或更新单行。
- **`unsafe` 默认禁止。** 确实需要时，在该模块单独放开并写明理由，由两人共同审查。
- **依赖和代码来源。** 只引入与 Apache-2.0 兼容的依赖（MIT、Apache-2.0、BSD 等）。允许的许可证清单在 `deny.toml`，由 CI 的 `licenses` 工作流检查；新依赖用了清单以外的许可证时，先确认它和 Apache-2.0 兼容，再加进清单，并在 PR 描述里写明。可以复用 openai/codex（Apache-2.0）的代码，复制时保留其版权声明并在文件头注明来源。nightly-lab 是非商用许可，只能参考设计，不得复制其代码。
- 注释和文档用中文，标识符用英文。

## 许可

提交到本目录的代码以 Apache-2.0 发布。
