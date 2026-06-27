//! CLI commands for `elicit_doc`.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tracing::instrument;

use crate::collect::{
    DepBuildConfig, TypeFeatureProbe, collect_dep_serde_features, collect_elicit_complete_paths,
    collect_inventory, collect_inventory_with_json_path, collect_member_dep_build_config,
    collect_member_dep_inventory, collect_member_dep_inventory_with_json_path,
    collect_proof_harness, collect_trait_prereqs, collect_trait_prereqs_for_inventory,
    collect_trenchcoat_pairs,
};
use crate::error::ElicitDocResult;
use crate::gaps::{build_impl_gaps, build_shadow_gaps};
use crate::impl_coverage::{ImplCoverageReport, build_impl_coverage_report};
use crate::report::{
    write_impl_coverage_csv, write_impl_gaps_csv, write_shadow_csv, write_shadow_gaps_csv,
    write_trenchcoats_csv,
};
use crate::shadow::{ShadowReport, build_shadow_report};
use crate::summary::{ShadowSkippedPair, write_summary_md};
use crate::trenchcoat::{WrapperCoverageMap, build_trenchcoat_report, build_wrapper_coverage_map};

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
#[instrument]
pub fn run() -> ElicitDocResult<()> {
    let cli = Cli::parse();
    let own = own_root()?;
    let output_dir = cli.output_dir.unwrap_or_else(|| own.join("verif/coverage"));

    // Resolve elicitation workspace: explicit flag > env > sibling directory
    let elicitation_workspace = cli.workspace.unwrap_or_else(|| {
        own.join("../elicitation")
            .canonicalize()
            .unwrap_or_else(|_| own.join("../elicitation"))
    });

    let mp = MultiProgress::new();

    match &cli.command {
        Commands::Run { only, crate_name } => {
            let run_impls = matches!(only, None | Some(ReportKind::Impls));
            let run_shadows = matches!(only, None | Some(ReportKind::Shadows));

            let impl_data = if run_impls {
                Some(run_impl_reports(
                    &elicitation_workspace,
                    &output_dir,
                    crate_name.as_deref(),
                    &mp,
                )?)
            } else {
                None
            };
            let shadow_data = if run_shadows {
                Some(run_shadow_reports(
                    &elicitation_workspace,
                    &output_dir,
                    crate_name.as_deref(),
                    &mp,
                )?)
            } else {
                None
            };

            // Write the executive summary only when we have data from both halves
            // and no single-crate filter is active.
            if let (
                None,
                Some((impl_reports, impl_gaps, wrapper_coverage)),
                Some((shadow_reports, shadow_gaps, skipped_shadow_pairs)),
            ) = (crate_name.as_ref(), &impl_data, &shadow_data)
            {
                let summary_path = output_dir.join("summary.md");
                write_summary_md(
                    impl_reports,
                    impl_gaps,
                    Some(wrapper_coverage),
                    shadow_reports,
                    shadow_gaps,
                    skipped_shadow_pairs,
                    &summary_path,
                )?;
                mp.println(format!("wrote {}", summary_path.display())).ok();
            }
        }
    }

    Ok(())
}

/// Supported third-party crates: (upstream crate name, elicitation feature flag, fallback dep features).
///
/// The fallback dep feature list is only used when cargo metadata cannot resolve the
/// real dependency spec from the `elicitation` crate. These should be the minimal set
/// of serde/schema features needed to get a useful build in that degraded mode.
/// An empty slice means fall straight back to default features.
const THIRD_PARTY_CRATES: &[(&str, &str, &[&str])] = &[
    // Date/time
    ("chrono", "chrono", &["serde"]),
    (
        "time",
        "time",
        &["serde", "serde-human-readable", "serde-well-known"],
    ),
    ("jiff", "jiff", &["serde"]),
    // Identifiers / strings
    ("uuid", "uuid", &["serde"]),
    ("url", "url", &["serde"]),
    ("regex", "regex", &[]),
    // Serialization
    ("serde_json", "serde_json", &[]),
    ("toml", "toml-types", &["serde"]),
    // Geo / spatial
    ("geo-types", "geo-types", &["serde"]),
    ("geo", "geo", &["use-serde"]),
    ("geojson", "geojson-types", &[]),
    ("georaster", "georaster-types", &[]),
    ("rstar", "rstar-types", &["serde"]),
    ("proj", "proj-types", &["geo-types"]),
    ("wkt", "wkt-types", &["geo-types", "serde"]),
    ("wkb", "wkb-types", &[]),
    // Storage
    ("redb", "redb-types", &[]),
    ("csv", "csv-types", &[]),
    // Accessibility
    ("accesskit", "accesskit", &["serde", "schemars"]),
    // HTTP
    ("reqwest", "reqwest", &["json", "cookies", "stream"]),
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
    ("polars", "elicit_polars"), // excluded from workspace — skipped if unavailable
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
    ("surrealdb-types", "elicit_surrealdb"), // excluded from workspace — skipped if unavailable
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

/// Create a spinner added to `mp` with a consistent style.
fn make_spinner(mp: &MultiProgress, msg: impl Into<String>) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    let style = ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")
        .map(|style| style.tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]))
        .unwrap_or_else(|_| ProgressStyle::default_spinner());
    pb.set_style(style);
    pb.enable_steady_tick(Duration::from_millis(80));
    pb.set_message(msg.into());
    pb
}

/// Create an overall count bar added to `mp`.
fn make_count_bar(mp: &MultiProgress, total: u64, prefix: &'static str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(total));
    let style = ProgressStyle::with_template(
        "{prefix:.bold} [{bar:40.green/dim}] {pos}/{len}  {elapsed_precise}",
    )
    .map(|style| style.progress_chars("█▉▊▋▌▍▎▏ "))
    .unwrap_or_else(|_| ProgressStyle::default_bar());
    pb.set_style(style);
    pb.set_prefix(prefix);
    pb
}

type ImplReportsResult = ElicitDocResult<(
    Vec<(String, ImplCoverageReport)>,
    Vec<crate::gaps::ImplGapEntry>,
    WrapperCoverageMap,
)>;
type ShadowReportsResult = ElicitDocResult<(
    Vec<(String, String, ShadowReport)>,
    Vec<crate::gaps::ShadowGapEntry>,
    Vec<ShadowSkippedPair>,
)>;

fn resolve_dep_build_config(
    workspace: &std::path::Path,
    member_crate_name: &str,
    crate_name: &str,
    fallback_features: &[&str],
) -> DepBuildConfig {
    match collect_member_dep_build_config(workspace, member_crate_name, crate_name) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                member_crate_name,
                crate_name,
                error = %e,
                "falling back to baked-in dependency feature hints"
            );
            DepBuildConfig {
                activated_features: fallback_features.iter().map(|f| (*f).to_string()).collect(),
                uses_default_features: true,
            }
        }
    }
}

#[instrument(skip(workspace, inventory, activated_features, candidate_unlock_features), fields(source_crate = %inventory.crate_name, feature_crate = report_crate_name))]
fn build_type_feature_probes(
    workspace: &std::path::Path,
    report_crate_name: &str,
    inventory: &crate::inventory::Inventory,
    activated_features: &[String],
    candidate_unlock_features: &[String],
    uses_default_features: bool,
) -> HashMap<String, TypeFeatureProbe> {
    if candidate_unlock_features.is_empty() {
        return HashMap::new();
    }

    let mut probe_features: BTreeSet<String> = activated_features
        .iter()
        .filter(|feature| feature.as_str() != "default")
        .cloned()
        .collect();
    probe_features.extend(candidate_unlock_features.iter().cloned());
    let probe_owned: Vec<String> = probe_features.into_iter().collect();
    let probe_refs: Vec<&str> = probe_owned.iter().map(String::as_str).collect();

    let probed_map = match collect_member_dep_inventory_with_json_path(
        workspace,
        "elicitation",
        report_crate_name,
        &probe_refs,
        uses_default_features,
    ) {
        Ok((_probe_inventory, probe_json)) => {
            match collect_trait_prereqs_for_inventory(&probe_json, inventory) {
                Ok(prereqs) => Some(prereqs),
                Err(error) => {
                    tracing::warn!(
                        source_crate = inventory.crate_name,
                        feature_crate = report_crate_name,
                        error = %error,
                        "could not collect probed trait prereqs for surfaced API types"
                    );
                    None
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                source_crate = inventory.crate_name,
                feature_crate = report_crate_name,
                error = %error,
                "could not probe report-crate features for surfaced API types"
            );
            None
        }
    };

    let mut probes = HashMap::new();
    for item in inventory.type_items() {
        let type_path = item.path_str();
        let probed_prereqs = probed_map
            .as_ref()
            .and_then(|map| map.get(&type_path).cloned());

        probes.insert(
            type_path,
            TypeFeatureProbe {
                feature_crate: report_crate_name.to_string(),
                candidate_unlock_features: candidate_unlock_features.to_vec(),
                probed_prereqs,
            },
        );
    }

    probes
}

fn run_impl_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
    mp: &MultiProgress,
) -> ImplReportsResult {
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
    let spinner = make_spinner(mp, "building elicitation docs…");
    let (_elicitation, elicitation_json) =
        collect_inventory_with_json_path(workspace, "elicitation", &["full"])?;
    spinner.finish_with_message("✓ elicitation docs built");
    let complete_paths = collect_elicit_complete_paths(&elicitation_json, "elicitation")?;
    let trenchcoat_pairs = collect_trenchcoat_pairs(&elicitation_json)?;
    let wrapper_prereqs = collect_trait_prereqs(&elicitation_json, "elicitation", false)?;
    let wrapper_coverage =
        build_wrapper_coverage_map(&trenchcoat_pairs, &complete_paths, &wrapper_prereqs);

    // Accumulate all reports for gap analysis at the end.
    // Also accumulate a combined foreign-type prereq map for the trenchcoat report,
    // per-dep available serde features, and per-dep activated features for gap classification.
    let mut all_reports: Vec<(String, crate::impl_coverage::ImplCoverageReport)> = Vec::new();
    let mut combined_foreign_prereqs: std::collections::HashMap<
        String,
        crate::collect::TraitPrereqs,
    > = std::collections::HashMap::new();
    let mut type_feature_probes: std::collections::HashMap<
        String,
        std::collections::HashMap<String, TypeFeatureProbe>,
    > = std::collections::HashMap::new();

    let total_crates = if only_crate.is_none() {
        (THIRD_PARTY_CRATES.len() + 1) as u64
    } else {
        0
    };
    let overall = if total_crates > 0 {
        make_count_bar(mp, total_crates, "impl coverage")
    } else {
        mp.add(ProgressBar::hidden())
    };

    // Third-party crates — documented from their registry source
    for (crate_name, _feature, dep_features) in THIRD_PARTY_CRATES {
        if only_crate.is_none_or(|c| c == *crate_name) {
            let dep_config =
                resolve_dep_build_config(workspace, "elicitation", crate_name, dep_features);
            let activated_owned = dep_config.activated_features.clone();
            let activated_refs: Vec<&str> = activated_owned.iter().map(String::as_str).collect();
            let spinner = make_spinner(mp, format!("building docs: {crate_name}…"));
            let (source, dep_json) = match collect_member_dep_inventory_with_json_path(
                workspace,
                "elicitation",
                crate_name,
                &activated_refs,
                dep_config.uses_default_features,
            ) {
                Ok(bundle) => bundle,
                Err(e) => {
                    tracing::warn!(crate_name, error = %e, "skipping: dep inventory failed");
                    spinner.finish_with_message(format!("✗ {crate_name}: {e}"));
                    mp.println(format!("skipped {crate_name}: {e}")).ok();
                    continue;
                }
            };
            spinner.finish_with_message(format!("✓ {crate_name} docs built"));

            let mut prereqs = collect_trait_prereqs_for_inventory(&dep_json, &source)?;
            let elicit_prereqs = collect_trait_prereqs_for_inventory(&elicitation_json, &source)?;
            for (path, p) in elicit_prereqs {
                prereqs.entry(path).or_default().merge(&p);
            }
            let row_feature_probes = match collect_dep_serde_features(
                workspace,
                crate_name,
                &activated_refs,
                dep_config.uses_default_features,
            ) {
                Ok((available_features, expanded_activated_features)) => {
                    let expanded_activated: std::collections::HashSet<&str> =
                        expanded_activated_features
                            .iter()
                            .map(String::as_str)
                            .collect();
                    let candidate_unlock_features: Vec<String> = available_features
                        .into_iter()
                        .filter(|feature| !expanded_activated.contains(feature.as_str()))
                        .collect();
                    build_type_feature_probes(
                        workspace,
                        crate_name,
                        &source,
                        &activated_owned,
                        &candidate_unlock_features,
                        dep_config.uses_default_features,
                    )
                }
                Err(error) => {
                    tracing::warn!(
                        crate_name,
                        error = %error,
                        "could not collect target-crate feature unlock hints"
                    );
                    HashMap::new()
                }
            };

            // Accumulate into combined map for trenchcoat analysis
            for (path, p) in &prereqs {
                combined_foreign_prereqs
                    .entry(path.clone())
                    .or_default()
                    .merge(p);
            }
            let report = build_impl_coverage_report(&source, &complete_paths, &harness, &prereqs);
            let safe_name = crate_name.replace('-', "_");
            let path = output_dir.join(format!("{safe_name}.csv"));
            write_impl_coverage_csv(
                &report,
                Some(&row_feature_probes),
                Some(&wrapper_coverage),
                &path,
            )?;
            mp.println(format!("wrote {}  ({})", path.display(), report.summary()))
                .ok();
            type_feature_probes.insert(crate_name.to_string(), row_feature_probes);
            all_reports.push((crate_name.to_string(), report));
            overall.inc(1);
        }
    }

    // Internal elicitation types
    if only_crate.is_none_or(|c| c == "elicitation") {
        let source = collect_inventory(workspace, "elicitation", &["full"])?;
        let prereqs = collect_trait_prereqs(&elicitation_json, "elicitation", false)?;
        let report = build_impl_coverage_report(&source, &complete_paths, &harness, &prereqs);
        let path = output_dir.join("internal.csv");
        write_impl_coverage_csv(&report, None, Some(&wrapper_coverage), &path)?;
        mp.println(format!("wrote {}  ({})", path.display(), report.summary()))
            .ok();
        all_reports.push(("elicitation".to_string(), report));
        overall.inc(1);
    }

    overall.finish_and_clear();

    // Consolidated gaps report (only when we ran more than one crate or all crates)
    let impl_gaps = if only_crate.is_none() || all_reports.len() > 1 {
        let pairs: Vec<(&str, &crate::impl_coverage::ImplCoverageReport)> =
            all_reports.iter().map(|(n, r)| (n.as_str(), r)).collect();
        let gaps = build_impl_gaps(&pairs, &type_feature_probes, &wrapper_coverage);
        let gaps_path = output_dir.join("gaps-impl.csv");
        write_impl_gaps_csv(&gaps, &gaps_path)?;
        let missing_our = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ImplGapKind::MissingOurTraits)
            .count();
        let ready = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ImplGapKind::ReadyForElicitComplete)
            .count();
        let gated = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ImplGapKind::FeatureGatedExternal)
            .count();
        mp.println(format!(
            "wrote {}  ({} gaps: {} missing_our_traits, {} ready_for_elicit_complete, {} feature_gated_external)",
            gaps_path.display(), gaps.len(), missing_our, ready, gated
        )).ok();

        // Trenchcoat report (only on full runs)
        if only_crate.is_none() {
            let trenchcoats = build_trenchcoat_report(
                &trenchcoat_pairs,
                &complete_paths,
                &wrapper_prereqs,
                &combined_foreign_prereqs,
            );
            let tc_path = output_dir.join("trenchcoats.csv");
            write_trenchcoats_csv(&trenchcoats, &tc_path)?;
            let tc_complete = trenchcoats
                .iter()
                .filter(|e| e.wrapper_elicit_complete)
                .count();
            let tc_incomplete = trenchcoats
                .iter()
                .filter(|e| !e.wrapper_elicit_complete)
                .count();
            mp.println(format!(
                "wrote {}  ({} trenchcoats: {} complete, {} incomplete)",
                tc_path.display(),
                trenchcoats.len(),
                tc_complete,
                tc_incomplete
            ))
            .ok();
        }

        gaps
    } else {
        Vec::new()
    };

    Ok((all_reports, impl_gaps, wrapper_coverage))
}

fn run_shadow_reports(
    workspace: &std::path::Path,
    output_dir: &std::path::Path,
    only_crate: Option<&str>,
    mp: &MultiProgress,
) -> ShadowReportsResult {
    std::fs::create_dir_all(output_dir)
        .map_err(|e| crate::error::ElicitDocError::io(e.to_string()))?;

    let mut all_shadow: Vec<(String, String, crate::shadow::ShadowReport)> = Vec::new();
    let mut skipped_shadow_pairs: Vec<ShadowSkippedPair> = Vec::new();

    let total_pairs = if only_crate.is_none() {
        SHADOW_PAIRS.len() as u64
    } else {
        0
    };
    let overall = if total_pairs > 0 {
        make_count_bar(mp, total_pairs, "shadow coverage")
    } else {
        mp.add(ProgressBar::hidden())
    };

    for (target, shadow) in SHADOW_PAIRS {
        if only_crate.is_none_or(|c| c == *target || c == *shadow) {
            let dep_config = resolve_dep_build_config(workspace, shadow, target, &[]);
            let activated_refs: Vec<&str> = dep_config
                .activated_features
                .iter()
                .map(String::as_str)
                .collect();
            let spinner = make_spinner(mp, format!("building docs: {target} → {shadow}…"));
            let target_inv = match collect_member_dep_inventory(
                workspace,
                shadow,
                target,
                &activated_refs,
                dep_config.uses_default_features,
            ) {
                Ok(inv) => inv,
                Err(e) => {
                    tracing::warn!(target, error = %e, "skipping shadow pair: upstream dep inventory failed");
                    spinner.finish_with_message(format!("✗ {target}: {e}"));
                    mp.println(format!("skipped {target} → {shadow}: {e}")).ok();
                    skipped_shadow_pairs.push(ShadowSkippedPair {
                        upstream_crate: (*target).to_string(),
                        shadow_crate: (*shadow).to_string(),
                        error: e.to_string(),
                    });
                    continue;
                }
            };
            let (shadow_inv, shadow_json) = match collect_inventory_with_json_path(
                workspace,
                shadow,
                &[],
            ) {
                Ok(bundle) => bundle,
                Err(e) => {
                    tracing::warn!(shadow, error = %e, "skipping shadow pair: shadow inventory failed");
                    spinner.finish_with_message(format!("✗ {shadow}: {e}"));
                    mp.println(format!("skipped {target} → {shadow}: {e}")).ok();
                    skipped_shadow_pairs.push(ShadowSkippedPair {
                        upstream_crate: (*target).to_string(),
                        shadow_crate: (*shadow).to_string(),
                        error: e.to_string(),
                    });
                    continue;
                }
            };
            spinner.finish_with_message(format!("✓ {target} → {shadow} docs built"));
            let shadow_complete = collect_elicit_complete_paths(&shadow_json, shadow)?;
            let shadow_prereqs = collect_trait_prereqs(&shadow_json, shadow, false)?;
            let report =
                build_shadow_report(&target_inv, &shadow_inv, &shadow_complete, &shadow_prereqs);
            let path = output_dir.join(format!("shadow-{target}.csv"));
            write_shadow_csv(&report, &path)?;
            mp.println(format!("wrote {}  ({})", path.display(), report.summary()))
                .ok();
            all_shadow.push((target.to_string(), shadow.to_string(), report));
            overall.inc(1);
        }
    }

    overall.finish_and_clear();

    // Consolidated shadow gaps report
    let shadow_gaps = if only_crate.is_none() || all_shadow.len() > 1 {
        let pairs: Vec<(&str, &str, &crate::shadow::ShadowReport)> = all_shadow
            .iter()
            .map(|(t, s, r)| (t.as_str(), s.as_str(), r))
            .collect();
        let gaps = build_shadow_gaps(&pairs)?;
        let gaps_path = output_dir.join("gaps-shadow.csv");
        write_shadow_gaps_csv(&gaps, &gaps_path)?;
        let missing = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::Missing)
            .count();
        let drifted = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::Drifted)
            .count();
        let stale = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::PossiblyStale)
            .count();
        let infra = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::InfrastructureExtra)
            .count();
        let verification = gaps
            .iter()
            .filter(|e| e.gap_kind == crate::gaps::ShadowGapKind::ShadowVerificationGap)
            .count();
        mp.println(format!(
            "wrote {}  ({} total: {} missing, {} drifted, {} possibly_stale, {} infra_extra, {} verification)",
            gaps_path.display(), gaps.len(), missing, drifted, stale, infra, verification
        )).ok();
        gaps
    } else {
        Vec::new()
    };
    Ok((all_shadow, shadow_gaps, skipped_shadow_pairs))
}
