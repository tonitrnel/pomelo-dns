use std::fs::{File, OpenOptions};
use std::path::Path;
use tracing::subscriber::DefaultGuard;
use tracing::Level;
use tracing_subscriber::{
    fmt::time::ChronoLocal,
    layer::SubscriberExt,
    {filter, Layer},
};

mod log_writer;
mod seq_layer;

pub use log_writer::LogWriter;

#[allow(unused)]
fn log_file(path: &Path) -> anyhow::Result<File> {
    use anyhow::Context;
    OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .with_context(|| format!("Failed to open log file '{path:?}'"))
}
pub fn registry_logs(writer: &mut LogWriter, access_log: bool) -> anyhow::Result<DefaultGuard> {
    let mut layers = Vec::new();
    let targets = filter::Targets::new().with_target("pomelo", Level::TRACE);
    let generic_layer = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_target(false)
        .with_timer(ChronoLocal::new("%F %X%.3f".to_string()))
        .with_filter(filter::filter_fn(|metadata| {
            metadata.target() != "pomelo::handler" && metadata.target() != "sequential"
        }));
    layers.push(generic_layer.boxed());
    #[cfg(target_os = "linux")]
    {
        let file = writer.create(Path::new("/var/log/pomelo/error.log"))?;
        let error_layer = tracing_subscriber::fmt::layer()
            .with_level(false)
            .with_target(true)
            .with_ansi(false)
            .compact()
            .with_timer(ChronoLocal::new("%F %X%.3f".to_string()))
            .with_writer(file)
            .with_filter(filter::filter_fn(|metadata| {
                metadata.target() != "pomelo::handler"
                    && metadata.target() != "sequential"
                    && metadata.level() >= &Level::DEBUG
            }));
        layers.push(error_layer.boxed());
    }

    if access_log {
        let sequential_layer = seq_layer::layer().with_filter(filter::filter_fn(|metadata| {
            metadata.target() == "pomelo::handler" || metadata.target() == "sequential"
        }));
        layers.push(sequential_layer.boxed());
        #[cfg(target_os = "linux")]
        {
            let file = writer.create(Path::new("/var/log/pomelo/access.log"))?;
            let access_layer = seq_layer::layer()
                .with_ansi(false)
                .with_writer(file)
                .with_filter(filter::filter_fn(|metadata| {
                    metadata.target() == "pomelo::handler" || metadata.target() == "sequential"
                }));
            layers.push(access_layer.boxed())
        }
    }
    let subscriber = tracing_subscriber::registry().with(targets).with(layers);
    let guard = tracing::subscriber::set_default(subscriber);
    Ok(guard)
}