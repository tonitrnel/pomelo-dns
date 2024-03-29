mod group;
mod hosts;
mod metadata;
mod resolution;
mod server;

use anyhow::Context;
use hickory_proto::rr::domain::Name;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;


static DEFAULT_GROUP: &str = "default";

#[derive(Debug)]
pub struct Inner {
    groups: group::Groups,
    servers: server::Servers,
    hosts: hosts::GroupHostMappings,
    pub metadata: metadata::Metadata,
    ipv6_resolution: resolution::GroupResolutionMappings,
}

impl Inner {
    pub fn load(path: &PathBuf) -> anyhow::Result<(Self, HashSet<PathBuf>)> {
        let path = if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir().unwrap().join(path)
        };
        let mut fs = fs::OpenOptions::new()
            .read(true)
            .open(&path)
            .with_context(|| format!("Config file \"{:?}\" not exists.", path))?;
        let mut text = String::new();
        let mut watch_paths = HashSet::new();
        fs.read_to_string(&mut text)
            .with_context(|| format!("Unable to read config file \"{:?}\".", path))?;
        watch_paths.insert(path);
        Ok((Inner::parse(&text, &mut watch_paths)?, watch_paths))
    }
    fn parse(str: &str, watch_paths: &mut HashSet<PathBuf>) -> anyhow::Result<Self> {
        let mut config = Inner {
            groups: HashMap::new(),
            servers: HashMap::new(),
            hosts: HashMap::new(),
            metadata: metadata::Metadata::default(),
            ipv6_resolution: HashMap::new(),
        };
        let lines = str.lines();
        let mut section: Option<Section> = None;
        let mut row = 0;
        for line in lines {
            let line = line.trim_end();
            row += 1;
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                let section_name = line.trim_matches(|c| c == '[' || c == ']');
                section = Some(parse_section(section_name));
                continue;
            }
            if let Some(section) = &section {
                match section {
                    Section::Unknown(section) => {
                        anyhow::bail!("Unknown section '{}'", section);
                    }
                    Section::Group => group::parse(row, line, &mut config.groups)?,
                    Section::Server => server::parse(row, line, &mut config)?,
                    Section::Host(sub) => hosts::parse(sub, row, line, &mut config, watch_paths)?,
                    Section::Metadata => metadata::parse(row, line, &mut config)?,
                    Section::IPv6Resolution => {
                        resolution::ipv6_resolution_parse(row, line, &mut config)?
                    }
                }
            } else {
                anyhow::bail!("Unexpected error, missing section {}", line)
            }
        }
        if !config.servers.contains_key(DEFAULT_GROUP) {
            anyhow::bail!(
                "Must specify a default upstream server, missing '{}' field in server section",
                DEFAULT_GROUP
            )
        }
        Ok(config)
    }
    pub fn attribute_group(&self, addr: &IpAddr) -> String {
        let group = self
            .groups
            .iter()
            .find(|(_, value)| {
                value.iter().any(|it| match it {
                    group::IpRange::Single(single) => match_ipaddr(single, addr),
                    group::IpRange::Range(range) => {
                        if range.is_empty() {
                            return false;
                        };
                        match addr {
                            IpAddr::V4(addr) => {
                                range.contains(&IpAddr::from(addr.to_ipv6_mapped()))
                            }
                            _ => range.contains(addr),
                        }
                    }
                })
            })
            .map(|it| it.0.as_str())
            .unwrap_or(DEFAULT_GROUP);
        group.to_string()
    }
    pub fn get_server(&self, group: impl AsRef<str>) -> &Vec<String> {
        let key = if self.servers.contains_key(group.as_ref()) {
            group.as_ref()
        } else {
            DEFAULT_GROUP
        };
        self.servers.get(key).as_ref().unwrap()
    }
    pub fn get_hosts(&self, group: impl AsRef<str>, domain: &str) -> anyhow::Result<Vec<IpAddr>> {
        let domain = Name::from_str(domain).with_context(||format!("Failed parse '{}' to Name", domain))?;
        let default = self.hosts.get(DEFAULT_GROUP).into_iter().flatten();
        let group = self.hosts.get(group.as_ref()).into_iter().flatten();
        Ok(group
            .chain(default)
            .find_map(|it| if it.1 == domain { Some(it.0) } else { None })
            // .filter_map(|it| if it.1 == domain { Some(it.0) } else { None })
            // .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>())
    }
    pub fn get_hostname(&self, group: impl AsRef<str>, addr: IpAddr) -> Option<String> {
        let default = self.hosts.get(DEFAULT_GROUP).into_iter().flatten();
        let group = self.hosts.get(group.as_ref()).into_iter().flatten();
        group.chain(default).find_map(|it| {
            if it.0 == addr {
                Some(it.1.to_utf8())
            } else {
                None
            }
        })
    }
    pub async fn is_allow_ipv6(&self, group: impl AsRef<str>, domain: &Name, addr: IpAddr) -> bool {
        let default_rules = self
            .ipv6_resolution
            .get(DEFAULT_GROUP)
            .into_iter()
            .flatten();
        let group_rules = self
            .ipv6_resolution
            .get(group.as_ref())
            .into_iter()
            .flatten();
        let rules = group_rules.chain(default_rules);
        for rule in rules {
            if !rule.payload_match(domain) {
                continue;
            }
            if !rule
                .check_is_allow(resolution::CheckArgs {
                    addr: &addr,
                    mmdb: self.metadata.mmdb.as_ref(),
                })
                .await
            {
                return false;
            }
            break;
        }
        true
    }
}

enum Section<'input> {
    Group,
    Server,
    Host(&'input str),
    Metadata,
    IPv6Resolution,
    Unknown(&'input str),
}

fn parse_section(section: &str) -> Section {
    let parts = section.split('.').collect::<Vec<_>>();
    match parts[0] {
        "group" => Section::Group,
        "server" => Section::Server,
        "hosts" => Section::Host(parts.get(1).copied().unwrap_or("default")),
        "metadata" => Section::Metadata,
        "ipv6_resolution" => Section::IPv6Resolution,
        _ => Section::Unknown(parts[0]),
    }
}

#[allow(clippy::wildcard_in_or_patterns)]
fn parse_key_value_pair(line: &str) -> Result<(String, String, usize), (anyhow::Error, usize)> {
    let mut key = String::new();
    let mut value = String::new();
    let (mut chars, mut column) = {
        let trimmed = line.trim_start();
        (trimmed.chars().peekable(), line.len() - trimmed.len())
    };
    let mut is_key = true;
    let mut in_quotes = false;
    let mut value_start_pos = 0;
    while let Some(ch) = chars.next() {
        column += 1;
        match ch {
            ' ' if is_key => {
                while let Some(&' ') = chars.peek() {
                    column += 1;
                    chars.next();
                }
                if !key.is_empty() {
                    is_key = false;
                    value_start_pos = column + 1;
                }
            }
            '"' => in_quotes = !in_quotes,
            '#' if !in_quotes => break,
            _ if ch.is_ascii_alphabetic()
                || ch.is_ascii_digit()
                || matches!(ch, '.' | '-' | ':')
                || !is_key
                || !ch.is_ascii() =>
            {
                if is_key {
                    key.push(ch)
                } else {
                    value.push(ch)
                }
            }
            _ => return Err((anyhow::format_err!("Unexpected character '{}'", ch), column)),
        }
    }
    if in_quotes {
        return Err((anyhow::format_err!("Unmatched quotes in input"), column));
    }
    Ok((key, value.trim().to_string(), value_start_pos))
}

fn match_ipaddr(a: &IpAddr, b: &IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V6(b)) | (IpAddr::V6(b), IpAddr::V4(a)) => &a.to_ipv6_mapped() == b,
        _ => a == b,
    }
}

fn read_hosts(path: &PathBuf) -> anyhow::Result<Vec<(String, Name)>> {
    if !path.is_file() {
        anyhow::bail!("host file does not exist, path = {:?}", path);
    }
    let hosts = fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for line in hosts.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let (key, mut value, _) = parse_key_value_pair(line)
            .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, line, col))?;
        // must be fqdn
        if !value.ends_with('.') {
            value.push('.')
        }
        entries.push((
            key.parse()
                .with_context(|| format!("Invalid ip addr '{}'", key))?,
            Name::from_ascii(value)?,
        ))
    }
    Ok(entries)
}


pub struct Config {
    ptr: AtomicPtr<Inner>,
    path: PathBuf
}

impl Config {
    pub fn new(path: PathBuf) -> anyhow::Result<Self> {
        // todo: watch directory
        let (inner, _watch_paths) =
            Inner::load(&path).with_context(|| "Failed to load config file")?;
        let inner_ptr = Arc::into_raw(Arc::new(inner)) as *mut Inner;
        let ptr = AtomicPtr::new(inner_ptr);
        Ok(Self {
            ptr,
            path
        })
    }
    /// 重载配置
    #[allow(unused)]
    pub fn reload(&self) -> anyhow::Result<()> {
        let (inner, _watch_paths) =
            Inner::load(&self.path).with_context(|| "Failed to load config file")?;
        let inner_ptr = Arc::into_raw(Arc::new(inner)) as *mut Inner;
        let old_ptr = self.ptr.swap(inner_ptr, Ordering::SeqCst);
        unsafe {
            // 转回 Arc 然后丢弃
            let _ = Arc::from_raw(old_ptr);
        };
        Ok(())
    }
    pub fn access(&self) -> Arc<Inner> {
        let ptr = self.ptr.load(Ordering::SeqCst);
        unsafe {
            // Temporarily create an Arc from the raw pointer
            let temp_arc = Arc::from_raw(ptr);
            // Clone the Arc to increase the reference count
            let cloned_arc = Arc::clone(&temp_arc);
            // Forget the temporary Arc to prevent decrementing the reference count on drop
            std::mem::forget(temp_arc);
            cloned_arc
        }
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        unsafe {
            // 转为 Arc 再丢弃
            let _ = Arc::from_raw(self.ptr.load(Ordering::SeqCst));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn it_works() {
        let path = Path::new("pomelo.conf");
        let config = Inner::load(&path.to_path_buf()).unwrap();
        println!("{:#?}", config);
    }
}
