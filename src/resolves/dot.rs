use crate::resolves::DNSResolver;
use anyhow::Context;
use std::collections::{HashMap, VecDeque};
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::OnceCell;
use tokio::time::Instant;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, KeyLogFile, RootCertStore};
use tokio_rustls::{client::TlsStream, TlsConnector};
use url::Url;

type Stream = TlsStream<TcpStream>;

static LIVE_STREAMS: OnceCell<Arc<Mutex<HashMap<Url, VecDeque<Stream>>>>> = OnceCell::const_new();

pub struct DoT {
    target: Url,
    tls_config: Arc<ClientConfig>,
}

pub fn make_tls_config() -> Arc<ClientConfig> {
    let mut root_cert_store = RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();
    // To change the path, see the environment variable: 'SSLKEYLOGFILE'
    config.key_log = Arc::new(KeyLogFile::new());
    Arc::new(config)
}
pub async fn build_tcp_stream(target: &Url) -> anyhow::Result<TcpStream> {
    let host = target
        .host_str()
        .with_context(|| "Missing host in the URL")?;
    let port = target
        .port_or_known_default()
        .with_context(|| format!("Missing port for host: {}", target))?;
    let addr = (host, port)
        .to_socket_addrs()
        .with_context(|| {
            format!(
                "Failed to parse host: {}, port: {} to socket address",
                host, port
            )
        })?
        .next()
        .with_context(|| {
            format!(
                "No socket addresses found for host: {}, port: {}",
                host, port
            )
        })?;
    let stream = TcpStream::connect(addr)
        .await
        .with_context(|| "TCP connect failed")?;
    Ok(stream)
}

pub async fn wrap_tls_stream(
    stream: TcpStream,
    target: &Url,
    connector: &TlsConnector,
) -> anyhow::Result<TlsStream<TcpStream>> {
    let server_name = ServerName::try_from(
        target
            .domain()
            .map_or_else(|| target.host_str(), Some)
            .with_context(|| format!("Failed to parse domain and host, url: '{}'", target))?
            .to_string(),
    )
    .with_context(|| "Invalid dns name")?;
    let stream = connector
        .connect(server_name, stream)
        .await
        .with_context(|| "Failed to wrap tls stream")?;
    Ok(stream)
}

impl DoT {
    pub fn new(target: &str) -> anyhow::Result<Self> {
        Ok(Self {
            target: Url::parse(target)?,
            tls_config: make_tls_config(),
        })
    }
    pub async fn build_connect(&self) -> anyhow::Result<Stream> {
        wrap_tls_stream(
            build_tcp_stream(&self.target).await?,
            &self.target,
            &TlsConnector::from(self.tls_config.clone()),
        )
        .await
    }
    async fn live_streams_guard<'a>(
    ) -> anyhow::Result<MutexGuard<'a, HashMap<Url, VecDeque<Stream>>>> {
        LIVE_STREAMS
            .get_or_init(|| async { Arc::new(Mutex::new(HashMap::new())) })
            .await
            .lock()
            .map_err(|err| anyhow::format_err!("lock poll failed, reason {:?}", err))
    }
    async fn take(&self) -> anyhow::Result<(bool, Stream)> {
        let stream = {
            let mut guard = Self::live_streams_guard().await?;
            if let Some(pool) = guard.get_mut(&self.target) {
                pool.pop_front()
            } else {
                None
            }
        };
        if let Some(stream) = stream {
            Ok((true, stream))
        } else {
            // println!("创建新的连接");
            Ok((false, self.build_connect().await?))
        }
    }
    async fn enqueue(&self, stream: Stream) -> anyhow::Result<()> {
        let mut guard = Self::live_streams_guard().await?;
        if let Some(pool) = guard.get_mut(&self.target) {
            pool.push_back(stream);
        } else {
            let mut pool = VecDeque::new();
            pool.push_back(stream);
            guard.insert(self.target.clone(), pool);
        }
        Ok(())
    }
}

impl DNSResolver for DoT {
    async fn resolve(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let (reuse, mut stream) = self.take().await?;
        let mut retry_count = 0;
        let mut buf = [0; 2];
        let start = Instant::now();
        loop {
            if !reuse {
                break;
            }
            // 尝试写入，如果失败，丢弃当前 stream 重新创建一个新的 stream
            if retry_count > 3 {
                anyhow::bail!(
                    "Unexpected error, no available stream found, retry {}",
                    retry_count
                )
            }
            let r = tokio::time::timeout(Duration::from_millis(100), stream.read(&mut buf)).await;
            match r {
                Ok(Ok(0)) => {
                    let (reuse, new_stream) = self.take().await?;
                    stream = new_stream;
                    if !reuse {
                        break;
                    }

                    retry_count += 1;
                    continue;
                }
                _ => break,
            }
        }
        println!("start write: {}ms", start.elapsed().as_millis());
        stream
            .write_all(&(bytes.len() as u16).to_be_bytes())
            .await?;
        stream.write_all(bytes).await?;
        stream.flush().await?;

        println!("start read: {}ms", start.elapsed().as_millis());
        let mut len_bytes = vec![0; 2];
        stream
            .read_exact(&mut len_bytes)
            .await
            .map_err(|_| -> ! { std::process::exit(1) })?;
        // .with_context(|| "Failed to read response data")?;
        let len = u16::from_be_bytes([len_bytes[0], len_bytes[1]]) as usize;
        let mut response = vec![0; len];
        stream.read_exact(&mut response).await?;
        if response.is_empty() {
            anyhow::bail!("Unexpected error, response should not be empty")
        }
        // let (stream, _) = stream.into_inner();
        self.enqueue(stream).await?;
        println!("query upstream server: {}ms", start.elapsed().as_millis());
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::op::Message;
    use hickory_proto::rr::RecordType;
    use hickory_proto::serialize::binary::BinDecodable;
    #[tokio::test]
    async fn it_works() {
        let mut dns = DoT::new("tls://1.1.1.1:853").unwrap();
        // query example.com
        let bytes = [
            0x00, 0x02, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x65,
            0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x03, 0x63, 0x6f, 0x6d, 0x00, 0x00, 0x01, 0x00,
            0x01,
        ];
        let response = dns.resolve(&bytes).await.unwrap();
        let message = Message::from_bytes(&response).unwrap();
        assert!(!message.answers().is_empty());
        assert_eq!(message.answers()[0].name().to_utf8(), "example.com.");
        assert_eq!(message.answers()[0].record_type(), RecordType::A);
    }
}
