use crate::config::parse_key_value_pair;
use anyhow::Context;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug)]
pub enum RollingRotation {
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Never,
}

#[derive(Debug)]
pub struct Log {
    pub level: tracing::Level,
    pub dir: Option<PathBuf>,
    pub max_files: Option<usize>,
    pub rotation: RollingRotation,
}

pub fn parse(row: usize, line: &str, log: &mut Log) -> anyhow::Result<()> {
    let (key, value,_) = parse_key_value_pair(line)
        .map_err(|(err, col)| anyhow::format_err!("{} in line {}:{}", err, row, col))?;
    match key.as_str() {
        "level" => {
            log.level = tracing::Level::from_str(&value)
                .with_context(|| format!("Failed to parse log level from value: '{}'", value))?;
        }
        "dir" => {
            let dir = PathBuf::from_str(&value).with_context(|| {
                format!("Failed to parse directory path from value: '{}'", value)
            })?;
            if !dir.is_dir() {
                anyhow::bail!(
                    "Provided log directory path is not a directory: '{}'",
                    value
                )
            }
            log.dir = Some(dir)
        }
        "max-files" => {
            let max_files = value
                .parse::<usize>()
                .with_context(|| format!("Failed to parse max_files from value: '{}'", value))?;
            log.max_files = Some(max_files)
        }
        "rotation" => {
            let rotation = match value.to_lowercase().as_str() {
                "hourly" => RollingRotation::Hourly,
                "daily" => RollingRotation::Daily,
                "weekly" => RollingRotation::Weekly,
                "monthly" => RollingRotation::Monthly,
                "never" => RollingRotation::Never,
                _ => anyhow::bail!("Invalid rotation value specified: '{}'. Valid values are 'hourly', 'daily', 'weekly', 'monthly', 'never'.", value)
            };
            log.rotation = rotation
        }
        _ => anyhow::bail!("Unknown log item {} specified: ", key),
    }
    Ok(())
}
