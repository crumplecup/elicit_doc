//! CSV report serialization.

use std::collections::HashMap;
use std::path::Path;

use tracing::instrument;

use crate::collect::TypeFeatureProbe;
use crate::error::{ElicitDocError, ElicitDocResult};
use crate::gaps::{
    ImplGapKind, ImplRowKind, ShadowCoverageKind, assess_impl_entry, assess_shadow_row,
};
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
pub fn write_impl_coverage_csv(
    report: &ImplCoverageReport,
    type_feature_probes: Option<&HashMap<String, TypeFeatureProbe>>,
    path: &Path,
) -> ElicitDocResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ElicitDocError::io(e.to_string()))?;
    }

    let mut wtr = csv::Writer::from_path(path).map_err(|e| ElicitDocError::csv(e.to_string()))?;

    wtr.write_record([
        "type_path",
        "type_kind",
        "api_family",
        "is_generic",
        "lifetime_params",
        "type_params",
        "elicit_impl",
        "proof_test",
        "composition_test",
        "row_kind",
        "primary_gap_kind",
        "our_traits_complete",
        "feature_unlock_possible",
        "feature_owner_crate",
        "blocked_reason",
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
        "missing_external_traits",
        "missing_our_traits",
        "candidate_unlock_features",
        "action",
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    let mut rows: Vec<_> = report
        .entries
        .iter()
        .map(|entry| {
            let assessment = assess_impl_entry(
                &report.source_crate,
                entry,
                type_feature_probes.and_then(|m| m.get(&entry.type_path)),
            );
            (entry, assessment)
        })
        .collect();
    rows.sort_by(
        |(left_entry, left_assessment), (right_entry, right_assessment)| {
            let left_family = api_family(&left_entry.type_path);
            let right_family = api_family(&right_entry.type_path);
            impl_row_kind_order(&left_assessment.row_kind)
                .cmp(&impl_row_kind_order(&right_assessment.row_kind))
                .then(left_family.cmp(&right_family))
                .then(left_entry.type_path.cmp(&right_entry.type_path))
        },
    );

    for (entry, assessment) in rows {
        let p = &entry.prereqs;
        let avail = |present: bool| trait_avail(present);
        wtr.write_record([
            &entry.type_path,
            &entry.type_kind.to_string(),
            &api_family(&entry.type_path),
            &entry.is_generic.to_string(),
            &entry.lifetime_params.join(";"),
            &entry.type_params.join(";"),
            &entry.elicit_impl.to_string(),
            &entry.proof_test.to_string(),
            &entry.composition_test.to_string(),
            &assessment.row_kind.to_string(),
            &assessment
                .primary_gap_kind
                .as_ref()
                .map(ImplGapKind::to_string)
                .unwrap_or_default(),
            &assessment.our_traits_complete.to_string(),
            &assessment.feature_gated_external.to_string(),
            &assessment.feature_owner_crate,
            &assessment.blocked_reason,
            avail(p.serialize),
            avail(p.deserialize),
            avail(p.json_schema),
            avail(p.elicitation_trait),
            avail(p.elicit_introspect),
            avail(p.elicit_spec),
            avail(p.elicit_prompt_tree),
            avail(p.to_code_literal),
            &assessment.can_be_direct.to_string(),
            &assessment.missing_external_traits,
            &assessment.missing_our_traits,
            &assessment.candidate_unlock_features,
            &assessment.action,
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
        "api_family",
        "status",
        "coverage_kind",
        "primary_gap_kind",
        "shadow_item",
        "drift_confidence",
        "shadow_elicit_impl",
        "verification_gap",
        "verification_ready",
        "shadow_can_be_direct",
        "shadow_missing_external_traits",
        "shadow_missing_our_traits",
        "action",
        "notes",
    ])
    .map_err(|e| ElicitDocError::csv(e.to_string()))?;

    let mut rows: Vec<_> = report
        .rows
        .iter()
        .map(|row| (row, assess_shadow_row(row)))
        .collect();
    rows.sort_by(
        |(left_row, left_assessment), (right_row, right_assessment)| {
            let left_family = api_family(&left_row.item_path);
            let right_family = api_family(&right_row.item_path);
            shadow_row_kind_order(
                &left_assessment.coverage_kind,
                left_assessment.verification_gap,
            )
            .cmp(&shadow_row_kind_order(
                &right_assessment.coverage_kind,
                right_assessment.verification_gap,
            ))
            .then(left_family.cmp(&right_family))
            .then(left_row.item_path.cmp(&right_row.item_path))
        },
    );

    for (row, assessment) in rows {
        wtr.write_record([
            &row.item_path,
            &row.item_kind.to_string(),
            &api_family(&row.item_path),
            &row.status.to_string(),
            &assessment.coverage_kind.to_string(),
            &assessment
                .primary_gap_kind
                .as_ref()
                .map(|kind| kind.to_string())
                .unwrap_or_default(),
            &row.shadow_item,
            &row.drift_confidence,
            &row.shadow_elicit_impl,
            &assessment.verification_gap.to_string(),
            &assessment.verification_ready.to_string(),
            &row.shadow_can_be_direct,
            &row.shadow_missing_external_traits,
            &row.shadow_missing_our_traits,
            &assessment.action,
            &row.notes,
        ])
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    }

    wtr.flush()
        .map_err(|e| ElicitDocError::csv(e.to_string()))?;
    tracing::info!(path = %path.display(), rows = report.rows.len(), "wrote shadow CSV");
    Ok(())
}

fn api_family(path: &str) -> String {
    let mut parts = path.split("::");
    let mut family = Vec::with_capacity(3);
    for _ in 0..3 {
        if let Some(part) = parts.next() {
            family.push(part);
        } else {
            break;
        }
    }
    family.join("::")
}

fn impl_row_kind_order(kind: &ImplRowKind) -> u8 {
    match kind {
        ImplRowKind::MissingOurTraits => 0,
        ImplRowKind::ReadyForElicitComplete => 1,
        ImplRowKind::FeatureGatedExternal => 2,
        ImplRowKind::ExternallyBlocked => 3,
        ImplRowKind::Covered => 4,
    }
}

fn shadow_row_kind_order(kind: &ShadowCoverageKind, verification_gap: bool) -> u8 {
    match (kind, verification_gap) {
        (ShadowCoverageKind::Missing, _) => 0,
        (ShadowCoverageKind::Drifted, _) => 1,
        (ShadowCoverageKind::Covered, true) => 2,
        (ShadowCoverageKind::PossiblyStale, _) => 3,
        (ShadowCoverageKind::InfrastructureExtra, _) => 4,
        (ShadowCoverageKind::Covered, false) => 5,
    }
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
        "feature_owner_crate",
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
            &e.feature_owner_crate,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TraitPrereqs;
    use crate::impl_coverage::{ImplCoverageEntry, ImplStatus, TestStatus};
    use crate::inventory::ItemKind;
    use crate::shadow::{ShadowRow, ShadowStatus};

    #[test]
    fn impl_csv_rows_include_actionable_gap_columns() {
        let report = ImplCoverageReport {
            source_crate: "reqwest".to_string(),
            source_version: "1.0.0".to_string(),
            entries: vec![ImplCoverageEntry {
                type_path: "reqwest::async_impl::client::Client".to_string(),
                type_kind: ItemKind::Struct,
                is_generic: false,
                lifetime_params: Vec::new(),
                type_params: Vec::new(),
                elicit_impl: ImplStatus::Missing,
                proof_test: TestStatus::Missing,
                composition_test: TestStatus::Missing,
                prereqs: TraitPrereqs {
                    serialize: false,
                    deserialize: false,
                    json_schema: false,
                    elicitation_trait: true,
                    elicit_introspect: true,
                    elicit_spec: false,
                    elicit_prompt_tree: false,
                    to_code_literal: false,
                },
                notes: String::new(),
            }],
            complete_count: 0,
            missing_impl_count: 1,
            missing_test_count: 1,
            flagged_concrete_count: 0,
        };
        let path =
            std::env::temp_dir().join(format!("elicit_doc-report-{}-impl.csv", std::process::id()));
        write_impl_coverage_csv(&report, None, &path).expect("impl csv");
        let csv = std::fs::read_to_string(&path).expect("read impl csv");
        std::fs::remove_file(&path).ok();

        assert!(csv.contains("row_kind"));
        assert!(csv.contains("primary_gap_kind"));
        assert!(csv.contains("action"));
        assert!(csv.contains("MissingOurTraits"));
        assert!(csv.contains("Add our traits to `reqwest::async_impl::client::Client`"));
    }

    #[test]
    fn shadow_csv_rows_include_actionable_gap_columns() {
        let report = ShadowReport {
            target_crate: "reqwest".to_string(),
            target_version: "1.0.0".to_string(),
            shadow_crate: "elicit_reqwest".to_string(),
            shadow_version: "1.0.0".to_string(),
            rows: vec![
                ShadowRow {
                    item_path: "reqwest::async_impl::client::ClientBuilder".to_string(),
                    item_kind: ItemKind::Struct,
                    status: ShadowStatus::Missing,
                    shadow_item: String::new(),
                    drift_confidence: String::new(),
                    shadow_elicit_impl: String::new(),
                    shadow_can_be_direct: String::new(),
                    shadow_missing_external_traits: String::new(),
                    shadow_missing_our_traits: String::new(),
                    notes: String::new(),
                },
                ShadowRow {
                    item_path: "reqwest::async_impl::client::Client".to_string(),
                    item_kind: ItemKind::Struct,
                    status: ShadowStatus::Covered,
                    shadow_item: "elicit_reqwest::Client".to_string(),
                    drift_confidence: String::new(),
                    shadow_elicit_impl: ImplStatus::Missing.to_string(),
                    shadow_can_be_direct: "true".to_string(),
                    shadow_missing_external_traits: String::new(),
                    shadow_missing_our_traits: "ElicitSpec".to_string(),
                    notes: String::new(),
                },
            ],
            covered_count: 1,
            missing_count: 1,
            extra_count: 0,
            drifted_count: 0,
            coverage_pct: 50.0,
            verification_gap_count: 1,
        };
        let path = std::env::temp_dir().join(format!(
            "elicit_doc-report-{}-shadow.csv",
            std::process::id()
        ));
        write_shadow_csv(&report, &path).expect("shadow csv");
        let csv = std::fs::read_to_string(&path).expect("read shadow csv");
        std::fs::remove_file(&path).ok();

        assert!(csv.contains("coverage_kind"));
        assert!(csv.contains("verification_gap"));
        assert!(
            csv.contains("Add a shadow for upstream `reqwest::async_impl::client::ClientBuilder`")
        );
        assert!(csv.contains("Finish `elicit_reqwest::Client` by adding our traits"));
    }
}
