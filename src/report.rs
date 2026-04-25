//! CSV report serialization.

use std::path::Path;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::impl_coverage::ImplCoverageReport;
use crate::shadow::ShadowReport;

/// Convert a bool trait presence + build flag into a three-value availability string.
///
/// - `"present"` — impl was found in the dep's rustdoc JSON
/// - `"absent"` — `--all-features` build succeeded but no impl found (truly missing from dep)
/// - `"feature_gated"` — build fell back to default features; impl *may* exist behind a flag
fn trait_avail(present: bool, all_features: bool) -> &'static str {
    if present {
        "present"
    } else if all_features {
        "absent"
    } else {
        "feature_gated"
    }
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
        let avail = |present: bool| trait_avail(present, entry.all_features_build);
        let blockers = p.external_blockers_with_avail(entry.all_features_build).join(";");
        wtr.write_record([
            &entry.type_path,
            &entry.type_kind.to_string(),
            &entry.is_generic.to_string(),
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
            avail(p.can_be_direct()),
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
            &row.notes,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = report.rows.len(), "wrote shadow CSV");
    Ok(())
}
