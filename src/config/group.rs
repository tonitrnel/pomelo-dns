use crate::config::parse_key_value_pair;
use anyhow::Context;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ops::Range;
use std::str::FromStr;

#[derive(Debug)]
pub enum IpRange {
    Single(IpAddr),
    Range(Range<IpAddr>),
}

pub type Groups = HashMap<String, Vec<IpRange>>;

pub fn parse(row: usize, line: &str, groups: &mut Groups) -> anyhow::Result<()> {
    let (key, value, _) = parse_key_value_pair(line)
        .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;
    groups.insert(key, parse_ip_range(&value)?);
    Ok(())
}

fn parse_ip_range(input: &str) -> anyhow::Result<Vec<IpRange>> {
    let mut list = Vec::new();
    for part in input.split(',').map(|it| it.trim()) {
        if part.contains('-') {
            let parts: Vec<&str> = part.split('-').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid range format {}", part);
            }
            let start = parts[0].trim().parse::<IpAddr>().map(to_ipv6_mapped)?;
            let end = parts[1].trim().parse::<IpAddr>().map(to_ipv6_mapped)?;
            list.push(IpRange::Range(Range { start, end }))
        } else if part.contains('/') {
            let parts: Vec<&str> = part.split('/').collect();
            if parts.len() != 2 {
                anyhow::bail!("Invalid CIDR format {}", part);
            }
            let ip = IpAddr::from_str(parts[0])?;
            let prefix_len = parts[1].parse::<u32>()?;
            if (ip.is_ipv4() && prefix_len > 32) || (ip.is_ipv6() && prefix_len > 128) {
                anyhow::bail!("prefix length {}", prefix_len);
            }
            match ip {
                IpAddr::V4(addr) => {
                    let mask = !((1 << (32 - prefix_len)) - 1);
                    let start_ip = Ipv4Addr::from(u32::from(addr) & mask);
                    let end_ip = Ipv4Addr::from(u32::from(start_ip) | (!mask));
                    list.push(IpRange::Range(Range {
                        start: IpAddr::from(start_ip.to_ipv6_mapped()),
                        end: IpAddr::from(end_ip.to_ipv6_mapped()),
                    }));
                }
                IpAddr::V6(addr) => {
                    let addr = u128::from(addr);
                    let mask = !((1u128 << (128 - prefix_len)) - 1);
                    let start_ip = Ipv6Addr::from(addr & mask);
                    let end_ip = Ipv6Addr::from(addr | (!mask));
                    list.push(IpRange::Range(Range {
                        start: IpAddr::V6(start_ip),
                        end: IpAddr::V6(end_ip),
                    }))
                }
            }
        } else {
            list.push(IpRange::Single(
                part.parse::<IpAddr>()
                    .map(to_ipv6_mapped)
                    .with_context(|| format!("{:?}", part))?,
            ));
        }
    }
    Ok(list)
}

fn to_ipv6_mapped(addr: IpAddr) -> IpAddr {
    match addr {
        IpAddr::V4(addr) => IpAddr::from(addr.to_ipv6_mapped()),
        _ => addr,
    }
}
