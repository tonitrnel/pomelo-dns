mod cache;
mod config;
mod handler;
mod logs;
mod ping;
mod resolves;
mod server;

use crate::config::Config;
use crate::logs::registry_logs;
use anyhow::Context;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, UdpSocket};

pub const MAX_CONNECTIONS: usize = 1024;

fn print_banner() {
    tracing::info!("");
    tracing::info!(r#"     _____                     _         _____  _   _  _____  "#);
    tracing::info!(r#"    |  __ \                   | |       |  __ \| \ | |/ ____| "#);
    tracing::info!(r#"    | |__) |__  _ __ ___   ___| | ___   | |  | |  \| | (___   "#);
    tracing::info!(r#"    |  ___/ _ \| '_ ` _ \ / _ \ |/ _ \  | |  | | . ` |\___ \  "#);
    tracing::info!(r#"    | |  | (_) | | | | | |  __/ | (_) | | |__| | |\  |____) | "#);
    tracing::info!(r#"    |_|   \___/|_| |_| |_|\___|_|\___/  |_____/|_| \_|_____/  "#);
    tracing::info!("");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = env::args().nth(2).unwrap_or("pomelo.conf".to_string());
    let config =
        Arc::new(Config::new(PathBuf::from(path)).with_context(|| "Failed to load config file")?);
    let (udp, tcp) = {
        let guard = config.access().await;
        registry_logs(&guard.log);
        let udp = UdpSocket::bind(&guard.metadata.bind)
            .await
            .with_context(|| format!("could not bind to udp: {}", &guard.metadata.bind))?;
        let tcp = TcpListener::bind(&guard.metadata.bind)
            .await
            .with_context(|| format!("could not bind to tcp: {}", &guard.metadata.bind))?;
        (udp, tcp)
    };
    print_banner();
    tracing::info!("The DNS Server running: ");
    tracing::info!(
        "udp://{}",
        udp.local_addr()
            .with_context(|| "could not lookup local address")?,
    );
    tracing::info!(
        "tcp://{}",
        tcp.local_addr()
            .with_context(|| "could not lookup local address")?
    );
    tracing::info!("awaiting connections...");
    match server::run_until_done(config, (tcp, udp)).await {
        Ok(()) => {
            tracing::info!("PomeloDNS stopping");
            Ok(())
        }
        Err(err) => {
            tracing::error!("PomeloDNS has encountered an error: {}", err);
            Err(err)
        }
    }
}
