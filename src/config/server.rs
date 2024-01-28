use std::collections::HashMap;
use crate::config::{Inner, DEFAULT_GROUP, parse_key_value_pair};

pub type Servers = HashMap<String, Vec<String>>;

pub fn parse(row: usize, line: &str, inner: &mut Inner) -> anyhow::Result<()> {
    let (key, value, _) = parse_key_value_pair(line)
        .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;
    if key != DEFAULT_GROUP && !inner.groups.contains_key(&key) {
        anyhow::bail!("Can't find group '{}' definition in {line}:1", key);
    }
    let addrs = value
        .split(',')
        .map(|it| it.trim().to_string())
        .collect::<Vec<_>>();
    inner.servers.insert(key, addrs);
    Ok(())
}