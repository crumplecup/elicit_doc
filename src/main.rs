//! CLI entry point for `elicit_doc`.

fn main() {
    use elicit_doc::cli::run;
    use tracing_subscriber::filter::LevelFilter;

    let env_filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing_subscriber::fmt().with_env_filter(env_filter).init();
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
