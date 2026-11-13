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
    let auth = match &config.oidc {
        Some(settings) => {
            let secret = croft_companion::config::load_secret_file(&settings.client_secret_file)?;
            auth.with_oidc(croft_companion::oidc::Provider::new(
                settings.clone(),
                secret,
                &config.public_url,
            ))
        }
        None => auth,
    };
    // T26: the Companion's own network is off limits to mail connections, except the networks
    // the admin allows (a mail server on the same LAN or Docker network).
    croft_protocols::netpolicy::set_address_policy(croft_companion::mail::address_policy(
        config.mail_allowed_networks.clone(),
    ));
    let runtime = tokio::runtime::Runtime::new().map_err(|e| CliError::Serve(e.to_string()))?;
    runtime
        .block_on(async {
            let cores = croft_companion::mail::MailCores::new(config.data_dir.clone());
            // T34e: mail folders of users deleted while their folder could not be removed.
            if let Ok(ids) = store.user_ids() {
                let removed = cores.sweep(&ids);
                if removed > 0 {
                    eprintln!("removed the mail of {removed} deleted user(s)");
                }
            }
            let mut app = http::App::new(store, auth).with_mail(cores);
            // T30: the web app, read once from its folder.
            if let Some(dir) = &config.web_dir {
                let web = croft_companion::webapp::WebApp::load(dir, &config.public_url)
                    .map_err(std::io::Error::other)?;
                app = app.with_web(web);
            }
            // The cores of users who have mail accounts start syncing at once.
            let opening = app.clone();
            tokio::task::spawn_blocking(move || opening.open_all_mail());
            http::serve(&config.listen, app).await
        })
        .map_err(|e| CliError::Serve(e.to_string()))?;
    Ok(String::new())
}
