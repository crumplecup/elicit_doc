//! CLI commands for `elicit_doc`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::collect::{collect_inventory, collect_proof_harness};
use crate::error::ElicitDocResult;
use crate::impl_coverage::build_impl_coverage_report;
use crate::report::{write_impl_coverage_csv, write_shadow_csv};
use crate::shadow::build_shadow_report;

/// Determine elicit_doc's own repo root via `cargo metadata`.
fn own_root() -> ElicitDocResult<PathBuf> {
    let meta = cargo_metadata::MetadataCommand::new()
        .exec()
        .map_err(|e| crate::error::ElicitDocError::cargo_metadata(e.to_string()))?;
    Ok(meta.workspace_root.into())
}

#[derive(Parser)]
#[command(
    name = "elicit_doc",
    about = "Coverage and drift analysis for elicitation"
)]
struct Cli {
    /// Path to the elicitation workspace root.
    ///
    /// Defaults to the ELICITATION_WORKSPACE env var, then
    /// `../elicitation` relative to this repo.
    #[arg(long, env = "ELICITATION_WORKSPACE")]
    workspace: Option<PathBuf>,

    /// Output directory for CSV reports (default: verif/coverage/ inside elicit_doc).
    #[arg(long)]
    output_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run all coverage reports.
    Run {
        /// Restrict to a specific report type.
        #[arg(long, value_enum)]
        only: Option<ReportKind>,
        /// Run only for a specific third-party crate name.
        #[arg(long)]
        crate_name: Option<String>,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum ReportKind {
    Impls,
    Shadows,
}

/// Entry point called from `main.rs`.
pub fn run() -> ElicitDocResult<()> {
    let cli = Cli::parse();
    let own = own_root()?;
    let output_dir = cli
        .output_dir
        .unwrap_or_else(|| own.join("verif/coverage"));

    // Resolve elicitation workspace: explicit flag > env > sibling directory
    let elicitation_workspace = cli.workspace.unwrap_or_else(|| {
        own.join("../elicitation")
            .canonicalize()
            .unwrap_or_else(|_| own.join("../elicitation"))
    });

    match &cli.command {
        Commands::Run { only, crate_name } => {
            let run_impls = matches!(only, None | Some(ReportKind::Impls));
            let run_shadows = matches!(only, None | Some(ReportKind::Shadows));

            if run_impls {
                run_impl_reports(&elicitation_workspace, &output_dir, crate_name.as_deref())?;
            }
            if run_shadows {
                run_shadow_reports(&elicitation_workspace, &output_dir, crate_name.as_deref())?;
            }
        }
    }

    Ok(())
}

/// Supported third-party crates with their elicitation feature flag names.
const THIRD_PARTY_CRATES: &[(&str, &str)] = &[
    ("uuid", "uuid"),
    ("url", "url"),
    ("geo", "geo-types"),
    ("geojson", "geojson"),
    ("chrono", "chrono"),
    ("serde_json", "serde_json"),
];

/// Shadow crate pairs: (target_crate, shadow_crate, feature_flag).
const SHADOW_PAIRS: &[(&str, &str, &str)] = &[
    ("bevy", "elicit_bevy", "bevy"),
    ("wgpu", "elicit_wgpu", "wgpu"),
    ("egui", "elicit_egui", "egui"),
    ("winit", "elicit_winit", "winit"),
    ("ratatui", "elicit_ratatui", "ratatui"),
];

fn run_impl_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
) -> ElicitDocResult<()> {
    // Scan proof harnesses from elicitation crate
    let harness_non_empty = workspace.join("crates/elicitation/tests/proof_non_empty_test.rs");
    let harness_composition = workspace.join("crates/elicitation/tests/proof_composition_test.rs");

    let mut harness = collect_proof_harness(&harness_non_empty)?;
    if harness_composition.exists() {
        let comp = collect_proof_harness(&harness_composition)?;
        harness.non_empty_types.extend(comp.non_empty_types);
        harness.composition_pairs.extend(comp.composition_pairs);
    }

    // Collect elicitation inventory once
    let elicitation = collect_inventory(workspace, "elicitation", &["full"])?;

    // std
    if only_crate.is_none_or(|c| c == "std") {
        let std_inv = collect_inventory(workspace, "std", &[])?;
        let report = build_impl_coverage_report(&std_inv, &elicitation, &harness);
        let path = output_dir.join("std.csv");
        write_impl_coverage_csv(&report, &path)?;
        println!("wrote {}  ({})", path.display(), report.summary());
    }

    // Third-party crates
    for (crate_name, feature) in THIRD_PARTY_CRATES {
        if only_crate.is_none_or(|c| c == *crate_name) {
            let source = collect_inventory(workspace, crate_name, &[feature])?;
            let report = build_impl_coverage_report(&source, &elicitation, &harness);
            let path = output_dir.join(format!("{crate_name}.csv"));
            write_impl_coverage_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
        }
    }

    // Internal elicitation types
    if only_crate.is_none_or(|c| c == "elicitation") {
        let report = build_impl_coverage_report(&elicitation, &elicitation, &harness);
        let path = output_dir.join("internal.csv");
        write_impl_coverage_csv(&report, &path)?;
        println!("wrote {}  ({})", path.display(), report.summary());
    }

    Ok(())
}

fn run_shadow_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
) -> ElicitDocResult<()> {
    for (target, shadow, feature) in SHADOW_PAIRS {
        if only_crate.is_none_or(|c| c == *target || c == *shadow) {
            let target_inv = collect_inventory(workspace, target, &[feature])?;
            let shadow_inv = collect_inventory(workspace, shadow, &[])?;
            let report = build_shadow_report(&target_inv, &shadow_inv);
            let path = output_dir.join(format!("shadow-{target}.csv"));
            write_shadow_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
        }
    }
    Ok(())
}
