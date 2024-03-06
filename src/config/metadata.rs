use crate::config::{parse_key_value_pair, read_hosts, Inner, DEFAULT_GROUP};
use anyhow::Context;
use std::path::PathBuf;

#[derive(Debug)]
pub struct Metadata {
    pub addn_host: Option<PathBuf>,
    pub cache_size: usize,
    pub bind: String,
    pub mmdb: Option<maxminddb::Reader<Vec<u8>>>,
    pub access_log: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self{
            addn_host: None,
            cache_size: 0,
            bind: String::new(),
            mmdb: None,
            access_log: true,
        }
    }
}
pub fn parse(row: usize, line: &str, inner: &mut Inner) -> anyhow::Result<()> {
    let (key, value, _) = parse_key_value_pair(line)
        .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;
    match key.as_str() {
        "addn-host" => {
            let path = PathBuf::from(value);
            if !path.is_file() {
                anyhow::bail!("Add-on host file does not exist, path: '{:?}'", path);
            }
            let hosts = read_hosts(&path)?;
            for (addr, name) in hosts {
                inner
                    .hosts
                    .entry(DEFAULT_GROUP.to_string())
                    .or_default()
                    .push((
                        addr.parse()
                            .with_context(|| format!("Invalid ip addr '{}'", addr))?,
                        name,
                    ))
            }
            inner.metadata.addn_host = Some(path)
        }
        "cache-size" => {
            inner.metadata.cache_size = value
                .parse::<u32>()
                .with_context(|| format!("Invalid u32 value '{}'", value))?
                as usize;
        }
        "bind" => {
            inner.metadata.bind = value;
        }
        "mmdb" => {
            let value = PathBuf::from(value);
            if !value.is_file() {
                anyhow::bail!("GeoIP file does not exist");
            }
            let reader = maxminddb::Reader::open_readfile(&value)
                .with_context(|| format!("Failed to parse mmdb file on '{:?}'", value))?;
            inner.metadata.mmdb = Some(reader);
        }
        "access_log" => {
            let value = match value.as_str() {
                "on" | "true" | "1" => true,
                "off" | "false" | "0" => false,
                _ => true,
            };
            inner.metadata.access_log = value;
        }
        _ => anyhow::bail!("Unknown metadata item specified: '{}'", key),
    }
    Ok(())
}
