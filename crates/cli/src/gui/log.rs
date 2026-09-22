//! GUI 运行日志：内存环形缓冲 + tracing Layer + 查询接口。
//!
//! 目的：GUI 后端是长期运行的本地服务，错误散落在控制台。本模块把
//! - API 层事件（请求/状态/耗时/错误）
//! - tracing 的 warn/error 事件
//! 收集到固定容量的环形缓冲，供前端「日志」面板展示，便于排查问题。

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 环形缓冲容量
const CAPACITY: usize = 500;

/// 一条日志
#[derive(Debug, Clone, serde::Serialize)]
pub struct LogEntry {
    /// Unix 毫秒时间戳
    pub ts_ms: u128,
    /// 级别：INFO / WARN / ERROR
    pub level: String,
    /// 来源：api / tracing
    pub source: String,
    pub message: String,
}

/// 共享日志存储（线程安全）
#[derive(Clone, Default)]
pub struct LogStore {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl LogStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(CAPACITY))),
        }
    }

    /// 追加一条日志（自动淘汰最旧的）
    pub fn push(&self, level: &str, source: &str, message: impl Into<String>) {
        let entry = LogEntry {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            level: level.to_string(),
            source: source.to_string(),
            message: message.into(),
        };
        if let Ok(mut buf) = self.inner.lock() {
            if buf.len() >= CAPACITY {
                buf.pop_front();
            }
            buf.push_back(entry);
        }
    }

    /// 便捷：记录 API 层日志
    pub fn api(&self, message: impl Into<String>) {
        self.push("INFO", "api", message);
    }

    /// 取全部日志（按时间正序）
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner
            .lock()
            .map(|buf| buf.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// 清空
    pub fn clear(&self) {
        if let Ok(mut buf) = self.inner.lock() {
            buf.clear();
        }
    }
}

/// 把本 LogStore 作为 tracing 的 Layer 挂载为全局 subscriber。
///
/// 只收集 WARN / ERROR（INFO 及以下忽略，避免噪音）。
pub fn init_tracing(store: LogStore) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let layer = TracingLayer { store };
    let subscriber = tracing_subscriber::registry().with(layer);
    // 忽略重复设置（例如多次初始化）——不致命
    let _ = subscriber.try_init();
}

/// tracing Layer：把 warn/error 事件写入 LogStore
struct TracingLayer {
    store: LogStore,
}

impl<S> tracing_subscriber::Layer<S> for TracingLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let level = match *meta.level() {
            tracing::Level::ERROR => "ERROR",
            tracing::Level::WARN => "WARN",
            _ => return, // 只收 warn/error
        };
        // 收集事件的 message 字段
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let msg = if visitor.0.is_empty() {
            meta.target().to_string()
        } else {
            format!("[{}] {}", meta.target(), visitor.0)
        };
        self.store.push(level, "tracing", msg);
    }
}

/// 从 tracing 事件里提取 message 字段
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{:?}", value);
        } else if self.0.is_empty() {
            self.0 = format!("{}={:?}", field.name(), value);
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }
}
