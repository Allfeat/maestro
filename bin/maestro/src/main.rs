//! Maestro - Substrate blockchain indexer.
//!
//! # Usage
//!
//! ```bash
//! # Start with default config
//! maestro
//!
//! # Start with environment overrides
//! DATABASE_URL=postgres://localhost/maestro WS_URL=ws://localhost:9944 maestro
//! ```

use anyhow::Result;
use clap::Parser;

mod bootstrap;
mod cli;
mod purge;
mod run;
mod setup;
mod tui;

use bootstrap::Bootstrap;
use cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    // --export-schema is a pure type-only operation — no DB, no Substrate,
    // no tracing setup needed.
    if cli.export_schema {
        setup::print_schema_sdl();
        return Ok(());
    }

    let bootstrap = Bootstrap::init(cli);
    let Some(services) = setup::setup_services(&bootstrap).await? else {
        return Ok(()); // --migrate-only / --purge already handled
    };

    run::run_services(bootstrap, services).await
}
