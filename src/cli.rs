//! CLI commands for `elicit_doc`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::collect::{collect_dep_inventory, collect_elicit_complete_paths, collect_inventory, collect_proof_harness, collect_trait_prereqs};
use crate::error::ElicitDocResult;
use crate::gaps::{build_impl_gaps, build_shadow_gaps};
use crate::impl_coverage::{ImplCoverageReport, build_impl_coverage_report};
use crate::report::{write_impl_coverage_csv, write_impl_gaps_csv, write_shadow_csv, write_shadow_gaps_csv};
use crate::shadow::{ShadowReport, build_shadow_report};
use crate::summary::write_summary_md;

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

            let impl_data = if run_impls {
                Some(run_impl_reports(&elicitation_workspace, &output_dir, crate_name.as_deref())?)
            } else {
                None
            };
            let shadow_data = if run_shadows {
                Some(run_shadow_reports(&elicitation_workspace, &output_dir, crate_name.as_deref())?)
            } else {
                None
            };

            // Write the executive summary only when we have data from both halves
            // and no single-crate filter is active.
            if crate_name.is_none() {
                if let (Some((impl_reports, impl_gaps)), Some((shadow_reports, shadow_gaps))) =
                    (&impl_data, &shadow_data)
                {
                    let summary_path = output_dir.join("summary.md");
                    write_summary_md(impl_reports, impl_gaps, shadow_reports, shadow_gaps, &summary_path)?;
                    println!("wrote {}", summary_path.display());
                }
            }
        }
    }

    Ok(())
}

/// Supported third-party crates: (upstream crate name, elicitation feature flag).
///
/// These are external deps with value/data types that could be `ElicitComplete`.
/// Use `collect_dep_inventory` to document them.
const THIRD_PARTY_CRATES: &[(&str, &str)] = &[
    // Date/time
    ("chrono", "chrono"),
    ("time", "time"),
    ("jiff", "jiff"),
    // Identifiers / strings
    ("uuid", "uuid"),
    ("url", "url"),
    ("regex", "regex"),
    // Serialization
    ("serde_json", "serde_json"),
    ("toml", "toml-types"),
    // Geo / spatial
    ("geo-types", "geo-types"),
    ("geo", "geo"),
    ("geojson", "geojson-types"),
    ("georaster", "georaster-types"),
    ("rstar", "rstar-types"),
    ("proj", "proj-types"),
    ("wkt", "wkt-types"),
    ("wkb", "wkb-types"),
    // Storage
    ("redb", "redb-types"),
    ("csv", "csv-types"),
    // Accessibility
    ("accesskit", "accesskit"),
    // HTTP
    ("reqwest", "reqwest"),
];

/// Shadow crate pairs: (upstream dep name, workspace member shadow name).
///
/// upstream → `collect_dep_inventory`, shadow → `collect_inventory`.
/// Crates excluded from the elicitation workspace (polars, surrealdb) are still
/// listed; the report loop skips them gracefully if they fail to build.
const SHADOW_PAIRS: &[(&str, &str)] = &[
    // Graphics / rendering
    ("bevy", "elicit_bevy"),
    ("wgpu", "elicit_wgpu"),
    ("egui", "elicit_egui"),
    ("winit", "elicit_winit"),
    // TUI
    ("ratatui", "elicit_ratatui"),
    // Async / networking
    ("tokio", "elicit_tokio"),
    ("tower", "elicit_tower"),
    ("axum", "elicit_axum"),
    ("reqwest", "elicit_reqwest"),
    // Data
    ("polars", "elicit_polars"),   // excluded from workspace — skipped if unavailable
    // CLI
    ("clap", "elicit_clap"),
    // Reactive / web
    ("leptos", "elicit_leptos"),
    // Serialization
    ("serde", "elicit_serde"),
    ("serde_json", "elicit_serde_json"),
    ("toml", "elicit_toml"),
    ("csv", "elicit_csv"),
    // Date/time
    ("chrono", "elicit_chrono"),
    ("time", "elicit_time"),
    ("jiff", "elicit_jiff"),
    // Identifiers / strings
    ("uuid", "elicit_uuid"),
    ("url", "elicit_url"),
    ("regex", "elicit_regex"),
    // Database
    ("sqlx", "elicit_sqlx"),
    ("redb", "elicit_redb"),
    ("surrealdb-types", "elicit_surrealdb"),  // excluded from workspace — skipped if unavailable
    // Geo / spatial
    ("geo-types", "elicit_geo_types"),
    ("geo", "elicit_geo"),
    ("geojson", "elicit_geojson"),
    ("georaster", "elicit_georaster"),
    ("rstar", "elicit_rstar"),
    ("proj", "elicit_proj"),
    ("wkt", "elicit_wkt"),
    ("wkb", "elicit_wkb"),
    // Units
    ("uom", "elicit_uom"),
    // Accessibility
    ("accesskit", "elicit_accesskit"),
];

fn run_impl_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
) -> ElicitDocResult<(Vec<(String, ImplCoverageReport)>, Vec<crate::gaps::ImplGapEntry>)> {
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

    // Accumulate all reports for gap analysis at the end.
    let mut all_reports: Vec<(String, crate::impl_coverage::ImplCoverageReport)> = Vec::new();

    // Third-party crates — documented from their registry source
    for (crate_name, _feature) in THIRD_PARTY_CRATES {
        if only_crate.is_none_or(|c| c == *crate_name) {
            let (source, all_features) = match collect_dep_inventory(workspace, crate_name) {
                Ok(pair) => pair,
                Err(e) => {
                    tracing::warn!(crate_name, error = %e, "skipping: dep inventory failed");
                    println!("skipped {crate_name}: {e}");
                    continue;
                }
            };
            let safe_name = crate_name.replace('-', "_");
            let dep_json = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(format!("target/doc/{safe_name}.json"));
            let mut prereqs = if dep_json.exists() {
                collect_trait_prereqs(&dep_json, crate_name, true)?
            } else {
                std::collections::HashMap::new()
            };
            let elicit_prereqs = collect_trait_prereqs(&elicitation_json, crate_name, true)?;
            for (path, p) in elicit_prereqs {
                prereqs.entry(path).or_default().merge(&p);
            }
            let report = build_impl_coverage_report(&source, &complete_paths, &harness, &prereqs, all_features);
            let path = output_dir.join(format!("{safe_name}.csv"));
            write_impl_coverage_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
            all_reports.push((crate_name.to_string(), report));
        }
    }

    // Internal elicitation types
    if only_crate.is_none_or(|c| c == "elicitation") {
        let source = collect_inventory(workspace, "elicitation", &["full"])?;
        let prereqs = collect_trait_prereqs(&elicitation_json, "elicitation", false)?;
        let report = build_impl_coverage_report(&source, &complete_paths, &harness, &prereqs, true);
        let path = output_dir.join("internal.csv");
        write_impl_coverage_csv(&report, &path)?;
        println!("wrote {}  ({})", path.display(), report.summary());
        all_reports.push(("elicitation".to_string(), report));
    }

    // Consolidated gaps report (only when we ran more than one crate or all crates)
    let impl_gaps = if only_crate.is_none() || all_reports.len() > 1 {
        let pairs: Vec<(&str, &crate::impl_coverage::ImplCoverageReport)> =
            all_reports.iter().map(|(n, r)| (n.as_str(), r)).collect();
        let gaps = build_impl_gaps(&pairs);
        let gaps_path = output_dir.join("gaps-impl.csv");
        write_impl_gaps_csv(&gaps, &gaps_path)?;
        let ready = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ImplGapKind::ReadyNow).count();
        let gated = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ImplGapKind::FeatureGated).count();
        let needs = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ImplGapKind::NeedsExternalImpl).count();
        println!(
            "wrote {}  ({} gaps: {} ready_now, {} feature_gated, {} needs_external_impl)",
            gaps_path.display(), gaps.len(), ready, gated, needs
        );
        gaps
    } else {
        Vec::new()
    };

    Ok((all_reports, impl_gaps))
}

fn run_shadow_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
) -> ElicitDocResult<(Vec<(String, String, ShadowReport)>, Vec<crate::gaps::ShadowGapEntry>)> {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| crate::error::ElicitDocError::io(e.to_string()))?;

    let mut all_shadow: Vec<(String, String, crate::shadow::ShadowReport)> = Vec::new();

    for (target, shadow) in SHADOW_PAIRS {
        if only_crate.is_none_or(|c| c == *target || c == *shadow) {
            let target_inv = match collect_dep_inventory(workspace, target) {
                Ok((inv, _)) => inv,
                Err(e) => {
                    tracing::warn!(target, error = %e, "skipping shadow pair: upstream dep inventory failed");
                    println!("skipped {target} → {shadow}: {e}");
                    continue;
                }
            };
            let shadow_inv = match collect_inventory(workspace, shadow, &[]) {
                Ok(inv) => inv,
                Err(e) => {
                    tracing::warn!(shadow, error = %e, "skipping shadow pair: shadow inventory failed");
                    println!("skipped {target} → {shadow}: {e}");
                    continue;
                }
            };
            let report = build_shadow_report(&target_inv, &shadow_inv);
            let path = output_dir.join(format!("shadow-{target}.csv"));
            write_shadow_csv(&report, &path)?;
            println!("wrote {}  ({})", path.display(), report.summary());
            all_shadow.push((target.to_string(), shadow.to_string(), report));
        }
    }

    // Consolidated shadow gaps report
    let shadow_gaps = if only_crate.is_none() || all_shadow.len() > 1 {
        let pairs: Vec<(&str, &str, &crate::shadow::ShadowReport)> =
            all_shadow.iter().map(|(t, s, r)| (t.as_str(), s.as_str(), r)).collect();
        let gaps = build_shadow_gaps(&pairs);
        let gaps_path = output_dir.join("gaps-shadow.csv");
        write_shadow_gaps_csv(&gaps, &gaps_path)?;
        let missing = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::Missing).count();
        let drifted = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::Drifted).count();
        let stale = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::PossiblyStale).count();
        let infra = gaps.iter().filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::InfrastructureExtra).count();
        println!(
            "wrote {}  ({} total: {} missing, {} drifted, {} possibly_stale, {} infra_extra)",
            gaps_path.display(), gaps.len(), missing, drifted, stale, infra
        );
        gaps
    } else {
        Vec::new()
    };
    Ok((all_shadow, shadow_gaps))
}
