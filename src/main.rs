//! CLI entry point for `elicit_doc`.

#[cfg(feature = "cli")]
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

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!("elicit_doc was built without the 'cli' feature.");
    eprintln!("Rebuild with: cargo build --features cli");
    std::process::exit(1);
}
