mod cache;
mod config;
mod handler;
mod logs;
mod pidfile;
mod ping;
mod resolves;
mod server;

use crate::config::Config;
use crate::logs::registry_logs;
use crate::pidfile::Pidfile;
use crate::server::ServerArgs;
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
    let _pid = Pidfile::new()?;
    let path = env::args()
        .nth(1)
        .unwrap_or("/etc/pomelo/pomelo.conf".to_string());
    let config =
        Arc::new(Config::new(PathBuf::from(path)).with_context(|| "Failed to load config file")?);
    let (mut log_writer, log_handle) = logs::LogWriter::new()?;
    let (udp, tcp) = {
        let config = config.access();
        registry_logs(&mut log_writer, config.metadata.access_log)?;
        let udp = UdpSocket::bind(&config.metadata.bind)
            .await
            .with_context(|| format!("could not bind to udp: {}", &config.metadata.bind))?;
        let tcp = TcpListener::bind(&config.metadata.bind)
            .await
            .with_context(|| format!("could not bind to tcp: {}", &config.metadata.bind))?;
        (udp, tcp)
    };
    print_banner();
    tracing::info!(
        "Pomelo {version} ({commit_id} {build_date}) built with docker{docker_version}, {system_version}, rustc{rustc_version}",
        build_date = env!("BUILD_DATE"),
        version = env!("CARGO_PKG_VERSION"),
        commit_id = env!("COMMIT_ID"),
        docker_version = env!("DOCKER_VERSION"),
        rustc_version = env!("RUSTC_VERSION"),
        system_version = env!("SYSTEM_VERSION"),
    );
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
    match server::run_until_done(
        ServerArgs {
            config,
            logs: Arc::new(log_writer),
        },
        (tcp, udp),
    )
    .await
    {
        Ok(()) => {
            println!("Pomelo stopping...");
        }
        Err(err) => {
            eprintln!("Pomelo has encountered an error: {}", err);
            return Err(err);
        }
    }
    match log_handle.await {
        Ok(result) => result?,
        Err(err) if err.is_panic() => {
            panic!("{}", err)
        }
        _ => (),
    };
    Ok(())
}
