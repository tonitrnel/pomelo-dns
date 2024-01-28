use crate::resolves::dot::{build_tcp_stream, make_tls_config, wrap_tls_stream};
use crate::resolves::http;
use crate::resolves::DNSResolver;
use anyhow::Context;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::TlsConnector;
use url::Url;

pub struct DoH {
    target: Url,
    tls_config: Arc<ClientConfig>,
}

impl DoH {
    pub fn new(target: &str) -> anyhow::Result<Self> {
        Ok(Self {
            target: Url::parse(target)?,
            tls_config: make_tls_config(),
        })
    }
    pub async fn build_connect(&self) -> anyhow::Result<TlsStream<TcpStream>> {
        wrap_tls_stream(
            build_tcp_stream(&self.target).await?,
            &self.target,
            &TlsConnector::from(self.tls_config.clone()),
        )
        .await
    }
}
impl DNSResolver for DoH {
    async fn resolve(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let mut stream = self.build_connect().await?;
        let req = http::h1::Request::new()
            .path(self.target.path())
            .header("content-type", "application/dns-message")
            .header(
                "host",
                self.target.host_str().with_context(|| "Missing host")?,
            )
            .header("content-length", &bytes.len().to_string())
            .body(bytes)
            .as_bytes();
        stream.write_all(&req).await?;
        stream.flush().await?;

        let response = http::h1::Response::from_stream(&mut stream).await?;
        if response.status_code != 200 {
            anyhow::bail!("{}", response.status_text)
        }
        Ok(response.body)
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
        let mut dns = DoH::new("https://1.1.1.1/dns-query").unwrap();
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
