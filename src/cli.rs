//! CLI commands for `elicit_doc`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::collect::{collect_dep_inventory, collect_elicit_complete_paths, collect_inventory, collect_proof_harness};
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

/// Supported third-party crates: (upstream crate name, elicitation feature flag).
/// These are external deps — use `collect_dep_inventory` to document them.
const THIRD_PARTY_CRATES: &[(&str, &str)] = &[
    ("uuid", "uuid"),
    ("url", "url"),
    ("geo-types", "geo-types"),
    ("geojson", "geojson"),
    ("chrono", "chrono"),
    ("serde_json", "serde_json"),
];

/// Shadow crate pairs: (upstream dep name, workspace member shadow name).
/// upstream → `collect_dep_inventory`, shadow → `collect_inventory`
const SHADOW_PAIRS: &[(&str, &str)] = &[
    ("bevy", "elicit_bevy"),
    ("wgpu", "elicit_wgpu"),
    ("egui", "elicit_egui"),
    ("winit", "elicit_winit"),
    ("ratatui", "elicit_ratatui"),
];

fn run_impl_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
) -> ElicitDocResult<()> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| crate::error::ElicitDocError::io(e.to_string()))?;

    // Scan proof harnesses from elicitation crate
    let harness_non_empty = workspace.join("crates/elicitation/tests/proof_non_empty_test.rs");
    let harness_composition = workspace.join("crates/elicitation/tests/proof_composition_test.rs");

    let mut harness = collect_proof_harness(&harness_non_empty)?;
    if harness_composition.exists() {
        let comp = collect_proof_harness(&harness_composition)?;
        harness.non_empty_types.extend(comp.non_empty_types);
        harness.composition_pairs.extend(comp.composition_pairs);
    }

    // Collect elicitation inventory once (with full feature set), then extract
    // the real ElicitComplete impl set directly from its rustdoc JSON.
    let _elicitation = collect_inventory(workspace, "elicitation", &["full"])?;
    let elicitation_json = workspace.join("target/doc/elicitation.json");
    let complete_paths = collect_elicit_complete_paths(&elicitation_json)?;

    // Third-party crates — documented from their registry source
    for (crate_name, _feature) in THIRD_PARTY_CRATES {
        if only_crate.is_none_or(|c| c == *crate_name) {
            let source = collect_dep_inventory(workspace, crate_name)?;
            let report = build_impl_coverage_report(&source, &complete_paths, &harness);
            let safe_name = crate_name.replace('-', "_");
            let path = output_dir.join(format!("{safe_name}.csv"));
            write_impl_coverage_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
        }
    }

    // Internal elicitation types
    if only_crate.is_none_or(|c| c == "elicitation") {
        let source = collect_inventory(workspace, "elicitation", &["full"])?;
        let report = build_impl_coverage_report(&source, &complete_paths, &harness);
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
    std::fs::create_dir_all(output_dir)
        .map_err(|e| crate::error::ElicitDocError::io(e.to_string()))?;

    for (target, shadow) in SHADOW_PAIRS {
        if only_crate.is_none_or(|c| c == *target || c == *shadow) {
            let target_inv = collect_dep_inventory(workspace, target)?;
            let shadow_inv = collect_inventory(workspace, shadow, &[])?;
            let report = build_shadow_report(&target_inv, &shadow_inv);
            let path = output_dir.join(format!("shadow-{target}.csv"));
            write_shadow_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
        }
    }
    Ok(())
}
