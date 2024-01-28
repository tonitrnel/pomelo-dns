use crate::cache::Cache;
use crate::config::Config;
use crate::handler::Handler;
use crate::MAX_CONNECTIONS;
use futures_util::FutureExt;
use std::net::SocketAddr;
use std::sync::{Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    signal,
    sync::Semaphore,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

const MAX_UDP_PACKET_SIZE: usize = 4096;

trait ServerTrait {
    async fn run(&mut self) -> anyhow::Result<()>;
    async fn reply(&mut self, bytes: &[u8]) -> anyhow::Result<()>;
}

pub struct UdpServer {
    socket: Arc<UdpSocket>,
    limit_connections: Arc<Semaphore>,
    shutdown_signal: CancellationToken,
    shared_buf: [u8; MAX_UDP_PACKET_SIZE],
    cache: Arc<Cache>,
    config: Arc<Config>,
}

impl UdpServer {
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut join_set = JoinSet::new();
        loop {
            let permit = self.limit_connections.clone().acquire_owned().await?;
            let shutdown_signal = self.shutdown_signal.clone();
            let (req, addr) = tokio::select! {
                v = self.accept()  => match v{
                    Ok(v) => v,
                    Err(err) => {
                        tracing::error!("{:?}", err);
                        continue;
                    }
                },
                _ = shutdown_signal.cancelled() => break,
            };
            let bytes = req.to_vec();
            let group = self.config.access().await.attribute_group(&addr.ip());
            let mut handler =
                Handler::new("udp", addr, group, self.cache.clone(), self.config.clone());
            let socket = self.socket.clone();
            join_set.spawn(async move {
                let ret = |bytes: Vec<u8>, addr| async move {
                    socket.send_to(&bytes, addr).await?;
                    Ok(())
                };
                handler.run(bytes, ret).await;
                drop(permit)
            });
            while FutureExt::now_or_never(join_set.join_next())
                .flatten()
                .is_some()
            {}
        }
        if self.shutdown_signal.is_cancelled() {
            Ok(())
        } else {
            anyhow::bail!("Unexpected close of UDP socket")
        }
    }
    pub async fn accept(&mut self) -> anyhow::Result<(&[u8], SocketAddr)> {
        let (len, addr) = self.socket.recv_from(&mut self.shared_buf).await?;
        Ok((&self.shared_buf[..len], addr))
    }
}

pub struct TcpServer {
    socket: Arc<TcpListener>,
    limit_connections: Arc<Semaphore>,
    shutdown_signal: CancellationToken,
    cache: Arc<Cache>,
    config: Arc<Config>,
}

impl TcpServer {
    pub async fn run(&mut self) -> anyhow::Result<()> {
        let mut join_set = JoinSet::new();
        loop {
            let permit = self.limit_connections.clone().acquire_owned().await?;
            let shutdown_signal = self.shutdown_signal.clone();
            let (mut stream, req, addr) = tokio::select! {
                v = self.accept()  => match v{
                    Ok(v) => v,
                    Err(err) => {
                        tracing::error!("{:?}", err);
                        continue;
                    }
                },
                _ = shutdown_signal.cancelled() => break,
            };
            let bytes = req.to_vec();
            let group = self.config.access().await.attribute_group(&addr.ip());
            let mut handler =
                Handler::new("tcp", addr, group, self.cache.clone(), self.config.clone());
            join_set.spawn(async move {
                let ret = |bytes: Vec<u8>, _addr| async move {
                    let len_bytes = (bytes.len() as u16).to_be_bytes();
                    stream.write_all(&len_bytes).await?;
                    stream.write_all(&bytes).await?;
                    Ok(())
                };
                handler.run(bytes, ret).await;
                drop(permit);
            });
            while FutureExt::now_or_never(join_set.join_next())
                .flatten()
                .is_some()
            {}
        }
        if self.shutdown_signal.is_cancelled() {
            Ok(())
        } else {
            anyhow::bail!("Unexpected close of TCP connection")
        }
    }
    pub async fn accept(&mut self) -> anyhow::Result<(TcpStream, Vec<u8>, SocketAddr)> {
        let (mut stream, addr) = self.socket.accept().await?;
        let mut len_bytes = [0; 2];
        stream.read_exact(&mut len_bytes).await?;
        let len = u16::from_be_bytes(len_bytes) as usize;
        let mut buf = vec![0; len];
        stream.read_exact(&mut buf).await?;
        Ok((stream, buf, addr))
    }
}

pub async fn run_until_done(
    config: Arc<Config>,
    binds: (TcpListener, UdpSocket),
) -> anyhow::Result<()> {
    let cache = Arc::new(Cache::with_capacity(
        config.access().await.metadata.cache_size,
    ));
    let mut join_set = JoinSet::new();
    let shutdown_signal = CancellationToken::new();
    let limit_connections = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    // register udp
    {
        let mut udp_server = UdpServer {
            socket: Arc::new(binds.1),
            limit_connections: limit_connections.clone(),
            shutdown_signal: shutdown_signal.clone(),
            shared_buf: [0; MAX_UDP_PACKET_SIZE],
            config: config.clone(),
            cache: cache.clone(),
        };
        join_set.spawn(async move { udp_server.run().await });
    }
    // register tcp
    {
        let mut tcp_server = TcpServer {
            socket: Arc::new(binds.0),
            limit_connections: limit_connections.clone(),
            shutdown_signal: shutdown_signal.clone(),
            config: config.clone(),
            cache: cache.clone(),
        };
        join_set.spawn(async move { tcp_server.run().await });
    }
    // register ctrl+c
    {
        let shutdown_signal = shutdown_signal.clone();
        join_set.spawn(async move {
            let _ = signal::ctrl_c().await;
            shutdown_signal.cancel();
            Ok(())
        });
    }

    while let Some(r) = join_set.join_next().await {
        match r {
            Ok(Ok(_)) => (),
            Ok(Err(e)) => return Err(e),
            Err(e) => anyhow::bail!("Internal error in spawn: {e}"),
        }
    }
    Ok(())
}
