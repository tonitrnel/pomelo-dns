use anyhow::Context;
use std::collections::HashMap;
use std::fmt::{Display, Formatter};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

pub const PROTOCOL: &str = "HTTP";
pub const VERSION: &str = "1.1";
#[derive(Debug, Clone)]
pub struct Response {
    pub status_code: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}
enum State {
    Protocol,
    Version,
    StatusCode,
    StatusText,
    HeaderName,
    HeaderValue,
}
impl Response {
    pub async fn from_stream(stream: &mut TlsStream<TcpStream>) -> anyhow::Result<Self> {
        let mut byte = [0];
        let mut state = State::Protocol;
        let mut protocol = String::new();
        let mut version = String::new();
        let mut status_code = String::new();
        let mut status_text = String::new();
        let mut header_name = String::new();
        let mut header_value = String::new();
        let mut headers = HashMap::new();
        while let Some(char) = next(stream, &mut byte).await {
            state = match state {
                State::Protocol => match char {
                    'A'..='Z' => {
                        protocol.push(char);
                        State::Protocol
                    }
                    '/' => {
                        if protocol != PROTOCOL {
                            anyhow::bail!("Not supported protocol: {}", protocol)
                        }
                        State::Version
                    }
                    _ => anyhow::bail!("Invalid protocol char '{}'", char),
                },
                State::Version => match char {
                    '0'..='9' | '.' => {
                        version.push(char);
                        State::Version
                    }
                    ' ' => {
                        if version != VERSION {
                            anyhow::bail!("Not supported version: {}", version)
                        }
                        State::StatusCode
                    }
                    _ => anyhow::bail!("Invalid protocol version char '{}'", char),
                },
                State::StatusCode => match char {
                    '0'..='9' => {
                        status_code.push(char);
                        State::StatusCode
                    }
                    ' ' => State::StatusText,
                    _ => anyhow::bail!("Invalid status code char '{}'", char),
                },
                State::StatusText => match char {
                    '\r' => {
                        next(stream, &mut byte).await;
                        State::HeaderName
                    }
                    _ => {
                        status_text.push(char);
                        State::StatusText
                    }
                },
                State::HeaderName => match char {
                    '\r' => {
                        next(stream, &mut byte).await;
                        break;
                    }
                    ':' => State::HeaderValue,
                    _ => {
                        header_name.push(char);
                        State::HeaderName
                    }
                },
                State::HeaderValue => match char {
                    '\r' => {
                        next(stream, &mut byte).await;
                        headers.insert(header_name.to_lowercase(), header_value.trim().to_string());
                        header_name.clear();
                        header_value.clear();
                        State::HeaderName
                    }
                    _ => {
                        header_value.push(char);
                        State::HeaderValue
                    }
                },
            }
        }
        let length = headers
            .get("content-length")
            .and_then(|it| it.parse::<u16>().ok())
            .with_context(|| "Unknown body length")?;
        let mut body = vec![0; length as usize];
        stream.read_exact(&mut body).await?;
        Ok(Self {
            status_code: status_code
                .parse::<u16>()
                .with_context(|| format!("Invalid status code: {}", status_code))?,
            status_text,
            headers,
            body,
        })
    }
}
async fn next(stream: &mut TlsStream<TcpStream>, byte: &mut [u8; 1]) -> Option<char> {
    stream.read_exact(byte).await.ok().map(|_| byte[0] as char)
}

pub struct Request<'input> {
    method: RequestMethod,
    headers: HashMap<&'input str, &'input str>,
    path: &'input str,
    body: Option<&'input [u8]>,
}
#[derive(Debug)]
pub enum RequestMethod {
    Option,
    Get,
    Post,
    Put,
    Delete,
    Head,
    Trace,
    Connect,
    Patch,
}
impl Display for RequestMethod {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestMethod::Option => f.write_str("OPTION"),
            RequestMethod::Get => f.write_str("GET"),
            RequestMethod::Post => f.write_str("POST"),
            RequestMethod::Put => f.write_str("PUT"),
            RequestMethod::Delete => f.write_str("DELETE"),
            RequestMethod::Head => f.write_str("HEAD"),
            RequestMethod::Trace => f.write_str("TRACE"),
            RequestMethod::Connect => f.write_str("CONNECT"),
            RequestMethod::Patch => f.write_str("PATCH"),
        }
    }
}
impl<'input> Request<'input> {
    pub fn new() -> Self {
        Self {
            path: "",
            method: RequestMethod::Get,
            headers: HashMap::from([("accept", "*/*")]),
            body: None,
        }
    }
    pub fn method(&mut self, method: RequestMethod) -> &mut Self {
        self.method = method;
        self
    }
    pub fn path(&mut self, path: &'input str) -> &mut Self {
        self.path = path;
        self
    }
    pub fn header(&mut self, name: &'input str, value: &'input str) -> &mut Self {
        self.headers.insert(name.as_ref(), value.trim());
        self
    }
    pub fn body(&mut self, bytes: &'input [u8]) -> &mut Self {
        self.body = Some(bytes);
        self
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut req = Vec::new();
        req.extend_from_slice(
            format!(
                "{method} {path} {PROTOCOL}/{VERSION}\r\n",
                method = self.method,
                path = self.path
            )
            .as_bytes(),
        );
        for header in &self.headers {
            req.extend_from_slice(format!("{}: {}\r\n", header.0, header.1).as_bytes());
        }
        req.extend_from_slice("\r\n".as_bytes());
        if let Some(body) = &self.body {
            req.extend_from_slice(body);
        }
        req
    }
}
