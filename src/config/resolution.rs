use crate::config::{parse_key_value_pair, Inner, DEFAULT_GROUP};
use crate::ping::ping_with_timeout;
use hickory_proto::rr::Name;
use lru::LruCache;
use maxminddb::{geoip2, Reader};
use std::collections::HashMap;
use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;

static PING_CACHE: OnceCell<Arc<Mutex<LruCache<IpAddr, bool>>>> = OnceCell::const_new();

#[derive(Debug)]
pub enum ResolutionDirective {
    Allow,
    Deny,
    Pingable,
    Country(String),
}

#[derive(Debug)]
pub enum ResolutionPayload {
    Domain(String),
    All,
}

impl FromStr for ResolutionPayload {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "ALL" {
            Ok(ResolutionPayload::All)
        } else {
            Ok(ResolutionPayload::Domain(s.to_string()))
        }
    }
}

#[derive(Debug)]
pub struct Resolution {
    directive: ResolutionDirective,
    payload: ResolutionPayload,
}

pub type GroupResolutionMappings = HashMap<String, Vec<Resolution>>;

impl Resolution {
    pub fn payload_match(&self, domain: &Name) -> bool {
        match &self.payload {
            ResolutionPayload::All => true,
            ResolutionPayload::Domain(it) => Name::from_ascii(it)
                .map(|it| it.eq_case(domain))
                .unwrap_or(false),
        }
    }
    pub async fn ping_cache<'a>() -> &'a Arc<Mutex<LruCache<IpAddr, bool>>> {
        PING_CACHE
            .get_or_init(|| async {
                Arc::new(Mutex::new(LruCache::new(NonZeroUsize::new(455).unwrap())))
            })
            .await
    }
    pub async fn check_is_allow<'input>(&self, args: CheckArgs<'input>) -> bool {
        match &self.directive {
            ResolutionDirective::Allow => true,
            ResolutionDirective::Deny => false,
            ResolutionDirective::Pingable => {
                {
                    let mut guard = Self::ping_cache().await.lock().unwrap();
                    if guard.contains(args.addr) {
                        return *guard.get(args.addr).unwrap();
                    }
                }
                let r = ping_with_timeout(*args.addr, Duration::from_millis(600))
                    .await
                    .is_ok();
                {
                    Self::ping_cache()
                        .await
                        .lock()
                        .map(|mut it| it.put(*args.addr, r))
                        .unwrap_or_default();
                }
                r
            }
            ResolutionDirective::Country(country) => {
                if let Ok(Some(iso_code)) = args
                    .mmdb
                    .unwrap()
                    .lookup::<geoip2::Country>(*args.addr)
                    .map(|it| it.country.and_then(|it| it.iso_code))
                {
                    country == iso_code
                } else {
                    false
                }
            }
        }
    }
}

pub struct CheckArgs<'input> {
    pub(crate) addr: &'input IpAddr,
    pub(crate) mmdb: Option<&'input Reader<Vec<u8>>>,
}

impl FromStr for Resolution {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (directive, payload) = if let Some(end) = s.strip_prefix("@allow:") {
            (ResolutionDirective::Allow, end)
        } else if let Some(end) = s.strip_prefix("@deny:") {
            (ResolutionDirective::Deny, end)
        } else if let Some(end) = s.strip_prefix("@pingable:") {
            (ResolutionDirective::Pingable, end)
        } else if let Some(end) = s.strip_prefix("@country:") {
            let parts = end.split('/').collect::<Vec<_>>();
            if parts.len() != 2 {
                anyhow::bail!("Country directive invalid: '{}'", s)
            }
            (ResolutionDirective::Country(parts[0].to_string()), parts[1])
        } else {
            anyhow::bail!("Invalid directive: '{}'", s);
        };
        Ok(Self {
            directive,
            payload: ResolutionPayload::from_str(payload)?,
        })
    }
}

pub fn ipv6_resolution_parse(row: usize, line: &str, inner: &mut Inner) -> anyhow::Result<()> {
    let (key, value, col) = parse_key_value_pair(line)
        .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;
    if key != DEFAULT_GROUP && !inner.groups.contains_key(&key) {
        anyhow::bail!("Can't find group '{}' definition in {line}:1", key);
    }
    inner.ipv6_resolution.insert(
        key,
        value
            .split(',')
            .map(|it| match Resolution::from_str(it.trim()) {
                r @ Ok(Resolution {
                    directive: ResolutionDirective::Country(_),
                    ..
                }) => {
                    if inner.metadata.mmdb.is_some() {
                        r
                    } else {
                        anyhow::bail!(
                            "mmdb not found, unable to use 'country' command in line {}:{}",
                            row,
                            col
                        )
                    }
                }
                r => r,
            })
            .collect::<Result<Vec<Resolution>, anyhow::Error>>()?,
    );
    Ok(())
}
