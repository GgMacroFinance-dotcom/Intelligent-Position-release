//! 命令行入口 `ip`：`ip serve` 启动服务，`ip check` 检查配置能不能用，`ip data pull` 拉取公开数据，
//! `ip context` 查看和修改岗位的决策者笔记和记忆摘要，`ip bench` 跑性能基准，`ip eval` 用真实模型跑评测集。

mod bench;
mod eval;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
#[command(name = "ip", version, about = "intelligent-position harness")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 启动 harness 服务。
    Serve {
        /// 监听地址；端口给 0 由系统分配。
        #[arg(long, default_value = "127.0.0.1:7878")]
        addr: SocketAddr,
        /// 组织配置文件（见 ip.example.yaml）。省略时只提供健康检查。
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// 逐项检查配置里的端点、密钥、数据源、MCP 服务器、Codex 和 Slack 能不能用。有失败时退出码为 1。
    Check {
        /// 组织配置文件。
        #[arg(long)]
        config: PathBuf,
    },
    /// 数据源相关操作。
    #[command(subcommand)]
    Data(DataCommand),
    /// 岗位的决策者笔记和记忆摘要：查看、写入、清空。服务在运行时也能用，下一轮对话生效。
    #[command(subcommand)]
    Context(ContextCommand),
    /// 配对手机、电脑（backlog F-7）。改名和吊销在控制台“设备”页做。
    #[command(subcommand)]
    Device(DeviceCommand),
    /// 跑性能基准，结果以一行 JSON 打印。
    Bench(bench::BenchArgs),
    /// 用真实模型跑评测集，按程序判定的检查项打分，输出通过率和花费。全部通过时退出码为 0。
    Eval(eval::EvalArgs),
}

#[derive(Subcommand)]
enum DataCommand {
    /// 按 catalog.yaml 里的 fetch 拉取国家统计局、FRED 等公开数据，并入 CSV。适合放进定时任务。
    Pull {
        /// 组织配置文件，用其中的 data_sources。
        #[arg(long)]
        config: PathBuf,
        /// 只拉这个数据源（名字）或这个序列（<数据源>/<序列>）。
        #[arg(long)]
        only: Option<String>,
    },
}

#[derive(Subcommand)]
enum DeviceCommand {
    /// 生成一个 2 分钟内有效、只能用一次的 6 位配对码。服务在运行时也能用。
    Pair {
        #[arg(long)]
        config: PathBuf,
        /// 配对后这台设备叫什么，比如“张三的手机”。用它连上后发的消息署这个名字。
        #[arg(long)]
        name: String,
        /// 控制台对外的地址，比如 https://ip.example.com。给了就在终端里画出二维码，手机扫码即可。
        #[arg(long)]
        url: Option<String>,
    },
}

#[derive(Subcommand)]
enum ContextCommand {
    /// 打印一个岗位的决策者笔记和记忆摘要。
    Show {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        role: String,
    },
    /// 用文件内容整体覆盖一层。
    Set {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        role: String,
        #[arg(value_enum)]
        layer: LayerArg,
        /// UTF-8 文本文件。
        #[arg(long)]
        file: PathBuf,
    },
    /// 清空一层。
    Clear {
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        role: String,
        #[arg(value_enum)]
        layer: LayerArg,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum LayerArg {
    /// 决策者笔记。
    Notes,
    /// 岗位记忆摘要。
    Memory,
}

impl From<LayerArg> for ip_runtime::memory::Layer {
    fn from(layer: LayerArg) -> Self {
        match layer {
            LayerArg::Notes => Self::Notes,
            LayerArg::Memory => Self::Summary,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve { addr, config } => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
                )
                .with_writer(std::io::stderr)
                .init();
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(async move {
                    let app = match config {
                        Some(path) => {
                            let config = ip_server::Config::load(&path)?;
                            Some(ip_server::App::from_config(&config).await?)
                        }
                        None => None,
                    };
                    ip_server::serve(addr, app).await
                })
        }
        Command::Check { config } => {
            let ok = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(check(config));
            if !ok {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Data(DataCommand::Pull { config, only }) => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(pull(config, only)),
        Command::Context(command) => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(context(command)),
        Command::Device(command) => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(device(command)),
        Command::Bench(args) => bench::run(args),
        Command::Eval(args) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
                )
                .with_writer(std::io::stderr)
                .init();
            eval::run(args)
        }
    }
}

/// 逐个序列打印结果；有失败时以非零状态退出，方便定时任务报警。
async fn pull(config: PathBuf, only: Option<String>) -> anyhow::Result<()> {
    let config = ip_server::Config::load(&config)?;
    if config.data_sources.is_empty() {
        anyhow::bail!("配置里没有 data_sources");
    }
    let fetcher = ip_data::fetch::Fetcher::new().map_err(anyhow::Error::msg)?;
    let reports = ip_data::pull(
        &config.data_sources,
        std::path::Path::new("."),
        &fetcher,
        only.as_deref(),
    )
    .await?;
    if reports.is_empty() {
        println!("没有写了 fetch 的序列");
        return Ok(());
    }
    let mut failed = 0;
    for r in &reports {
        match &r.outcome {
            Ok(ok) => {
                let upstream = ok
                    .upstream_name
                    .as_deref()
                    .map(|n| format!("，指标：{n}"))
                    .unwrap_or_default();
                println!(
                    "✓ {}：取到 {} 期，最新 {}，共 {} 期{upstream}",
                    r.series, ok.fetched, ok.latest, ok.total
                );
            }
            Err(e) => {
                failed += 1;
                println!("✗ {}：{e}", r.series);
            }
        }
    }
    if failed > 0 {
        anyhow::bail!("{failed} 个序列拉取失败");
    }
    Ok(())
}

/// `ip device pair`：直接把配对码写进数据库，运行中的服务换令牌时读得到。
async fn device(command: DeviceCommand) -> anyhow::Result<()> {
    let DeviceCommand::Pair { config, name, url } = command;
    let config = ip_server::Config::load(&config)?;
    std::fs::create_dir_all(&config.data_dir)?;
    let store = ip_store::Store::open(config.data_dir.join("ip.db"))?;
    let pairing = ip_server::devices::create_pairing(&store, &name, url.as_deref(), ip_core::now_ms())
        .await
        .map_err(anyhow::Error::msg)?;
    if let Some(link) = &pairing.link {
        if let Some(qr) = ip_server::devices::qr_terminal(link) {
            println!("{qr}");
        }
        println!("用手机相机扫上面的二维码，或者打开 {link}");
    } else if url.is_some() {
        anyhow::bail!("--url 要以 https:// 或 http:// 开头");
    }
    println!("配对码：{}（{}，2 分钟内有效，只能用一次）", pairing.code, pairing.name);
    println!("在控制台点“令牌”，输入这 6 位数字也可以。");
    Ok(())
}

/// `ip context`：直接读写数据库里的设置。和运行中的服务用同一个库，写入后下一轮对话就会带上。
async fn context(command: ContextCommand) -> anyhow::Result<()> {
    use ip_runtime::memory::{self, Layer};

    let (path, role) = match &command {
        ContextCommand::Show { config, role }
        | ContextCommand::Set { config, role, .. }
        | ContextCommand::Clear { config, role, .. } => (config, role),
    };
    let config = ip_server::Config::load(path)?;
    let role = ip_core::RoleId::from(role.as_str());
    if !config.roles.iter().any(|r| r.id == role) {
        anyhow::bail!("配置里没有岗位 {role}");
    }
    // 服务还没启动过时数据目录不存在，先建出来，这样可以在第一次启动前就把笔记写好。
    std::fs::create_dir_all(&config.data_dir)?;
    let store = ip_store::Store::open(config.data_dir.join("ip.db"))?;
    match command {
        ContextCommand::Show { .. } => {
            for (layer, title, max) in [
                (Layer::Notes, "决策者笔记", config.memory.notes_max_chars),
                (Layer::Summary, "岗位记忆摘要", config.memory.summary_max_chars),
            ] {
                match memory::get(&store, layer, &role).await? {
                    Some(entry) => {
                        let chars = entry.text.chars().count();
                        let over = if chars > max {
                            format!("，超过上限 {max} 字，只有前面的部分会放进提示词")
                        } else {
                            String::new()
                        };
                        println!("# {title}（{chars} 字{over}）\n{}\n", entry.text);
                    }
                    None => println!("# {title}\n（空）\n"),
                }
            }
            let threads = store.thread_memories(&role, 50).await?;
            println!("# 从对话里提炼的摘要（最近 {} 份）", threads.len());
            for m in &threads {
                println!(
                    "\n## {}（对话 {}）\n{}",
                    m.slug.as_deref().unwrap_or("无标题"),
                    m.thread_id,
                    m.summary.as_deref().unwrap_or_default()
                );
            }
            println!();
            if !config.memory.enabled_for(&role) {
                println!(
                    "注意：这个岗位没有开记忆（memory.enabled），岗位记忆摘要不会放进提示词，也不会从对话里提炼；决策者笔记不受影响。"
                );
            }
        }
        ContextCommand::Set { layer, file, .. } => {
            let text = std::fs::read_to_string(&file)?;
            if text.trim().is_empty() {
                anyhow::bail!("{} 是空的；要清空请用 ip context clear", file.display());
            }
            memory::put(&store, layer.into(), &role, &text, ip_core::now_ms()).await?;
            println!("已写入 {} 字", text.trim().chars().count());
        }
        ContextCommand::Clear { layer, .. } => {
            memory::put(&store, layer.into(), &role, "", ip_core::now_ms()).await?;
            println!("已清空");
        }
    }
    Ok(())
}

/// `ip check`：打印每项检查的结果，返回是否全部没有失败。
async fn check(path: PathBuf) -> bool {
    let config = match ip_server::Config::load(&path) {
        Ok(c) => c,
        Err(e) => {
            println!("✗ 配置文件 {}：{e}", path.display());
            return false;
        }
    };
    println!("✓ 配置文件 {}：格式正确", path.display());
    let report = ip_server::check::check(&config, &|var| std::env::var(var).ok()).await;
    print!("{}", report.render());
    !report.failed()
}
