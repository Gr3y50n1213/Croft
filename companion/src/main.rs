// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! `croft-companion`: see `croft_companion::cli::USAGE`.

use croft_companion::cli::{self, CliError, Command};
use croft_companion::config::Config;
use croft_companion::{api, http};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{}", out.trim_end());
            }
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("croft-companion: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<String, CliError> {
    let (config_path, command) = cli::parse(args)?;
    let config = Config::load(config_path.as_deref())?;
    if command != Command::Serve {
        return cli::run(&config, &command);
    }
    // `serve` below; `healthcheck` needs no database (handled in `cli::run`).
    // Held while the server runs: a master key rotation waits for it to stop.
    let _lock = cli::serve_lock(&config)?;
    let (store, key) = cli::open(&config)?;
    let auth = api::Auth::new(
        &config.public_url,
        key,
        config.trusted_proxies.clone(),
        api::Auth::system_clock(),
    )
    .map_err(|e| CliError::Serve(e.to_string()))?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| CliError::Serve(e.to_string()))?;
    runtime
        .block_on(http::serve(&config.listen, http::App::new(store, auth)))
        .map_err(|e| CliError::Serve(e.to_string()))?;
    Ok(String::new())
}
