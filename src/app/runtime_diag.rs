//! Runtime diagnostics init and the panic report hook.

#![allow(unused_imports)]

use crate::app::*;
use crate::prelude::*;

pub(crate) fn init_runtime_diagnostics() {
    pub(crate) static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let filter = crate::diag::setting("RIV_LOG")
            .or_else(|| std::env::var("RUST_LOG").ok())
            .unwrap_or_else(|| "warn".to_string());

        let env_filter = tracing_subscriber::EnvFilter::try_new(filter)
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));

        let _ = tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .compact()
            .try_init();

        if crate::diag::setting("RIV_PUFFIN").is_some() {
            puffin::set_scopes_on(true);
        }
    });
}

pub(crate) fn install_panic_report_hook() {
    pub(crate) static INIT: OnceLock<()> = OnceLock::new();

    INIT.get_or_init(|| {
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let payload = if let Some(msg) = panic_info.payload().downcast_ref::<&str>() {
                (*msg).to_string()
            } else if let Some(msg) = panic_info.payload().downcast_ref::<String>() {
                msg.clone()
            } else {
                "<non-string panic payload>".to_string()
            };

            let location = panic_info
                .location()
                .map(|loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()))
                .unwrap_or_else(|| "<unknown location>".to_string());

            let current_thread = std::thread::current();
            let thread_name = current_thread.name().unwrap_or("unnamed");
            let backtrace = std::backtrace::Backtrace::force_capture();
            let timestamp = format!("{:?}", std::time::SystemTime::now());

            let panic_report = format!(
                "[{timestamp}] thread='{thread_name}'\nlocation: {location}\npayload: {payload}\n\nbacktrace:\n{backtrace}\n"
            );

            let log_dir = std::env::temp_dir().join("rust-image-viewer");
            if std::fs::create_dir_all(&log_dir).is_ok() {
                let _ = std::fs::write(log_dir.join("panic.log"), panic_report);
            }

            previous_hook(panic_info);
        }));
    });
}
