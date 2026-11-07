//! Debug command line for CairnLauncher. Exercises every capability of the core crate.

mod plugins;

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cairn_core::config::MemoryPolicy;
use cairn_core::event::LogLevel;
use cairn_core::game::instance::LoaderKind;
use cairn_core::game::manifest::VersionType;
use cairn_core::install::LoaderSpec;
use cairn_core::java::JavaRequirement;
use cairn_core::launch::LaunchOptions;
use cairn_core::modpack::ExportOptions;
use cairn_core::resource::InstallReport;
use cairn_core::resource::curseforge as cfapi;
use cairn_core::resource::local::ResourceKind;
use cairn_core::resource::modrinth as mrapi;
use cairn_core::{Event, InstallRequest, InstallTarget, LaunchOutcome, Launcher, LauncherOptions, OptiFineChoice, TaskHandle};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cairn", version, about = "CairnLauncher 调试命令行")]
struct Cli {
    /// Data directory. Defaults to `CAIRN_HOME`, or a `.cairn` folder next to the exe.
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    /// Print detailed logs.
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Do not run script plugins, so their launch changes are skipped.
    #[arg(long, global = true)]
    no_plugins: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List installable Minecraft versions.
    Versions {
        #[arg(long)]
        snapshots: bool,
        /// Include old alpha and beta versions.
        #[arg(long)]
        old: bool,
        #[arg(long)]
        refresh: bool,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
    /// List the loader versions available for a Minecraft version.
    Loaders {
        /// fabric / quilt / forge / neoforge
        kind: String,
        mc: String,
        #[arg(long, default_value_t = 15)]
        limit: usize,
    },
    /// Install a version, optionally with a loader.
    Install {
        mc: String,
        /// Standalone instance name, an alternative to --root. Defaults to the MC version.
        #[arg(long)]
        name: Option<String>,
        /// Install into a registered standard .minecraft directory.
        #[arg(long)]
        root: Option<String>,
        /// Version folder name in the standard layout.
        #[arg(long)]
        version_name: Option<String>,
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        fabric: Option<String>,
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        quilt: Option<String>,
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        forge: Option<String>,
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        neoforge: Option<String>,
        /// Also install OptiFine. Optionally give a version such as HD_U_I6.
        /// Works with vanilla, Forge or Fabric.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        optifine: Option<String>,
    },
    /// Add OptiFine to an existing Forge or Fabric instance.
    AddOptifine {
        instance: String,
        #[arg(long)]
        version: Option<String>,
    },
    /// List all instances.
    Instances,
    /// Show or change an instance.
    Instance {
        id: String,
        /// Fixed maximum memory in MiB. 0 restores automatic memory.
        #[arg(long)]
        memory: Option<u32>,
        #[arg(long)]
        java: Option<PathBuf>,
        /// Standard layout: whether to enable version isolation.
        #[arg(long)]
        isolation: Option<bool>,
        #[arg(long)]
        server: Option<String>,
        /// Delete this instance. Also needs --yes.
        #[arg(long)]
        delete: bool,
        #[arg(long)]
        yes: bool,
    },
    /// Manage Java runtimes.
    Java {
        #[command(subcommand)]
        cmd: JavaCmd,
    },
    /// Manage accounts.
    Account {
        #[command(subcommand)]
        cmd: AccountCmd,
    },
    /// Launch an instance.
    Launch {
        id: String,
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        server: Option<String>,
        /// Only print the launch command.
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        demo: bool,
        /// Return right after launching, without waiting for the game to exit.
        #[arg(long)]
        no_wait: bool,
        /// Do not show game output in the terminal.
        #[arg(long)]
        quiet: bool,
        /// Smoke test: after the game window appears, wait this many seconds, then end the game.
        #[arg(long)]
        smoke: Option<u64>,
    },
    /// Manage standard .minecraft directories.
    Root {
        #[command(subcommand)]
        cmd: RootCmd,
    },
    /// Test the download speed of each source.
    Sources,
    /// List installed plugins, or create one from a template.
    Plugins {
        #[command(subcommand)]
        cmd: Option<PluginsCmd>,
    },
    /// Local resources: list, check dependencies, check updates.
    Res {
        instance: String,
        /// mod / resourcepack / shaderpack / save
        #[arg(long, default_value = "mod")]
        kind: String,
        /// Check for missing required dependencies.
        #[arg(long)]
        deps: bool,
        /// Check for updates through Modrinth.
        #[arg(long)]
        updates: bool,
        /// Apply updates right after checking.
        #[arg(long)]
        apply: bool,
    },
    /// Search Modrinth (default) or CurseForge.
    Search {
        query: String,
        #[arg(long)]
        cf: bool,
        /// mod / modpack / resourcepack / shader
        #[arg(long, default_value = "mod")]
        r#type: String,
        #[arg(long)]
        mc: Option<String>,
        #[arg(long)]
        loader: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: u32,
    },
    /// Install the latest compatible Modrinth version, with required dependencies.
    Add {
        instance: String,
        /// Modrinth project ID or slug.
        project: String,
        #[arg(long)]
        no_deps: bool,
    },
    /// Install the latest compatible CurseForge file of a mod, with required dependencies.
    AddCf {
        instance: String,
        mod_id: u64,
        #[arg(long)]
        file: Option<u64>,
        #[arg(long)]
        no_deps: bool,
    },
    /// Enable or disable a resource file.
    Toggle {
        path: PathBuf,
        #[arg(long)]
        disable: bool,
    },
    /// Import and export modpacks.
    Pack {
        #[command(subcommand)]
        cmd: PackCmd,
    },
    /// Show or change the config.
    Config {
        #[arg(long)]
        ms_client_id: Option<String>,
        #[arg(long)]
        curseforge_key: Option<String>,
        /// auto / prefer_official / prefer_mirror / official_only / mirror_only
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        concurrency: Option<usize>,
        /// Whether to follow the system proxy (true or false).
        #[arg(long)]
        proxy: Option<bool>,
    },
}

#[derive(Subcommand)]
enum JavaCmd {
    List {
        #[arg(long)]
        rescan: bool,
    },
    /// Download the official runtime for a Java major version.
    Download { major: u32 },
}

#[derive(Subcommand)]
enum AccountCmd {
    List,
    Offline {
        name: String,
    },
    /// Sign in with the system browser.
    Microsoft,
    /// Switch the current account. Give a name, an ID, or a unique part of either.
    #[command(alias = "select")]
    Use {
        account: String,
    },
    /// Switch to the next account, wrapping around.
    Next,
    /// Switch to the previous account, wrapping around.
    Prev,
    /// Bind an account to an instance. It is used first when the instance launches.
    /// Omit the account to unbind.
    Bind {
        instance: String,
        account: Option<String>,
    },
    Remove {
        id: String,
    },
}

#[derive(Subcommand)]
enum PackCmd {
    /// Read the modpack info.
    Info { path: PathBuf },
    /// Import as a new standalone instance.
    Import {
        path: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    /// Export as .mrpack.
    Export {
        instance: String,
        output: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "1.0.0")]
        version: String,
    },
}

#[derive(Subcommand)]
enum PluginsCmd {
    /// Create a working plugin in the plugins folder, with API types for editors.
    New {
        /// Plugin id: lowercase letters, digits, '.', '_' and '-'.
        id: String,
        #[arg(long)]
        name: Option<String>,
    },
    /// Write the current API type definitions (cairn.d.ts) into a plugin folder.
    Types { dir: PathBuf },
}

#[derive(Subcommand)]
enum RootCmd {
    List,
    Add { name: String, path: PathBuf },
    Remove { name: String },
}

fn fmt_bytes(b: u64) -> String {
    const U: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < 3 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}

/// Print events as they arrive. Progress refreshes one line in place.
/// Logs and game output print line by line.
fn spawn_printer(launcher: &Launcher, verbose: bool, show_game: bool) {
    let mut rx = launcher.subscribe();
    tokio::spawn(async move {
        let mut last_line = Instant::now() - Duration::from_secs(1);
        loop {
            let ev = match rx.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            };
            match ev {
                Event::TaskProgress { progress: p, .. } => {
                    if last_line.elapsed() < Duration::from_millis(150) {
                        continue;
                    }
                    last_line = Instant::now();
                    let mut line = format!("{:>5.1}% {}", p.fraction * 100.0, p.stage);
                    if p.files_total > 0 {
                        line.push_str(&format!("  {}/{} 个文件", p.files_done, p.files_total));
                    }
                    if p.bytes_total > 0 {
                        line.push_str(&format!("  {}/{}", fmt_bytes(p.bytes_done), fmt_bytes(p.bytes_total)));
                    }
                    if p.speed > 0 {
                        line.push_str(&format!("  {}/s", fmt_bytes(p.speed)));
                    }
                    eprint!("\r\x1b[2K{line}");
                    let _ = std::io::stderr().flush();
                }
                Event::TaskFinished { ok, error, .. } => {
                    eprint!("\r\x1b[2K");
                    if !ok && let Some(e) = error {
                        eprintln!("✗ {e}");
                    }
                }
                Event::Log { level, message } => {
                    if verbose || matches!(level, LogLevel::Warn | LogLevel::Error) {
                        eprintln!("\r\x1b[2K[{level:?}] {message}");
                    }
                }
                Event::GameStarted { pid, .. } => eprintln!("\r\x1b[2K▶ 游戏进程已启动(PID {pid})"),
                Event::GameWindowShown { .. } => eprintln!("▶ 游戏窗口已出现"),
                Event::GameOutput { line, .. } if show_game => println!("{line}"),
                Event::GameExited { code, crashed, .. } => {
                    eprintln!("■ 游戏已退出(退出码 {code:?}{})", if crashed { ",疑似崩溃" } else { "" })
                }
                _ => {}
            }
        }
    });
}

/// Wait for the task to finish. On Ctrl+C, cancel the task and wait for it to wrap up.
async fn run_task<T>(task: TaskHandle<T>) -> Result<T> {
    let token = task.cancel_token();
    let wait = task.wait();
    tokio::pin!(wait);
    let result = tokio::select! {
        r = &mut wait => r,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\n正在取消……");
            token.cancel();
            wait.await
        }
    };
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

fn main() -> Result<()> {
    // The command line starts itself with this flag to run script plugins in a separate process.
    if std::env::args().any(|arg| arg == cairn_plugin::host::HOST_FLAG) {
        cairn_plugin::host::run();
        return Ok(());
    }
    // The launcher needs little concurrency, so 4 worker threads are enough.
    // The default of one thread per logical CPU core wastes memory and scheduling overhead.
    tokio::runtime::Builder::new_multi_thread().worker_threads(4).max_blocking_threads(16).enable_all().build()?.block_on(run())
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let filter = if cli.verbose { "cairn_core=debug" } else { "cairn_core=warn" };
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).without_time().init();

    let launcher = Launcher::new(LauncherOptions { data_dir: cli.data_dir.clone() }).await.context("初始化失败")?;
    let show_game = !matches!(&cli.cmd, Cmd::Launch { quiet: true, .. });
    spawn_printer(&launcher, cli.verbose, show_game);

    match cli.cmd {
        Cmd::Versions { snapshots, old, refresh, limit } => {
            let m = launcher.version_manifest(refresh).await?;
            println!("最新正式版 {},最新快照 {}", m.latest.release, m.latest.snapshot);
            for v in m
                .versions
                .iter()
                .filter(|v| match v.kind {
                    VersionType::Release => true,
                    VersionType::Snapshot => snapshots,
                    VersionType::OldAlpha | VersionType::OldBeta => old,
                    VersionType::Other => snapshots,
                })
                .take(limit)
            {
                println!("{:<24} {:<10} {}", v.id, format!("{:?}", v.kind), v.release_time.get(..10).unwrap_or(""));
            }
        }
        Cmd::Loaders { kind, mc, limit } => {
            let kind = parse_loader_kind(&kind)?;
            for v in launcher.loader_versions(kind, &mc).await?.into_iter().take(limit) {
                println!(
                    "{:<28} {}{}{}",
                    v.version,
                    if v.stable { "稳定" } else { "测试" },
                    if v.recommended { " [推荐]" } else { "" },
                    v.requires.map(|r| format!("  (兼容 {r})")).unwrap_or_default()
                );
            }
        }
        Cmd::Install { mc, name, root, version_name, fabric, quilt, forge, neoforge, optifine } => {
            let opt = |s: Option<String>| s.map(|v| Some(v).filter(|v| !v.is_empty()));
            let loaders: Vec<LoaderSpec> = [
                opt(fabric).map(LoaderSpec::Fabric),
                opt(quilt).map(LoaderSpec::Quilt),
                opt(forge).map(LoaderSpec::Forge),
                opt(neoforge).map(LoaderSpec::NeoForge),
            ]
            .into_iter()
            .flatten()
            .collect();
            if loaders.len() > 1 {
                bail!("一次只能安装一个加载器");
            }
            let target = match root {
                Some(root) => InstallTarget::Standard { root, version_name },
                None => InstallTarget::Isolated { name: name.unwrap_or_else(|| mc.clone()) },
            };
            let started = Instant::now();
            let optifine = optifine.map(|v| OptiFineChoice { version: Some(v).filter(|v| !v.is_empty()) });
            let inst =
                run_task(launcher.install(InstallRequest { mc_version: mc, loader: loaders.into_iter().next(), optifine, fabric_api: true, target })).await?;
            println!("✓ 安装完成:{}(ID:{},耗时 {:.1}s)", inst.name, inst.id, started.elapsed().as_secs_f64());
        }
        Cmd::AddOptifine { instance, version } => {
            let path = run_task(launcher.add_optifine(&instance, version)).await?;
            println!("✓ 已放入 {}", path.display());
        }
        Cmd::Instances => {
            for i in launcher.instances().await {
                let loaders: Vec<String> = i.loaders.iter().map(|l| format!("{:?} {}", l.kind, l.version)).collect();
                println!(
                    "{:<36} {:<10} MC {:<12} {}{}",
                    i.id,
                    format!("{:?}", i.layout),
                    i.mc_version.as_deref().unwrap_or("?"),
                    loaders.join(", "),
                    i.problem.map(|p| format!("  ⚠ {p}")).unwrap_or_default()
                );
            }
        }
        Cmd::Instance { id, memory, java, isolation, server, delete, yes } => {
            if delete {
                if !yes {
                    bail!("删除实例会永久移除其文件,请加 --yes 确认");
                }
                launcher.delete_instance(&id).await?;
                println!("✓ 已删除 {id}");
                return Ok(());
            }
            let inst = launcher.instance(&id).await?;
            if memory.is_some() || java.is_some() || isolation.is_some() || server.is_some() {
                let mut s = inst.settings.clone();
                if let Some(m) = memory {
                    s.memory = (m > 0).then_some(MemoryPolicy::Fixed { mib: m });
                }
                if let Some(j) = java {
                    s.java_path = Some(j).filter(|p| !p.as_os_str().is_empty());
                }
                if let Some(iso) = isolation {
                    s.version_isolation = Some(iso);
                }
                if let Some(sv) = server {
                    s.server = Some(sv).filter(|v| !v.is_empty());
                }
                launcher.save_instance_settings(&id, s).await?;
                println!("✓ 已保存");
            }
            println!("{}", serde_json::to_string_pretty(&launcher.instance(&id).await?)?);
        }
        Cmd::Java { cmd } => match cmd {
            JavaCmd::List { rescan } => {
                let started = Instant::now();
                let list = launcher.java_list(rescan).await;
                for j in &list {
                    println!(
                        "Java {:<14} {:<5} {}{}{}",
                        j.version,
                        j.arch,
                        j.path.display(),
                        if j.is_jdk { "  [JDK]" } else { "" },
                        if j.managed { "  [自管]" } else { "" }
                    );
                }
                println!("共 {} 个,用时 {} ms", list.len(), started.elapsed().as_millis());
            }
            JavaCmd::Download { major } => {
                let req = JavaRequirement { min: Some((major, 0, 0)), max: Some((major + 1, 0, 0)), json_major: Some(major), component: None };
                let j = run_task(launcher.java_download(req)).await?;
                println!("✓ 已安装 Java {}:{}", j.version, j.path.display());
            }
        },
        Cmd::Account { cmd } => match cmd {
            AccountCmd::List => {
                for a in launcher.accounts().await {
                    println!("{} {:<20} {:<10} {} {}", if a.selected { "*" } else { " " }, a.name, format!("{:?}", a.kind), a.uuid, a.id);
                }
            }
            AccountCmd::Offline { name } => {
                let a = launcher.add_offline_account(&name).await?;
                println!("✓ 已添加离线账号 {}(UUID {})", a.name, a.uuid);
            }
            AccountCmd::Microsoft => {
                let login = async {
                    let login = launcher.begin_microsoft_login().await?;
                    eprintln!("使用 Microsoft 账号登录");
                    eprintln!("请在浏览器中打开以下网站，并输入设备代码完成登录。");
                    eprintln!("网址: {}", login.verification_uri());
                    eprintln!("设备代码: {}", login.user_code());
                    eprintln!("有效期: {} 秒。Ctrl+C 取消。", login.expires_in());
                    eprintln!("授权完成后，启动器将自动检测登录状态。");
                    if let Err(error) = login.open_browser() {
                        eprintln!("{error}");
                    }
                    login.finish().await
                };
                let a = tokio::select! {
                    result = login => result?,
                    _ = tokio::signal::ctrl_c() => bail!("登录已取消"),
                };
                println!("✓ 已登录微软账号 {}", a.name);
            }
            AccountCmd::Use { account } => {
                let a = launcher.switch_account(&account).await?;
                println!("✓ 当前账号:{}({:?})", a.name, a.kind);
            }
            AccountCmd::Next | AccountCmd::Prev => {
                let a = launcher.cycle_account(matches!(cmd, AccountCmd::Next)).await?;
                println!("✓ 当前账号:{}({:?})", a.name, a.kind);
            }
            AccountCmd::Bind { instance, account } => match launcher.bind_instance_account(&instance, account.as_deref()).await? {
                Some(a) => println!("✓ {instance} 将使用账号 {}", a.name),
                None => println!("✓ 已解除 {instance} 的账号绑定,使用全局当前账号"),
            },
            AccountCmd::Remove { id } => {
                launcher.remove_account(&id).await?;
                println!("✓ 已移除 {id}");
            }
        },
        Cmd::Plugins { cmd: None } => plugins::list(launcher.paths().root())?,
        Cmd::Plugins { cmd: Some(PluginsCmd::New { id, name }) } => {
            let dir = cairn_plugin::template::create(launcher.paths().root(), &id, name.as_deref().unwrap_or(&id)).map_err(anyhow::Error::msg)?;
            println!("已创建插件:{}", dir.display());
            println!("在启动器的「设置 → 插件」中启用它;修改文件后插件会自动重新加载。");
        }
        Cmd::Plugins { cmd: Some(PluginsCmd::Types { dir }) } => {
            println!("已写入 {}", cairn_plugin::template::write_types(&dir).map_err(anyhow::Error::msg)?.display());
        }
        Cmd::Launch { id, account, server, dry_run, demo, no_wait, smoke, .. } => {
            let plugins = if cli.no_plugins { None } else { plugins::Plugins::start(launcher.paths().root()) };
            if let Some(p) = &plugins {
                launcher.set_launch_hook(Some(p.clone()));
            }
            let opts = LaunchOptions { account_id: account, server, dry_run, demo, ..Default::default() };
            let mut events = launcher.subscribe();
            let started = Instant::now();
            let outcome = run_task(launcher.launch(&id, opts)).await;
            // Launch handlers are no longer needed once the launch is decided.
            if let Some(p) = plugins {
                launcher.set_launch_hook(None);
                tokio::task::block_in_place(|| p.stop());
            }
            match outcome? {
                LaunchOutcome::Planned { plan, command } => {
                    eprintln!("Java {}  堆 {} MiB  目录 {}", plan.java.version, plan.heap_mib, plan.game_dir.display());
                    println!("{command}");
                }
                LaunchOutcome::Started { game } => {
                    eprintln!("准备耗时 {} ms", started.elapsed().as_millis());
                    if let Some(secs) = smoke {
                        let (l, gid) = (launcher.clone(), game.id);
                        tokio::spawn(async move {
                            let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
                            loop {
                                match tokio::time::timeout_at(deadline, events.recv()).await {
                                    Ok(Ok(Event::GameWindowShown { game })) if game == gid => break,
                                    Ok(Ok(_)) | Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                                    _ => {
                                        eprintln!("冒烟测试失败:3 分钟内未出现游戏窗口");
                                        l.kill_game(gid);
                                        return;
                                    }
                                }
                            }
                            tokio::time::sleep(Duration::from_secs(secs)).await;
                            eprintln!("冒烟测试通过:窗口已出现 {secs} 秒,结束游戏");
                            l.kill_game(gid);
                        });
                    }
                    if !no_wait && let Some(g) = launcher.game(game.id) {
                        let exit = g.wait().await;
                        // Give the last few output lines time to print.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        if exit.crashed {
                            bail!("游戏异常退出(退出码 {:?},运行 {} 秒)", exit.code, exit.run_secs);
                        }
                    }
                }
            }
        }
        Cmd::Root { cmd } => match cmd {
            RootCmd::List => {
                for r in launcher.config().game_roots {
                    println!("{:<16} {}", r.name, r.path.display());
                }
            }
            RootCmd::Add { name, path } => {
                launcher.add_game_root(&name, &path).await?;
                println!("✓ 已登记 {name} → {}", path.display());
            }
            RootCmd::Remove { name } => {
                launcher.remove_game_root(&name).await?;
                println!("✓ 已移除 {name}");
            }
        },
        Cmd::Res { instance, kind, deps, updates, apply } => {
            let kind = match kind.as_str() {
                "mod" => ResourceKind::Mod,
                "resourcepack" => ResourceKind::ResourcePack,
                "shaderpack" => ResourceKind::ShaderPack,
                "save" => ResourceKind::Save,
                other => bail!("未知的资源类型 {other}"),
            };
            if deps {
                let missing = launcher.missing_dependencies(&instance).await?;
                if missing.is_empty() {
                    println!("✓ 必需依赖齐全");
                }
                for m in missing {
                    println!("✗ {}({})缺少依赖 {} {}", m.mod_name, m.file_name, m.dependency, m.version_range.unwrap_or_default());
                }
            } else if updates {
                let list = launcher.check_mod_updates(&instance).await?;
                for u in &list {
                    println!("{:<40} {} → {}", u.file_name, u.current_version, u.new_version);
                }
                println!("共 {} 个可更新", list.len());
                if apply && !list.is_empty() {
                    let n = run_task(launcher.apply_mod_updates(list)).await?;
                    println!("✓ 已更新 {n} 个");
                }
            } else {
                let started = Instant::now();
                let list = launcher.local_resources(&instance, kind).await?;
                for r in &list {
                    let meta = r.meta.as_ref();
                    println!(
                        "{} {:<44} {:<24} {}",
                        if r.enabled { "●" } else { "○" },
                        r.file_name,
                        meta.and_then(|m| m.id.clone()).unwrap_or_default(),
                        meta.and_then(|m| m.version.clone()).unwrap_or_default()
                    );
                }
                println!("共 {} 个,用时 {} ms", list.len(), started.elapsed().as_millis());
            }
        }
        Cmd::Search { query, cf, r#type, mc, loader, limit } => {
            if cf {
                let class_id = match r#type.as_str() {
                    "modpack" => cfapi::class::MODPACKS,
                    "resourcepack" => cfapi::class::RESOURCE_PACKS,
                    "shader" => cfapi::class::SHADERS,
                    _ => cfapi::class::MODS,
                };
                let q = cfapi::SearchQuery {
                    query,
                    class_id: Some(class_id),
                    mc_version: mc,
                    loader: loader.as_deref().map(parse_loader_kind).transpose()?,
                    page_size: limit,
                    ..Default::default()
                };
                let r = launcher.curseforge_search(&q).await?;
                for m in r.data {
                    println!("{:<10} {:<36} {:>10.0} 下载  {}", m.id, m.name, m.download_count, m.summary.chars().take(40).collect::<String>());
                }
            } else {
                let q = mrapi::SearchQuery {
                    query,
                    project_type: Some(r#type),
                    mc_version: mc,
                    loaders: loader.into_iter().collect(),
                    limit,
                    ..Default::default()
                };
                let r = launcher.modrinth_search(&q).await?;
                for h in r.hits {
                    println!("{:<24} {:<36} {:>10} 下载  {}", h.slug, h.title, h.downloads, h.description.chars().take(40).collect::<String>());
                }
                println!("共 {} 个结果", r.total_hits);
            }
        }
        Cmd::Add { instance, project, no_deps } => {
            let versions = launcher.modrinth_versions(&project, Some(&instance)).await?;
            let v = versions.first().with_context(|| format!("{project} 没有适用于该实例的版本"))?;
            println!("选择版本 {}({})", v.version_number, v.name);
            let report = run_task(launcher.modrinth_install(&instance, &v.id, !no_deps)).await?;
            print_report(&report);
        }
        Cmd::AddCf { instance, mod_id, file, no_deps } => {
            let file_id = match file {
                Some(f) => f,
                None => launcher.curseforge_files(mod_id, Some(&instance)).await?.first().map(|f| f.id).context("没有适用于该实例的文件")?,
            };
            let report = run_task(launcher.curseforge_install(&instance, file_id, !no_deps)).await?;
            print_report(&report);
        }
        Cmd::Toggle { path, disable } => {
            let p = launcher.set_resource_enabled(&path, !disable).await?;
            println!("✓ {}", p.display());
        }
        Cmd::Pack { cmd } => match cmd {
            PackCmd::Info { path } => println!("{}", serde_json::to_string_pretty(&launcher.inspect_modpack(&path).await?)?),
            PackCmd::Import { path, name } => {
                let r = run_task(launcher.import_modpack(path, name)).await?;
                println!("✓ 已导入为实例 {}({:?},{} 个文件)", r.instance.id, r.format, r.files);
                for m in r.manual {
                    println!("  需手动下载:{}({})→ 放入 {}\n    {}", m.name, m.file_name, m.target_dir.display(), m.url);
                }
            }
            PackCmd::Export { instance, output, name, version } => {
                let opts = ExportOptions { name, version, summary: None, include: Vec::new(), output };
                let out = run_task(launcher.export_mrpack(&instance, opts)).await?;
                println!("✓ 已导出 {}", out.display());
            }
        },
        Cmd::Sources => {
            for s in launcher.test_sources().await {
                println!("{:<10} {:<8} 延迟 {}", s.name, format!("{:?}", s.kind), s.latency_ms.map(|v| format!("{v} ms")).unwrap_or_else(|| "不可用".into()));
            }
        }
        Cmd::Config { ms_client_id, curseforge_key, source, concurrency, proxy } => {
            let strategy = match source.as_deref() {
                None => None,
                Some(s) => Some(serde_json::from_value(serde_json::Value::String(s.to_string())).context("未知的下载源策略")?),
            };
            let cfg = launcher
                .update_config(|c| {
                    if let Some(id) = ms_client_id {
                        c.credentials.ms_client_id = Some(id).filter(|s| !s.is_empty());
                    }
                    if let Some(k) = curseforge_key {
                        c.credentials.curseforge_api_key = Some(k).filter(|s| !s.is_empty());
                    }
                    if let Some(s) = strategy {
                        c.download.strategy = s;
                    }
                    if let Some(n) = concurrency {
                        c.download.max_concurrency = n;
                    }
                    if let Some(p) = proxy {
                        c.download.follow_system_proxy = p;
                    }
                })
                .await?;
            let mut shown = serde_json::to_value(&cfg)?;
            // Do not echo keys to the terminal.
            if let Some(cred) = shown.get_mut("credentials") {
                for k in ["ms_client_id", "curseforge_api_key"] {
                    if cred.get(k).is_some_and(|v| !v.is_null()) {
                        cred[k] = serde_json::Value::String("(已设置)".into());
                    }
                }
            }
            println!("{}", serde_json::to_string_pretty(&shown)?);
            println!("配置文件:{}", launcher.paths().config_file().display());
        }
    }
    Ok(())
}

fn print_report(r: &InstallReport) {
    for f in &r.installed {
        println!("✓ 已安装 {f}");
    }
    for f in &r.already_present {
        println!("· 已存在 {f}");
    }
    for m in &r.manual {
        println!("! 需手动下载 {}({})→ 放入 {}\n    {}", m.name, m.file_name, m.target_dir.display(), m.url);
    }
}

fn parse_loader_kind(s: &str) -> Result<LoaderKind> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "fabric" => LoaderKind::Fabric,
        "quilt" => LoaderKind::Quilt,
        "forge" => LoaderKind::Forge,
        "neoforge" | "neo" => LoaderKind::NeoForge,
        "optifine" | "of" => LoaderKind::OptiFine,
        _ => bail!("未知的加载器 {s}(可选 fabric / quilt / forge / neoforge / optifine)"),
    })
}
