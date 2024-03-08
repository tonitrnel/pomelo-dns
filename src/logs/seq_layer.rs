use std::collections::HashMap;
use std::env;
use std::fmt::{self, Debug, Display, Formatter, Write};
use std::io;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use nu_ansi_term::{Color, Style};
use tracing::{field::Field, span::Attributes, Event, Id, Level, Subscriber, Metadata};
use tracing_subscriber::{
    field::Visit,
    fmt::MakeWriter,
    fmt::{
        format,
        time::{ChronoLocal, FormatTime},
    },
    layer::Context,
    registry::LookupSpan,
    Layer,
};

#[derive(Copy, Clone)]
struct FormatterArgs {
    ansi: bool,
    display_target: bool,
    display_filename: bool,
    display_line_number: bool,
    display_level: bool,
}

pub struct SequentialLogLayer<S, W = fn() -> io::Stdout> {
    fmt_args: FormatterArgs,
    logs: Arc<Mutex<HashMap<Id, Vec<String>>>>,
    make_writer: W,
    _inner: PhantomData<fn(S)>,
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

struct SequentialLogLayerFormatter<'input, S> {
    fmt_args: FormatterArgs,
    event: &'input Event<'input>,
    ctx: Context<'input, S>,
}

impl<S> SequentialLogLayerFormatter<'_, S>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn format_level(&self, f: &mut Formatter<'_>, level: &Level) -> fmt::Result {
        if !self.fmt_args.display_level { return Ok(())  }
        if self.fmt_args.ansi {
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
    fn format_target(&self, f: &mut Formatter<'_>, meta: &Metadata, dimmed: &Style) -> fmt::Result{
        if !self.fmt_args.display_target { return Ok(()) }
        write!(f, "{}{} ", dimmed.paint(meta.target()), dimmed.paint(":"))
    }
    fn format_file(&self,f: &mut Formatter<'_>, meta: &Metadata, dimmed: &Style) -> fmt::Result{
        if !self.fmt_args.display_filename { return Ok(()) }
        let line_number = if self.fmt_args.display_line_number{
            meta.line()
        } else { 
            None
        };
        let filename = if let Some(filename) = meta.file() {
            filename
        } else { 
            return Ok(())
        };
        write!(f, "{}{}{}", dimmed.paint(filename), dimmed.paint(":"), if line_number.is_some() {""} else {" "})?;
        if let Some(line_number) = line_number{
            write!(f, "{}{}:{} ",dimmed.prefix(),line_number,dimmed.suffix())?;
        }
        Ok(())
    }
    fn bold(&self) -> Style {
        if self.fmt_args.ansi {
            Style::new().bold()
        } else {
            Style::new()
        }
    }
    fn dimmed(&self) -> Style {
        if self.fmt_args.ansi {
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
        
        let dimmed = self.dimmed();
        
        if *level != Level::ERROR {
            self.format_scope(f)?;
        }
        
        self.format_target(f, meta, &dimmed)?;
        
        self.format_file(f, meta, &dimmed)?;
        
        self.format_fields(f)?;
        writeln!(f)
    }
}

/// 用于保证 Span 内日志输出的顺序
impl<S> SequentialLogLayer<S> {
    fn new() -> Self {
        let ansi = env::var("NO_COLOR").map_or(true, |v| v.is_empty());
        Self {
            fmt_args: FormatterArgs {
                ansi,
                display_level: true,
                display_target: false,
                display_filename: false,
                display_line_number: false,
            },
            logs: Arc::new(Mutex::new(HashMap::new())),
            make_writer: io::stdout,
            _inner: PhantomData,
        }
    }
    #[allow(unused)]
    pub fn with_ansi(self, ansi: bool) -> Self {
        Self {
            fmt_args: FormatterArgs {
                ansi,
                ..self.fmt_args
            },
            ..self
        }
    }

    #[allow(unused)]
    pub fn with_target(self, display_target: bool) -> Self {
        Self {
            fmt_args: FormatterArgs{
                display_target,
                ..self.fmt_args
            },
            ..self
        }
    }
    #[allow(unused)]
    pub fn with_file(self, display_filename: bool) -> Self {
        Self {
            fmt_args: FormatterArgs{
                display_filename,
                ..self.fmt_args
            },
            ..self
        }
    }
    #[allow(unused)]
    pub fn with_line_number(self, display_line_number: bool) -> Self {
        Self {
            fmt_args: FormatterArgs{
                display_line_number,
                ..self.fmt_args
            },
            ..self
        }
    }
}
impl<S, W> SequentialLogLayer<S, W>
where
    W: for<'writer> MakeWriter<'writer> + 'static,
{
    #[allow(unused)]
    pub fn with_writer<W2>(self, make_writer: W2) -> SequentialLogLayer<S, W2>
    where
        W2: for<'writer> MakeWriter<'writer> + 'static,
    {
        SequentialLogLayer {
            fmt_args: self.fmt_args,
            _inner: self._inner,
            logs: self.logs,
            make_writer,
        }
    }
}

impl<S, W> Layer<S> for SequentialLogLayer<S, W>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    W: for<'writer> MakeWriter<'writer> + 'static,
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
                    fmt_args: self.fmt_args,
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
            let mut writer = self.make_writer.make_writer();
            for mut message in messages {
                if !message.ends_with('\n') {
                    message.push('\n')
                }
                io::Write::write_all(&mut writer, message.as_bytes()).unwrap()
            }
        }
    }
}

pub fn layer<S>() -> SequentialLogLayer<S> {
    SequentialLogLayer::new()
}
