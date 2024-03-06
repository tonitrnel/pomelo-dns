use crate::cache::Cache;
use crate::config::Config;
use crate::resolves::resolve;
use anyhow::Context;
use hickory_proto::op::{Message, MessageType, Query};
use hickory_proto::rr::{rdata, Name, RData, Record, RecordType};
use hickory_proto::serialize::binary::BinDecodable;
use std::fmt::Write;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

pub struct Handler {
    pub addr: SocketAddr,
    pub cache: Arc<Cache>,
    pub config: Arc<Config>,
    pub timeout: Duration,
    pub group: String,
    pub start: Instant,
    pub protocol: &'static str,
}

impl Handler {
    pub fn new(
        protocol: &'static str,
        addr: SocketAddr,
        group: String,
        cache: Arc<Cache>,
        config: Arc<Config>,
    ) -> Self {
        Self {
            addr,
            cache,
            timeout: Duration::from_secs(60),
            group,
            config,
            start: Instant::now(),
            protocol,
        }
    }

    #[tracing::instrument(name = "Dns Query", skip(self, bytes, send_ret))]
    pub async fn run<F, Fut>(&mut self, bytes: Vec<u8>, send_ret: F)
    where
        F: FnOnce(Vec<u8>, SocketAddr) -> Fut,
        Fut: Future<Output = anyhow::Result<()>>,
    {
        if let Err(err) = self.handle(bytes, send_ret).await {
            tracing::error!("{}", format_err(err, 34));
        }
    }
    async fn handle<F, Fut>(&mut self, bytes: Vec<u8>, send_ret: F) -> anyhow::Result<()>
    where
        F: FnOnce(Vec<u8>, SocketAddr) -> Fut,
        Fut: Future<Output = anyhow::Result<()>>,
    {
        let req =
            Message::from_bytes(&bytes).with_context(|| "Failed to parse message from bytes")?;
        tracing::trace!(
            "----[IP: {protocol}://{addr}]#{id:0>5} [GROUP: {group}]------------------------------------------",
            protocol = self.protocol,
            addr = self.addr.ip(),
            id = req.id(),
            group = self.group,
        );
        tracing::trace!("[->](Q) Queries: {}", format_queries(req.queries()));
        if let Some(res) = Self::print_err_and_flatten(
            self.resolve_from_hosts(&req)
                .await
                .with_context(|| "Failed to resolve from hosts"),
        ) {
            self.print_dns_query_detail('L', &req, &res);
            send_ret(
                res.to_vec()
                    .with_context(|| "Failed to convert response to vec")?,
                self.addr,
            )
            .await
            .with_context(|| "Failed to send response")?;
            return Ok(());
        }
        if let Some(res) = Self::print_err_and_flatten(
            self.lookup_dns_cache(&req)
                .with_context(|| "Failed to lookup DNS cache"),
        ) {
            self.print_dns_query_detail('C', &req, &res);
            send_ret(
                res.to_vec()
                    .with_context(|| "Failed to convert cached response to vec")?,
                self.addr,
            )
            .await
            .with_context(|| "Failed to send cached response")?;
            return Ok(());
        }
        let res = self
            .forward_dns_query(&bytes)
            .await
            .with_context(|| "Failed to forward DNS query")?;
        let mut res = Message::from_bytes(&res)
            .with_context(|| "Failed to parse forwarded response from bytes")?;
        if req
            .queries()
            .iter()
            .any(|it| matches!(it.query_type(), RecordType::AAAA))
        {
            self.resolution(&mut res)
                .await
                .with_context(|| "Failed during AAAA record resolution")?;
        }
        self.cache_dns_record(&res)
            .with_context(|| "Failed to cache DNS record")?;
        self.print_dns_query_detail('F', &req, &res);
        send_ret(
            res.to_vec()
                .with_context(|| "Failed to convert final response to vec")?,
            self.addr,
        )
        .await
        .with_context(|| "Failed to send final response")?;
        Ok(())
    }
    const PTR_IPV4_SUFFIX: &'static str = ".in-addr.arpa.";
    const PTR_IPV6_SUFFIX: &'static str = ".ip6.arpa.";
    async fn resolve_from_hosts(&self, req: &Message) -> anyhow::Result<Option<Message>> {
        let mut answers = Vec::<Record>::new();
        for query in req.queries() {
            let name = query.name().to_utf8();
            match query.query_type() {
                RecordType::PTR => {
                    if let Err(err) = self.local_reverse_dns_query(&name, query, &mut answers) {
                        tracing::warn!("{}", format_err(err, 41))
                    };
                }
                RecordType::A => {
                    let addrs = self
                        .config
                        .access()
                        .get_hosts(&self.group, &name)
                        .into_iter()
                        .filter_map(|it| match it {
                            IpAddr::V4(addr) => Some(addr),
                            IpAddr::V6(_) => None,
                        })
                        .collect::<Vec<_>>();
                    if addrs.is_empty() {
                        continue;
                    };
                    let name = Name::from_ascii(name)?;
                    answers.extend(addrs.into_iter().map(|it| {
                        Record::new()
                            .set_name(name.clone())
                            .set_record_type(RecordType::A)
                            .set_ttl(1)
                            .set_data(Some(RData::A(rdata::A(it))))
                            .to_owned()
                    }))
                }
                RecordType::AAAA => {
                    let addrs = self
                        .config
                        .access()
                        .get_hosts(&self.group, &name)
                        .into_iter()
                        .filter_map(|it| match it {
                            IpAddr::V6(addr) => Some(addr),
                            IpAddr::V4(_) => None,
                        })
                        .collect::<Vec<_>>();
                    if addrs.is_empty() {
                        continue;
                    };
                    let name = Name::from_ascii(name)?;
                    answers.extend(addrs.into_iter().map(|it| {
                        Record::new()
                            .set_name(name.clone())
                            .set_record_type(RecordType::AAAA)
                            .set_ttl(1)
                            .set_data(Some(RData::AAAA(rdata::AAAA(it))))
                            .to_owned()
                    }))
                }
                _ => continue,
            }
        }
        if answers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(
                req.to_owned()
                    .set_message_type(MessageType::Response)
                    .add_answers(answers)
                    .to_owned(),
            ))
        }
    }
    fn local_reverse_dns_query(
        &self,
        name: &str,
        query: &Query,
        answers: &mut Vec<Record>,
    ) -> anyhow::Result<()> {
        // 反查 Ip addr
        let addr = if let Some(addr) = name.strip_suffix(Self::PTR_IPV4_SUFFIX) {
            let parts = addr
                .split('.')
                .map(|it| {
                    it.parse::<u8>().with_context(|| {
                        format!("Failed to parse IPv4 address part: {} name: {}", it, name)
                    })
                })
                .rev()
                .collect::<Result<Vec<_>, _>>()?;
            IpAddr::from(Ipv4Addr::new(parts[0], parts[1], parts[2], parts[3]))
        } else if let Some(addr) = name.strip_suffix(Self::PTR_IPV6_SUFFIX) {
            let parts = addr
                .split('.')
                .rev()
                .collect::<Vec<_>>()
                .chunks(4)
                .map(|it| {
                    u16::from_str_radix(&it.join(""), 16).with_context(|| {
                        format!("Failed to parse IPv6 address part: {:?} name: {}", it, name)
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            IpAddr::from(Ipv6Addr::new(
                parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6], parts[7],
            ))
        } else {
            return Ok(());
        };
        if let Some(hostname) = self.config.access().get_hostname(&self.group, addr) {
            answers.push(
                Record::new()
                    .set_name(query.name().to_owned())
                    .set_record_type(RecordType::PTR)
                    .set_data(Some(RData::PTR(rdata::PTR(Name::from_ascii(hostname)?))))
                    .to_owned(),
            );
        }
        Ok(())
    }
    fn lookup_dns_cache(&mut self, req: &Message) -> anyhow::Result<Option<Message>> {
        let mut guard = match self.cache.access()? {
            Some(guard) => guard,
            None => return Ok(None),
        };
        let mut answers = Vec::new();
        for query in req.queries() {
            let name = query.name().to_utf8();
            let qtype = query.query_type();
            match qtype {
                RecordType::A | RecordType::AAAA => {
                    if let Some(records) = guard.get(&name, qtype) {
                        answers.extend(records)
                    };
                }
                _ => continue,
            }
        }
        if answers.is_empty() {
            Ok(None)
        } else {
            Ok(Some(
                req.to_owned()
                    .set_message_type(MessageType::Response)
                    .add_answers(answers)
                    .to_owned(),
            ))
        }
    }
    async fn forward_dns_query(&mut self, bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
        let config = self.config.access();
        let server = config.get_server(&self.group);
        let now = Instant::now();
        tokio::select! {
            res = resolve(&server[0], bytes) =>  {
                Ok(res?)
            },
            _ = tokio::time::sleep(self.timeout) => {
                anyhow::bail!("query upstream server timeout, elapsed {}ms", now.elapsed().as_millis())
            }
        }
    }
    async fn resolution(&self, message: &mut Message) -> anyhow::Result<()> {
        let answers = message.answers_mut();
        let mut tasks = Vec::new();
        let config = self.config.access();
        for answer in answers.iter() {
            if let Some(RData::AAAA(rdata::AAAA(addr))) = answer.data() {
                let domain = answer.name().clone();
                let group = self.group.clone();
                let addr = IpAddr::from(addr.to_owned());
                let config = config.clone();
                tasks.push(tokio::spawn(async move {
                    config.is_allow_ipv6(&group, &domain, addr).await
                }));
            } else {
                tasks.push(tokio::spawn(async move { true }))
            }
        }
        let allows = futures::future::join_all(tasks)
            .await
            .into_iter()
            .map(|it| it.unwrap_or(false))
            .collect::<Vec<_>>();
        *answers = answers
            .drain(..)
            .enumerate()
            .filter_map(
                |(idx, record)| {
                    if allows[idx] {
                        Some(record)
                    } else {
                        None
                    }
                },
            )
            .collect();
        Ok(())
    }
    fn cache_dns_record(&self, _message: &Message) -> anyhow::Result<()> {
        if !self.cache.enabled() {
            return Ok(());
        }
        todo!();
    }
    fn print_dns_query_detail(&self, stage: char, _req: &Message, res: &Message) {
        let indent = " ".repeat(41);
        tracing::trace!(
            "[<-:{}ms]({stage}) Answers: {}",
            self.start.elapsed().as_millis(),
            format_answers(&indent, res.answers())
        );
    }
    fn print_err_and_flatten<T>(input: Result<Option<T>, anyhow::Error>) -> Option<T> {
        input.unwrap_or_else(|err| {
            tracing::error!("{}", format_err(err, 41));
            None
        })
    }
}

pub(crate) fn format_err(err: anyhow::Error, indent: usize) -> String {
    let ind = " ".repeat(indent);
    format!(
        "{}\n{}",
        err,
        err.chain()
            .skip(1)
            .fold(String::new(), |mut output, it| {
                let _ = writeln!(output, "{ind}Caused by: {it}");
                output
            })
            .trim_end_matches('\n')
    )
}

fn format_queries(queries: &[Query]) -> String {
    queries
        .iter()
        .map(|it| {
            format!(
                "{}: type {}, class {}",
                it.name(),
                it.query_type(),
                it.query_class()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_answers(indent: &str, answers: &[Record]) -> String {
    let items = answers
        .iter()
        .map(|it| {
            format!(
                "{}: type {}, class {}{}",
                it.name(),
                it.record_type(),
                it.dns_class(),
                match it.data() {
                    Some(RData::AAAA(addr)) => format!(", addr {}", addr),
                    Some(RData::A(addr)) => format!(", addr {}", addr),
                    Some(RData::CNAME(cname)) => format!(", cname {}", cname),
                    Some(RData::SOA(mname)) => format!(", mname {}", mname),
                    Some(RData::PTR(ptr)) => format!(", {}", ptr),
                    _ => "".to_string(),
                }
            )
        })
        .collect::<Vec<_>>();
    if items.len() == 1 {
        items.join("; ")
    } else if items.is_empty() {
        "<None>".to_string()
    } else {
        let tab = " ".repeat(4);
        items
            .into_iter()
            .fold(String::from("\n"), |mut output, it| {
                let _ = writeln!(output, "{indent}{tab}{it};");
                output
            })
            .trim_end_matches('\n')
            .to_string()
    }
}
