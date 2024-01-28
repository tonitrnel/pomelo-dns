use nu_ansi_term::{Color, Style};
use std::collections::HashMap;
use std::fmt::{Debug, Display, Formatter, Write};
use std::sync::{Arc, Mutex};
use std::{env, fmt};
use tracing::{field::Field, span::Attributes, Event, Id, Level, Subscriber};
use tracing_subscriber::{
    field::Visit,
    fmt::{
        format,
        time::{ChronoLocal, FormatTime},
    },
    layer::{Context, SubscriberExt},
    registry::LookupSpan,
    util::SubscriberInitExt,
    {filter, Layer},
};
use crate::config;

struct SequentialLogLayer {
    ansi: bool,
    logs: Arc<Mutex<HashMap<Id, Vec<String>>>>,
}

struct StringVisitor {
    content: String,
}

impl StringVisitor {
    fn new() -> Self {
        Self {
            content: String::new(),
        }
    }
}

impl Display for StringVisitor {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.content)
    }
}

impl Visit for StringVisitor {
    fn record_debug(&mut self, _field: &Field, value: &dyn Debug) {
        write!(self.content, "{:?}", value).unwrap();
    }
}

impl SequentialLogLayer {
    fn new() -> Self {
        let ansi = env::var("NO_COLOR").map_or(true, |v| v.is_empty());
        Self {
            ansi,
            logs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

struct SequentialLogLayerFormatter<'input, S> {
    ansi: bool,
    event: &'input Event<'input>,
    ctx: Context<'input, S>,
}

impl<S> SequentialLogLayerFormatter<'_, S>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn format_level(&self, f: &mut Formatter<'_>, level: &Level) -> fmt::Result {
        if self.ansi {
            let str = format!("{:>5}", level);
            match *level {
                Level::TRACE => write!(f, "{}", Color::Purple.paint(str)),
                Level::DEBUG => write!(f, "{}", Color::Blue.paint(str)),
                Level::INFO => write!(f, "{}", Color::Green.paint(str)),
                Level::WARN => write!(f, "{}", Color::Yellow.paint(str)),
                Level::ERROR => write!(f, "{}", Color::Red.paint(str)),
            }?;
        } else {
            write!(f, "{:>5}", level)?;
        }
        f.write_char(' ')
    }
    fn format_timestamp(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let timer = ChronoLocal::new("%F %X%.3f".to_string());
        let mut writer = format::Writer::new(f);
        let style = self.dimmed();
        write!(writer, "{}", style.prefix())?;
        timer.format_time(&mut writer)?;
        write!(writer, "{}", style.suffix())?;
        f.write_char(' ')
    }
    fn format_scope(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let bold = self.bold();
        let mut seen = false;
        let span = self
            .event
            .parent()
            .and_then(|id| self.ctx.span(id))
            .or_else(|| self.ctx.lookup_current());
        let scope = span.into_iter().flat_map(|span| span.scope().from_root());

        for span in scope {
            seen = true;
            write!(f, "{}", bold.paint(span.metadata().name()))?;
        }

        if seen {
            f.write_char(' ')?
        }
        Ok(())
    }
    fn format_fields(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut visitor = StringVisitor::new();
        self.event.record(&mut visitor);
        write!(f, "{} ", visitor)
    }
    fn bold(&self) -> Style {
        if self.ansi {
            Style::new().bold()
        } else {
            Style::new()
        }
    }
    fn dimmed(&self) -> Style {
        if self.ansi {
            Style::new().dimmed()
        } else {
            Style::new()
        }
    }
}

impl<S> Display for SequentialLogLayerFormatter<'_, S>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let meta = self.event.metadata();
        self.format_timestamp(f)?;
        let level = meta.level();
        self.format_level(f, level)?;
        if *level != Level::ERROR {
            self.format_scope(f)?;
        }
        self.format_fields(f)?;
        Ok(())
    }
}

impl<S> Layer<S> for SequentialLogLayer
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let mut logs = self.logs.lock().unwrap();
        logs.entry(id.clone()).or_default();
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        if let Some(span_id) = ctx.current_span().id() {
            let mut logs = self.logs.lock().unwrap();
            if let Some(pool) = logs.get_mut(span_id) {
                let formatter = SequentialLogLayerFormatter {
                    ansi: self.ansi,
                    event,
                    ctx,
                };
                pool.push(format!("{}", formatter))
            }
        }
    }
    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        let mut logs = self.logs.lock().unwrap();
        if let Some(messages) = logs.remove(&id) {
            for message in messages {
                println!("{}", message)
            }
        }
    }
}

pub fn registry_logs(_log: &config::log::Log) {
    let filter = filter::Targets::new().with_target("pomelo", Level::TRACE);

    let generic_layer = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_target(false)
        .with_timer(ChronoLocal::new("%F %X%.3f".to_string()))
        .with_filter(filter::filter_fn(|metadata| {
            metadata.target() != "pomelo::handler" && metadata.target() != "sequential"
        }));

    let sequential_layer = SequentialLogLayer::new().with_filter(filter::filter_fn(|metadata| {
        metadata.target() == "pomelo::handler" || metadata.target() == "sequential"
    }));
    tracing_subscriber::registry()
        .with(filter)
        .with(generic_layer)
        .with(sequential_layer)
        .init();
}
