//! Captures Bevy's tracing logs and forwards them to the editor's Console
//! panel through the IPC channel.

use bevy::log::BoxedLayer;
use bevy::prelude::*;
use crossbeam_channel::Sender;
use tracing_subscriber::layer::Context as TraceContext;
use tracing_subscriber::Layer;

use crate::state::IpcChannels;

/// Install hook passed to `LogPlugin::custom_layer`.
///
/// Creates the log channel, installs the drain resource and returns the
/// tracing layer that feeds it.
pub fn ipc_log_layer(app: &mut App) -> Option<BoxedLayer> {
    let (tx, rx) = crossbeam_channel::unbounded::<forge_ipc::LogEntry>();
    app.insert_resource(LogDrain(rx));
    Some(Box::new(ForgeLogLayer { tx }))
}

/// Main-world resource draining captured entries into IPC batches.
#[derive(Resource)]
pub struct LogDrain(crossbeam_channel::Receiver<forge_ipc::LogEntry>);

/// tracing layer serialising events into [`forge_ipc::LogEntry`]s.
struct ForgeLogLayer {
    tx: Sender<forge_ipc::LogEntry>,
}

struct FieldVisitor {
    message: String,
    extra: String,
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
        } else {
            if !self.extra.is_empty() {
                self.extra.push(' ');
            }
            self.extra.push_str(&format!("{}={value:?}", field.name()));
        }
    }
}

impl<S> Layer<S> for ForgeLogLayer
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: TraceContext<'_, S>) {
        let mut visitor = FieldVisitor { message: String::new(), extra: String::new() };
        event.record(&mut visitor);
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => forge_ipc::LogLevel::Error,
            tracing::Level::WARN => forge_ipc::LogLevel::Warn,
            tracing::Level::INFO => forge_ipc::LogLevel::Info,
            tracing::Level::DEBUG => forge_ipc::LogLevel::Debug,
            tracing::Level::TRACE => forge_ipc::LogLevel::Trace,
        };
        let mut message = visitor.message;
        if !visitor.extra.is_empty() {
            if !message.is_empty() {
                message.push(' ');
            }
            message.push_str(&visitor.extra);
        }
        let entry = forge_ipc::LogEntry {
            level,
            time: timestamp_hms(),
            target: event.metadata().target().to_string(),
            message,
        };
        // A dropped entry under an extreme log storm is acceptable.
        let _ = self.tx.send(entry);
    }
}

/// "12:45:10" wall-clock stamp for console rows.
pub fn timestamp_hms() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % 86_400;
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// Drains captured entries and ships them to the editor in batches.
pub fn push_logs(drain: Option<ResMut<LogDrain>>, channels: Res<IpcChannels>) {
    let Some(drain) = drain else { return };
    let mut batch = Vec::new();
    while let Ok(entry) = drain.0.try_recv() {
        batch.push(entry);
        if batch.len() >= 64 {
            break;
        }
    }
    if !batch.is_empty() {
        let _ = channels.evt_tx.send(forge_ipc::RuntimeToEditor::Logs(batch));
    }
}
