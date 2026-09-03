mod cli;
mod config;
mod db;
mod handlers;
mod http;
mod init;
mod manager;
mod models;

// Use mimalloc for musl builds (musl's default malloc is very slow).
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::sync::Arc;

use clap::Parser;

use cli::Commands;
use handlers::{Consts, Ctx};
use manager::Manager;

#[tokio::main]
async fn main() {
    init::logger();

    let cli = cli::Cli::parse();

    // DB path from --db flag.
    let db_path = cli.db_path.to_string_lossy().to_string();

    // Handle CLI flags.
    if let Some(cmd) = cli.command {
        match cmd {
            // Generate a new config file.
            Commands::NewConfig { path } => {
                match config::generate_sample(&path) {
                    Ok(_) => {
                        log::info!("config file generated: {}", path.display());
                    }
                    Err(e) => {
                        log::error!("error generating config: {}", e);
                        std::process::exit(1);
                    }
                }
                return;
            }

            // Create a new SQLite database with schema.
            Commands::Install { yes } => {
                if cli.db_path.exists() {
                    log::error!("database '{}' already exists", cli.db_path.display());
                    std::process::exit(1);
                }
                if let Err(e) = db::install_schema(&db_path, !yes).await {
                    log::error!("error installing schema: {}", e);
                    std::process::exit(1);
                }
                return;
            }
        }
    }

    // For server mode, DB must exist.
    db::exists(&cli.db_path);

    // Load config.
    let config = config::load_all(&cli.config);

    // Create database pool.
    let db = match db::init(&db_path, config.db.max_conns, false).await {
        Ok(pool) => pool,
        Err(e) => {
            log::error!("error connecting to database: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize manager.
    let mgr = Arc::new(Manager::new(db));

    // Setup the global app context used in HTTP handlers.
    let ctx = Arc::new(Ctx {
        mgr,

        // Global constants populated from config.
        consts: Consts {
            root_url: config.app.root_url,

            api_default_per_page: config.api_results.per_page,
            api_max_per_page: config.api_results.max_per_page,
        },
    });

    // Start the HTTP server.
    let routes = http::init_handlers(ctx);
    let addr = config.app.address;

    log::info!("starting server on {}", addr);

    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("error listening on {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, routes).await {
        log::error!("server error: {}", e);
        std::process::exit(1);
    }
}
