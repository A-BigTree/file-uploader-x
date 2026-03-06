use std::fmt;
use std::io;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::{
    FmtContext,
    format::{FormatEvent, FormatFields, Writer},
};
use tracing_subscriber::registry::LookupSpan;

// logging configuration
pub struct UploaderLoggingFormatter;

impl<S, N> FormatEvent<S, N> for UploaderLoggingFormatter
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let thread_id = std::thread::current().id();
        let level = event.metadata().level();

        write!(writer, "[{}][{:?}][{}] ", timestamp, thread_id, level)?;

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

pub fn init_logging<P: AsRef<std::path::Path>>(log_file_path: P) -> io::Result<()> {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::prelude::*;

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // let file = std::fs::File::create(log_file_path)?;

    let stdout_layer = fmt::layer()
        .with_target(false)
        .event_format(UploaderLoggingFormatter)
        .with_filter(env_filter.clone());

    /*
    let file_layer = fmt::layer()
        .with_target(false)
        .event_format(UploaderLoggingFormatter)
        .with_writer(file)
        .with_filter(env_filter);
    */

    tracing_subscriber::registry()
        .with(stdout_layer)
        // .with(file_layer)
        .init();

    Ok(())
}
