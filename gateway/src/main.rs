// This Source Code Form is subject to the terms of the GNU Affero General Public License,
// version 3 or later. See LICENSE-AGPL-3.0 at the repository root.

//! `croft-push-gateway [serve]`, `croft-push-gateway generate-key`, `croft-push-gateway healthcheck`.

use std::io::{Read, Write};
use std::process::ExitCode;

use croft_push_gateway::config::Config;
use croft_push_gateway::fcm::{Fcm, ServiceAccount};
use croft_push_gateway::seal::GatewayKey;
use croft_push_gateway::server;

fn main() -> ExitCode {
    let command = std::env::args().nth(1).unwrap_or_else(|| "serve".into());
    let result = match command.as_str() {
        "serve" => serve(),
        "generate-key" => GatewayKey::generate_hex()
            .map(|k| println!("{}", k.as_str()))
            .map_err(|e| e.to_string()),
        "healthcheck" => healthcheck(),
        _ => Err("usage: croft-push-gateway [serve | generate-key | healthcheck]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("croft-push-gateway: {e}");
            ExitCode::FAILURE
        }
    }
}

fn config() -> Result<Config, String> {
    let toml = match std::env::var("CROFT_GATEWAY_CONFIG") {
        Ok(path) => Some(
            std::fs::read_to_string(&path)
                .map_err(|_| "the configuration file (CROFT_GATEWAY_CONFIG) cannot be read")?,
        ),
        Err(_) => None,
    };
    Config::load(toml.as_deref(), std::env::vars()).map_err(|e| e.to_string())
}

fn serve() -> Result<(), String> {
    let config = config()?;
    let key = std::fs::read_to_string(&config.key_file)
        .map_err(|_| "the key file cannot be read (make one with `generate-key`)".to_string())
        .and_then(|t| {
            GatewayKey::from_hex(&zeroize::Zeroizing::new(t)).map_err(|e| e.to_string())
        })?;
    let account = std::fs::read(&config.service_account_file)
        .map_err(|_| "the service account file cannot be read".to_string())
        .and_then(|b| {
            ServiceAccount::from_json(&zeroize::Zeroizing::new(b)).map_err(|e| e.to_string())
        })?;
    let fcm = Fcm::new(account, &config.fcm_base);
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let listener = tokio::net::TcpListener::bind(&config.listen)
            .await
            .map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
        let app = server::App::new(&config, key, fcm);
        tokio::spawn(server::report(app.counts.clone()));
        eprintln!("croft-push-gateway: listening on {}", config.listen);
        server::serve_on(listener, app, shutdown_signal())
            .await
            .map_err(|e| e.to_string())
    })
}

/// `GET /health` on this machine's listener (the Docker health check).
fn healthcheck() -> Result<(), String> {
    let config = config()?;
    let port = config
        .listen
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .ok_or("no port in listen")?;
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(4)))
        .map_err(|e| e.to_string())?;
    s.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut answer = String::new();
    let _ = s.take(4096).read_to_string(&mut answer);
    if answer.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err("unhealthy".into())
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = term => {},
    }
}
