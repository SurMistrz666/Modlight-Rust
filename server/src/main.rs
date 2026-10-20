//! `modlight-server`: port of `server.ts` (serve) and `migrate.ts` (migrate).
//!
//! ```text
//! modlight-server [serve]   migrate the database, then serve (what the
//!                           upstream container did: migrate.js, then server.js)
//! modlight-server migrate   only migrate + seed, like `pnpm db:migrate`
//! modlight-server health    GET /api/health on this container's own port;
//!                           exit 0 on a 2xx (the compose healthcheck, which
//!                           upstream ran with `node -e fetch(...)`)
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

const USAGE: &str = "usage: modlight-server [serve|migrate|health]";

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
    if command == "health" {
        return health();
    }
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

/// The container healthcheck: `fetch('http://127.0.0.1:3000/api/health')`
/// with `r.ok`, on the port this process would listen on (`PORT`, default
/// 3000 as in the image). No runtime, no TLS: one HTTP/1.1 request.
fn health() -> ExitCode {
    use std::io::{Read, Write};
    use std::time::Duration;
    let port = std::env::var("PORT").ok().and_then(|p| p.trim().parse::<u16>().ok()).unwrap_or(3000);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let probe = || -> std::io::Result<bool> {
        let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        write!(stream, "GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")?;
        let mut head = [0u8; 16];
        let mut read = 0;
        while read < head.len() {
            let n = stream.read(&mut head[read..])?;
            if n == 0 {
                break;
            }
            read += n;
        }
        // "HTTP/1.1 200 OK": the status code is bytes 9..12.
        let status = std::str::from_utf8(&head[..read])
            .ok()
            .and_then(|line| line.get(9..12))
            .and_then(|code| code.parse::<u16>().ok());
        Ok(matches!(status, Some(200..=299)))
    };
    match probe() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(err) => {
            eprintln!("health: {err}");
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
    let cpu_text = cpu.map(|c| c.to_string()).unwrap_or_else(|| "unlimited".into());
    let mb = |b: f64| (b / (1024.0 * 1024.0)).round();
    let mem_text = mem.map(|m| format!("{} MB", mb(m))).unwrap_or_else(|| "unlimited".into());
    // Upstream also printed the V8 heap and the libuv pool, which a Rust
    // process does not have; the libvips thread count is the same setting.
    tracing::info!(
        "> limits: cpu {cpu_text} core(s), memory {mem_text}, libvips threads {}, worker threads sized for {} core(s)",
        *modlight_server::media::sharp_runtime::SHARP_CONCURRENCY,
        effective_cpu_count()
    );
    let budget = modlight_server::media::transform_cost::CONTAINER_MEMORY_BYTES as f64;
    if let Some(m) = mem
        && m != budget
    {
        tracing::warn!(
            "> warning: the transform budget is sized for {} MB but this container has {} MB. Update CONTAINER_MEMORY_BYTES in src/media/transform_cost.rs.",
            mb(budget),
            mb(m)
        );
    }
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
