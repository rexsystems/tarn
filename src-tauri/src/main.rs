#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // GlitchTip (Sentry-compatible). Keep the guard for the whole process so
    // events flush on exit; panics are captured by the panic integration.
    let _guard = sentry::init(
        sentry::ClientOptions::new()
            .dsn("https://f37a2e88dde34be7a9d494592639b5f3@error.mythical.systems/7")
            .release(sentry::release_name!().unwrap_or_else(|| "tarn@unknown".into()))
            .environment(if cfg!(debug_assertions) {
                "development"
            } else {
                "production"
            })
            .traces_sample_rate(0.01)
            .debug(cfg!(debug_assertions)),
    );

    #[cfg(debug_assertions)]
    {
        sentry::configure_scope(|scope| {
            scope.set_tag("app", "tarn");
            scope.set_tag("mode", "dev");
        });
        // One-shot so you can confirm the project receives events.
        sentry::capture_message("Tarn GlitchTip connected (dev)", sentry::Level::Info);
    }

    tarn_lib::run()
}
