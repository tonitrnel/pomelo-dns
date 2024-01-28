pub mod default;
pub mod doh;
pub mod dot;
mod http;

use crate::resolves::doh::DoH;
pub use default::Default;
pub use dot::DoT;
use std::borrow::Cow;

pub trait DNSResolver {
    async fn resolve(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>>;
}

pub async fn resolve(server: &str, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    if server.starts_with("tls://") {
        let (_, addr, port) = split_addr(server);
        let addr = format!("{}:{}", addr, port.unwrap_or("853"));
        let mut dns = DoT::new(&addr)?;
        dns.resolve(bytes).await
    } else if server.starts_with("https://") {
        let addr = if server.ends_with("/dns-query") {
            Cow::Borrowed(server)
        } else {
            Cow::Owned(format!("{}/dns-query", server))
        };
        let mut dns = DoH::new(&addr)?;
        dns.resolve(bytes).await
    } else {
        let (_, addr, port) = split_addr(server);
        let addr = format!("{}:{}", addr, port.unwrap_or("53"));
        let mut dns = Default::new(&addr);
        dns.resolve(bytes).await
    }
}

fn split_addr(input: &str) -> (Option<&str>, &str, Option<&str>) {
    let mut parts = input.split("://").peekable();
    let (protocol, rest) = match parts.next() {
        Some(p) if parts.peek().is_some() => (Some(p), parts.next().unwrap()),
        _ => (None, input),
    };

    let mut rest_parts = rest.split(':');
    let addr = rest_parts.next().unwrap();
    let port = rest_parts.next();

    (protocol, addr, port)
}
