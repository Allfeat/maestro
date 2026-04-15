//! Custom `tracing` layer that pushes formatted log records into a shared
//! ring buffer consumed by the TUI logs panel.
//!
//! Only installed when `--tui` is active: in that mode stdout belongs to
//! ratatui and no `fmt` layer may run. The buffer is an `Arc<Mutex<…>>`
//! cloned between the tracing layer (producer) and `AppState` (consumer).

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const DEFAULT_CAPACITY: usize = 2048;

#[derive(Clone, Debug)]
pub struct LogLine {
    pub level: Level,
    pub message: String,
    pub fields: String,
    pub at: SystemTime,
}

/// Bounded ring buffer of log lines, shared across threads.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<LogLine>>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    fn push(&self, line: LogLine) {
        let mut buf = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if buf.len() == self.capacity {
            buf.pop_front();
        }
        buf.push_back(line);
    }

    /// Snapshot matching `min_level` (lower-severity levels filtered out).
    /// Returns oldest-to-newest, suitable for directly slicing the tail.
    pub fn snapshot_filtered(&self, min_level: Option<Level>) -> Vec<LogLine> {
        let buf = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        buf.iter()
            .filter(|l| match min_level {
                None => true,
                Some(min) => l.level <= min, // tracing: ERROR < WARN < INFO
            })
            .cloned()
            .collect()
    }
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

/// `tracing` layer that captures every event into a [`LogBuffer`].
pub struct TuiLogLayer {
    buffer: LogBuffer,
}

impl TuiLogLayer {
    pub fn new(buffer: LogBuffer) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for TuiLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = LogVisitor::default();
        event.record(&mut visitor);
        self.buffer.push(LogLine {
            level: *meta.level(),
            message: visitor.message,
            fields: visitor.fields,
            at: SystemTime::now(),
        });
    }
}

#[derive(Default)]
struct LogVisitor {
    message: String,
    fields: String,
}

impl LogVisitor {
    fn push_field(&mut self, name: &str, value: impl std::fmt::Display) {
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(&mut self.fields, "{name}={value}");
    }
}

impl Visit for LogVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.push_field(field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(&mut self.message, "{value:?}");
        } else {
            self.push_field(field.name(), format_args!("{value:?}"));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push_field(field.name(), value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push_field(field.name(), value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push_field(field.name(), value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push_field(field.name(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::subscriber::with_default;
    use tracing_subscriber::prelude::*;

    #[test]
    fn layer_captures_message_and_fields() {
        let buffer = LogBuffer::with_capacity(16);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(buffer.clone()));

        with_default(subscriber, || {
            tracing::info!(block = 42u64, "indexed");
            tracing::warn!("slow");
            tracing::error!(error = "boom", "failed");
        });

        let lines = buffer.snapshot_filtered(None);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0].level, Level::INFO);
        assert_eq!(lines[0].message, "indexed");
        assert!(lines[0].fields.contains("block=42"));
        assert_eq!(lines[1].message, "slow");
        assert_eq!(lines[2].level, Level::ERROR);
        assert!(lines[2].fields.contains("error=") && lines[2].fields.contains("boom"));
    }

    #[test]
    fn ring_buffer_drops_oldest_past_capacity() {
        let buffer = LogBuffer::with_capacity(3);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(buffer.clone()));
        with_default(subscriber, || {
            for i in 0..5u64 {
                tracing::info!(i, "tick");
            }
        });
        let lines = buffer.snapshot_filtered(None);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].fields.contains("i=2"));
        assert!(lines[2].fields.contains("i=4"));
    }

    #[test]
    fn filter_keeps_only_min_level_and_higher() {
        let buffer = LogBuffer::with_capacity(16);
        let subscriber = tracing_subscriber::registry().with(TuiLogLayer::new(buffer.clone()));
        with_default(subscriber, || {
            tracing::info!("i");
            tracing::warn!("w");
            tracing::error!("e");
        });
        let warn_plus = buffer.snapshot_filtered(Some(Level::WARN));
        assert_eq!(warn_plus.len(), 2);
        assert!(warn_plus.iter().all(|l| l.level <= Level::WARN));

        let err_only = buffer.snapshot_filtered(Some(Level::ERROR));
        assert_eq!(err_only.len(), 1);
        assert_eq!(err_only[0].level, Level::ERROR);
    }
}
