//! CLI entry point for `elicit_doc`.

fn main() {
    use elicit_doc::cli::run;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("elicit_doc=info".parse().unwrap()),
        )
        .init();
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
