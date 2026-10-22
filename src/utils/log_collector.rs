//! 全局日志收集 — Global log collection via tracing subscriber.
//!
//! 程序无控制台（`#![windows_subsystem = "windows"]`），功能线程里的
//! `println`/`eprintln` 会被静默丢弃。本模块提供安装到全局的 tracing
//! subscriber：将 INFO 以上的事件格式化为文本行收集到共享 buffer，
//! GUI 主线程每帧 [`drain`] 到日志面板。

use std::io;
use std::sync::{Arc, Mutex, TryLockError};
use tracing::Level;
use tracing_subscriber::fmt::MakeWriter;

/// 日志收集器 — 共享 buffer 的句柄（Clone 廉价），GUI 每帧调用 [`drain`]。
#[derive(Clone)]
pub struct LogCollector {
    buf: Arc<Mutex<Vec<String>>>,
}

impl LogCollector {
    /// 安装全局 tracing subscriber（INFO 以上），返回收集器句柄。
    ///
    /// 每个进程只能安装一次（`set_global_default` 唯一）；必须在任何
    /// 功能线程输出日志之前调用。收集的行数与 [`cap`] 一致（超出丢弃最旧）。
    pub fn install(cap: usize) -> LogCollector {
        let buf = Arc::new(Mutex::new(Vec::with_capacity(64)));
        let writer = LogWriter {
            buf: buf.clone(),
            cap,
        };
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(Level::INFO)
            .with_writer(writer)
            .finish();
        tracing::subscriber::set_global_default(subscriber)
            .expect("tracing subscriber already set");
        LogCollector { buf }
    }

    /// 取走全部已收集的日志行（不阻塞）。每帧调用一次即可。
    pub fn drain(&self) -> Vec<String> {
        let mut buf = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *buf)
    }

    /// 清空已缓冲的日志行（GUI 日志面板 Clear 按钮）。
    ///
    /// 与 GUI 已显示的行一起清空后，面板保持空白；之后新写入的日志照常收集。
    pub fn clear(&self) {
        self.buf.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// 非阻塞快照（panic hook 崩溃落盘专用）：`try_lock` 拿不到锁
    /// （WouldBlock）返回 `None` — 调用方写空日志段，绝不等待：持锁线程
    /// 可能正是 panic 线程，阻塞式 `lock` 会永久死锁。毒化（Poisoned）时
    /// 取回内部数据 — 日志 buffer 只是现场快照，毒化无碍。
    pub fn try_snapshot(&self) -> Option<Vec<String>> {
        match self.buf.try_lock() {
            Ok(buf) => Some(buf.clone()),
            Err(TryLockError::Poisoned(e)) => Some(e.into_inner().clone()),
            Err(TryLockError::WouldBlock) => None,
        }
    }
}

/// 写入端 — tracing-subscriber fmt 对每个事件一次 `write_all`（完整一行），
/// 因此一次 `write` 即一行；去除尾部换行后压入共享 buffer。
struct LogWriter {
    buf: Arc<Mutex<Vec<String>>>,
    cap: usize,
}

impl<'a> MakeWriter<'a> for LogWriter {
    type Writer = LogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        LogWriter {
            buf: self.buf.clone(),
            cap: self.cap,
        }
    }
}

impl io::Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf)
            .trim_end_matches('\n')
            .to_string();
        if !line.is_empty() {
            let mut lines = self.buf.lock().unwrap_or_else(|e| e.into_inner());
            lines.push(line);
            if lines.len() > self.cap {
                let excess = lines.len() - self.cap;
                lines.drain(0..excess);
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 不安装全局 subscriber，直接构造共享 buffer（install 每进程只能一次）。
    fn collector() -> LogCollector {
        LogCollector {
            buf: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[test]
    fn drain_takes_all_pending_lines() {
        let c = collector();
        c.buf
            .lock()
            .unwrap()
            .extend(["first".to_string(), "second".to_string()]);
        assert_eq!(c.drain(), vec!["first".to_string(), "second".to_string()]);
        assert!(c.drain().is_empty());
    }

    #[test]
    fn clear_discards_pending_lines() {
        let c = collector();
        c.buf
            .lock()
            .unwrap()
            .extend(["old".to_string(), "older".to_string()]);
        c.clear();
        assert!(c.drain().is_empty());
        // 清空后新日志仍可正常收集。
        c.buf.lock().unwrap().push("new".to_string());
        assert_eq!(c.drain(), vec!["new".to_string()]);
    }
}
