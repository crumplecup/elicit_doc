//! CSV report serialization.

use std::path::Path;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::impl_coverage::ImplCoverageReport;
use crate::shadow::ShadowReport;

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
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    for entry in &report.entries {
        wtr.write_record([
            &entry.type_path,
            &entry.type_kind.to_string(),
            &entry.is_generic.to_string(),
            &entry.type_params.join(";"),
            &entry.elicit_impl.to_string(),
            &entry.proof_test.to_string(),
            &entry.composition_test.to_string(),
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
