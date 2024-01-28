use crate::config::{parse_key_value_pair, read_hosts, Inner, DEFAULT_GROUP};
use anyhow::Context;
use hickory_proto::rr::Name;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;

pub type Hosts = Vec<(IpAddr, Name)>;
pub type GroupHostMappings = HashMap<String, Hosts>;

pub fn parse(sub: &str, row: usize, line: &str, inner: &mut Inner) -> anyhow::Result<()> {
    if sub != DEFAULT_GROUP && !inner.groups.contains_key(sub) {
        anyhow::bail!("Can't find group '{}' definition in {row}:8", sub);
    }
    if let Some(end) = line.trim_start().strip_prefix("@include") {
        let path = PathBuf::from(end.trim());
        if !path.is_file() {
            anyhow::bail!("Include file does not exist, path: '{:?}'", path);
        }
        let hosts = read_hosts(&path)?
            .into_iter()
            .map(|(addr, name)| {
                Ok((
                    addr.parse()
                        .with_context(|| format!("Invalid ip addr '{}'", addr))?,
                    name,
                ))
            })
            .collect::<Result<Vec<_>, anyhow::Error>>()?;
        inner
            .hosts
            .entry(sub.to_string())
            .or_default()
            .extend(hosts);
    } else {
        let (key, value, _) = parse_key_value_pair(line)
            .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;

        inner.hosts.entry(sub.to_string()).or_default().push((
            key.parse()
                .with_context(|| format!("Invalid ip addr '{}'", key))?,
            Name::from_ascii(value)?,
        ));
    }
    Ok(())
}
