//! `modlight-server`: port of `server.ts` (serve) and `migrate.ts` (migrate).
//!
//! ```text
//! modlight-server [serve]   migrate the database, then serve (what the
//!                           upstream container did: migrate.js, then server.js)
//! modlight-server migrate   only migrate + seed, like `pnpm db:migrate`
//! ```
//!
//! Both load `.env` from the working directory first without overriding
//! variables that are already set (`import "dotenv/config"`). Logs go to
//! stderr; `RUST_LOG` filters them (default `info`).

use std::process::ExitCode;

use modlight_server::config::{Config, env_bool};
use modlight_server::db;
use modlight_server::http::{self, App};
use modlight_server::paths::Paths;
use modlight_server::runtime::limits::{cgroup_cpu_limit, cgroup_memory_limit, effective_cpu_count};
use modlight_server::state::AppState;

const USAGE: &str = "usage: modlight-server [serve|migrate]";

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn,hyper=warn"));
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr());
    tracing_subscriber::fmt().with_env_filter(filter).with_ansi(ansi).with_writer(std::io::stderr).init();
}

fn main() -> ExitCode {
    // `.env` first, then logging, exactly once.
    let _ = dotenvy::dotenv();
    init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("serve");
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("could not start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    let result = match command {
        "serve" => runtime.block_on(serve()),
        "migrate" => runtime.block_on(migrate()),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("unknown command {other:?}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!("{err:#}");
            ExitCode::FAILURE
        }
    }
}

/// `migrate.ts`: DATABASE_URL as given (no production checks: migrate.ts never
/// loaded lib/config), PG* variables when it is unset, one connection.
async fn migrate() -> anyhow::Result<()> {
    let url = std::env::var("DATABASE_URL").unwrap_or_default();
    let registration = env_bool(std::env::var("REGISTRATION_ENABLED").ok().as_deref(), false);
    let paths = Paths::resolve();
    let pool = db::connect(&url, 1).await?;
    db::migrate::run(&pool, &paths.migrations, registration).await?;
    pool.close().await;
    Ok(())
}

async fn serve() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let paths = Paths::resolve();

    // Ensure runtime directories exist (server.ts).
    let mut runtime_dirs = vec![config.uploads.tus_tmp_path.clone(), config.uploads.tmp_path.clone()];
    if config.storage.driver == "local" {
        runtime_dirs.push(config.storage.local_path.clone());
    }
    for dir in &runtime_dirs {
        std::fs::create_dir_all(dir).map_err(|e| anyhow::anyhow!("could not create {dir}: {e}"))?;
    }

    // The upstream container ran migrate.js before the server.
    {
        let pool = db::connect(&config.db.url, 1).await?;
        db::migrate::run(&pool, &paths.migrations, config.auth.registration_enabled).await?;
        pool.close().await;
    }

    let pool = db::connect_lazy(&config.db.url)?;
    let state = AppState::new(config, paths, pool)?;
    let addr = http::listen_addr(&state)?;
    let mut jobs = modlight_server::jobs::start(&state);

    let listener =
        tokio::net::TcpListener::bind(addr).await.map_err(|e| anyhow::anyhow!("could not listen on {addr}: {e}"))?;
    let mode = if state.config.is_prod { "prod" } else { "dev" };
    tracing::info!("> {} ready on http://{addr} ({mode})", state.config.app_name);
    if !state.shell.is_built() {
        tracing::warn!("web/dist/index.html not found: pages are served with a placeholder shell");
    }
    report_runtime_limits();

    http::serve(listener, App::new(state), shutdown_signal()).await?;
    jobs.abort_all();
    tracing::info!("stopped");
    Ok(())
}

/// Say, once, what this process thinks it is allowed to use (server.ts
/// `reportRuntimeLimits`, without the V8 and libuv lines that do not apply).
fn report_runtime_limits() {
    let cpu = cgroup_cpu_limit();
    let mem = cgroup_memory_limit();
    if cpu.is_none() && mem.is_none() {
        return; // not containerised; nothing to report
    }
    let cpu = cpu.map(|c| c.to_string()).unwrap_or_else(|| "unlimited".into());
    let mem = mem.map(|m| format!("{} MB", (m / (1024.0 * 1024.0)).round())).unwrap_or_else(|| "unlimited".into());
    tracing::info!(
        "> limits: cpu {cpu} core(s), memory {mem}, worker threads sized for {} core(s)",
        effective_cpu_count()
    );
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutting down");
}
