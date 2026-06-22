//! CSV report serialization.

use std::path::Path;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::impl_coverage::ImplCoverageReport;
use crate::shadow::ShadowReport;

/// Convert a bool trait presence into a two-value availability string.
///
/// - `"present"` — impl was found in the dep's rustdoc JSON
/// - `"absent"` — no impl found (dep was built with its activated features)
fn trait_avail(present: bool) -> &'static str {
    if present { "present" } else { "absent" }
}

/// Write an [`ImplCoverageReport`] to a CSV file at `path`.
#[instrument(skip(report, path), fields(path = %path.display()))]
pub fn write_impl_coverage_csv(report: &ImplCoverageReport, path: &Path) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "type_path",
        "type_kind",
        "is_generic",
        "lifetime_params",
        "type_params",
        "elicit_impl",
        "proof_test",
        "composition_test",
        // ElicitComplete supertrait prereqs (present / absent / feature_gated)
        "has_serialize",
        "has_deserialize",
        "has_json_schema",
        "has_elicitation",
        "has_elicit_introspect",
        "has_elicit_spec",
        "has_elicit_prompt_tree",
        "has_to_code_literal",
        "can_be_direct",
        "external_blockers",
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for entry in &report.entries {
        let p = &entry.prereqs;
        let avail = |present: bool| trait_avail(present);
        let blockers = p.external_blockers_absent().join(";");
        wtr.write_record([
            &entry.type_path,
            &entry.type_kind.to_string(),
            &entry.is_generic.to_string(),
            &entry.lifetime_params.join(";"),
            &entry.type_params.join(";"),
            &entry.elicit_impl.to_string(),
            &entry.proof_test.to_string(),
            &entry.composition_test.to_string(),
            avail(p.serialize),
            avail(p.deserialize),
            avail(p.json_schema),
            avail(p.elicitation_trait),
            avail(p.elicit_introspect),
            avail(p.elicit_spec),
            avail(p.elicit_prompt_tree),
            avail(p.to_code_literal),
            avail(entry.can_be_direct()),
            &blockers,
            &entry.notes,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = report.entries.len(), "wrote impl coverage CSV");
    Ok(())
}

/// Write a [`ShadowReport`] to a CSV file at `path`.
#[instrument(skip(report, path), fields(path = %path.display()))]
pub fn write_shadow_csv(report: &ShadowReport, path: &Path) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "item_path",
        "item_kind",
        "status",
        "shadow_item",
        "drift_confidence",
        "shadow_elicit_impl",
        "shadow_can_be_direct",
        "shadow_missing_external_traits",
        "shadow_missing_our_traits",
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for row in &report.rows {
        wtr.write_record([
            &row.item_path,
            &row.item_kind.to_string(),
            &row.status.to_string(),
            &row.shadow_item,
            &row.drift_confidence,
            &row.shadow_elicit_impl,
            &row.shadow_can_be_direct,
            &row.shadow_missing_external_traits,
            &row.shadow_missing_our_traits,
            &row.notes,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = report.rows.len(), "wrote shadow CSV");
    Ok(())
}

/// Write the consolidated impl gaps list to a CSV file at `path`.
#[instrument(skip(entries, path), fields(path = %path.display()))]
pub fn write_impl_gaps_csv(
    entries: &[crate::gaps::ImplGapEntry],
    path: &Path,
) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "source_crate",
        "type_path",
        "type_kind",
        "is_generic",
        "gap_kind",
        "our_traits_complete",
        "can_be_direct",
        "elicit_complete_gap",
        "feature_gated_external",
        "blocked_by_orphan_rule",
        "missing_external_traits",
        "missing_our_traits",
        "candidate_unlock_features",
        "action",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for e in entries {
        wtr.write_record([
            &e.source_crate,
            &e.type_path,
            &e.type_kind.to_string(),
            &e.is_generic.to_string(),
            &e.gap_kind.to_string(),
            &e.our_traits_complete.to_string(),
            &e.can_be_direct.to_string(),
            &e.elicit_complete_gap.to_string(),
            &e.feature_gated_external.to_string(),
            &e.blocked_by_orphan_rule.to_string(),
            &e.missing_external_traits,
            &e.missing_our_traits,
            &e.candidate_unlock_features,
            &e.action,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = entries.len(), "wrote impl gaps CSV");
    Ok(())
}

/// Write the consolidated shadow gaps list to a CSV file at `path`.
#[instrument(skip(entries, path), fields(path = %path.display()))]
pub fn write_shadow_gaps_csv(
    entries: &[crate::gaps::ShadowGapEntry],
    path: &Path,
) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "target_crate",
        "shadow_crate",
        "item_path",
        "item_kind",
        "gap_kind",
        "matched_shadow_item",
        "drift_confidence",
        "shadow_elicit_impl",
        "shadow_can_be_direct",
        "shadow_missing_external_traits",
        "shadow_missing_our_traits",
        "action",
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for e in entries {
        wtr.write_record([
            &e.target_crate,
            &e.shadow_crate,
            &e.item_path,
            &e.item_kind.to_string(),
            &e.gap_kind.to_string(),
            &e.matched_shadow_item,
            &e.drift_confidence,
            &e.shadow_elicit_impl,
            &e.shadow_can_be_direct,
            &e.shadow_missing_external_traits,
            &e.shadow_missing_our_traits,
            &e.action,
            &e.notes,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = entries.len(), "wrote shadow gaps CSV");
    Ok(())
}

/// Write the trenchcoat report to a CSV file at `path`.
#[instrument(skip(entries, path), fields(path = %path.display()))]
pub fn write_trenchcoats_csv(
    entries: &[crate::trenchcoat::TrenchcoatEntry],
    path: &Path,
) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "foreign_crate",
        "foreign_type",
        "wrapper_path",
        "wrapper_elicit_complete",
        "wrapper_missing_our_traits",
        "foreign_missing_our_traits",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for e in entries {
        wtr.write_record([
            &e.foreign_crate,
            &e.foreign_type,
            &e.wrapper_path,
            &e.wrapper_elicit_complete.to_string(),
            &e.wrapper_missing_our_traits,
            &e.foreign_missing_our_traits,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = entries.len(), "wrote trenchcoats CSV");
    Ok(())
}
