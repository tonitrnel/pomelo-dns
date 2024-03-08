use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use tokio::task::JoinHandle;
use tracing_subscriber::fmt::MakeWriter;

enum LogTask {
    Write(usize, Vec<u8>),
    Flush(usize),
    Reopen,
    AddFile(usize, PathBuf, File),
    Terminal,
}
pub struct Writer<'a> {
    id: usize,
    sender: &'a mpsc::Sender<LogTask>,
}
impl Write for Writer<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sender
            .send(LogTask::Write(self.id, buf.to_vec()))
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to send log task"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.sender
            .send(LogTask::Flush(self.id))
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to send flush task"))?;
        Ok(())
    }
}

pub struct LogWriter {
    id: usize,
    sender: mpsc::Sender<LogTask>,
}

impl LogWriter {
    pub fn new() -> anyhow::Result<(Self, JoinHandle<anyhow::Result<()>>)> {
        let (sender, tasks) = mpsc::channel::<LogTask>();
        let handle = tokio::spawn(async move {
            let mut map: HashMap<usize, (PathBuf, File)> = HashMap::new();
            for task in tasks {
                match task {
                    LogTask::Write(id, buf) => {
                        let file = match map.get_mut(&id) {
                            Some(r) => &mut r.1,
                            None => continue,
                        };
                        if let Err(err) = file.write_all(&buf) {
                            eprintln!("Failed to write to log file: {}", err);
                        }
                    }
                    LogTask::Flush(id) => {
                        let file = match map.get_mut(&id) {
                            Some(r) => &mut r.1,
                            None => continue,
                        };
                        if let Err(err) = file.flush() {
                            eprintln!("Failed to flush log file: {}", err);
                        };
                    }
                    LogTask::Reopen => {
                        for (_, (path, file)) in map.iter_mut() {
                            *file = match Self::open(path) {
                                Ok(file) => file,
                                Err(err) => {
                                    eprintln!("Failed to reopen log file: {}", err);
                                    return Err(err);
                                }
                            };
                        }
                    }
                    LogTask::AddFile(id, path, file) => {
                        map.insert(id, (path, file));
                    }
                    // 似乎不合适
                    LogTask::Terminal => break,
                }
            }
            Ok(()) as anyhow::Result<()>
        });
        Ok((Self { id: 0, sender }, handle))
    }
    fn open(path: &Path) -> anyhow::Result<File> {
        use anyhow::Context;
        OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .with_context(|| format!("Failed to open log file '{path:?}'"))
    }
    pub fn create_file_writer(&self, path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = Self::open(&path)?;
        let id = self.id + 1;
        self.sender
            .send(LogTask::AddFile(id, path, file))
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to send log task"))?;
        Ok(Self {
            id,
            sender: self.sender.clone(),
        })
    }
    /// 用于轮转日志，将会重新打开日志文件
    pub fn reopen(&self) -> anyhow::Result<()> {
        self.sender
            .send(LogTask::Reopen)
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Failed to send log task"))?;
        Ok(())
    }
    pub fn terminal(&self) {
        let _ = self.sender.send(LogTask::Terminal);
    }
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = Writer<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        Writer {
            id: self.id,
            sender: &self.sender,
        }
    }
}
