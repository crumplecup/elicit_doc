//! Shadow crate coverage analysis — use cases 3 and 4.
//!
//! Compares a target crate's [`Inventory`] against its shadow crate inventory
//! to produce a [`ShadowReport`] showing coverage, extras, and probable drifts.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::collect::{ElicitCompleteSet, TraitPrereqs};
use crate::impl_coverage::ImplStatus;
use crate::inventory::{Inventory, Item, ItemKind};

/// How a target item relates to the shadow crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowStatus {
    /// Present in both target and shadow with matching name.
    Covered,
    /// In shadow but not in target (extra / stale).
    Extra,
    /// In target but not in shadow (gap).
    Missing,
    /// Probable rename — target item matched to a shadow item by fuzzy heuristic.
    Drifted,
}

impl std::fmt::Display for ShadowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Covered => write!(f, "Covered"),
            Self::Extra => write!(f, "Extra"),
            Self::Missing => write!(f, "Missing"),
            Self::Drifted => write!(f, "Drifted"),
        }
    }
}

/// One row in a shadow coverage report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowRow {
    pub item_path: String,
    pub item_kind: ItemKind,
    pub status: ShadowStatus,
    /// Path of the matched shadow item (for Covered/Drifted rows).
    pub shadow_item: String,
    /// Confidence score for drift matches (0.0–1.0, empty for non-drift rows).
    pub drift_confidence: String,
    /// Verification readiness of the matched shadow type, when applicable.
    pub shadow_elicit_impl: String,
    pub shadow_can_be_direct: String,
    pub shadow_missing_external_traits: String,
    pub shadow_missing_our_traits: String,
    pub notes: String,
}

/// A probable rename between target and shadow.
#[derive(Debug, Clone)]
pub struct DriftPair {
    pub target_item: Item,
    pub shadow_item: Item,
    pub confidence: f32,
}

/// Full shadow coverage report for one target+shadow pair.
#[derive(Debug, Clone)]
pub struct ShadowReport {
    pub target_crate: String,
    pub target_version: String,
    pub shadow_crate: String,
    pub shadow_version: String,
    pub rows: Vec<ShadowRow>,
    pub covered_count: usize,
    pub missing_count: usize,
    pub extra_count: usize,
    pub drifted_count: usize,
    pub coverage_pct: f32,
    pub verification_gap_count: usize,
}

impl ShadowReport {
    /// Summary line for CLI output.
    pub fn summary(&self) -> String {
        format!(
            "{} vs {} ({:.1}% covered: {} covered, {} missing, {} extra, {} drifted)",
            self.target_crate,
            self.shadow_crate,
            self.coverage_pct,
            self.covered_count,
            self.missing_count,
            self.extra_count,
            self.drifted_count,
        )
    }
}

/// Build a [`ShadowReport`] by diffing `target` against `shadow`.
///
/// Coverage is determined by **exact bare name match**: `egui::Vec2` is covered
/// only by a shadow item also named `Vec2` (module path doesn't matter).
/// `EguiVec2` does NOT cover `Vec2` — that is a naming error, not a mirror.
///
/// Drift detection runs separately on unmatched items and flags probable renames
/// with edit-distance heuristics (shown as `Drifted`, not `Covered`).
#[instrument(
    skip(target, shadow, shadow_complete, shadow_prereqs),
    fields(target = %target.crate_name, shadow = %shadow.crate_name)
)]
pub fn build_shadow_report(
    target: &Inventory,
    shadow: &Inventory,
    shadow_complete: &ElicitCompleteSet,
    shadow_prereqs: &HashMap<String, TraitPrereqs>,
) -> ShadowReport {
    // Exact name → shadow item (same kind wins over cross-kind collision).
    // We collect all shadow items by bare name; if multiple items share a name,
    // prefer the one whose kind matches the target item at lookup time.
    let mut shadow_by_name: HashMap<&str, Vec<&Item>> = HashMap::new();
    for item in &shadow.items {
        if !counts_toward_shadow_coverage(item) {
            continue;
        }
        shadow_by_name
            .entry(item.name.as_str())
            .or_default()
            .push(item);
    }

    // Normalized map used only for drift detection (not coverage).
    let mut shadow_normalized: HashMap<String, Vec<&Item>> = HashMap::new();
    for item in &shadow.items {
        if !counts_toward_shadow_coverage(item) {
            continue;
        }
        shadow_normalized
            .entry(normalize_name(&item.name))
            .or_default()
            .push(item);
    }

    let mut rows: Vec<ShadowRow> = Vec::new();

    for target_item in &target.items {
        if !counts_toward_shadow_coverage(target_item) {
            continue;
        }
        // Exact name match — kind-preferred, any accepted
        let exact = shadow_by_name
            .get(target_item.name.as_str())
            .and_then(|candidates| {
                candidates
                    .iter()
                    .find(|c| c.kind == target_item.kind)
                    .or_else(|| candidates.first())
                    .copied()
            });

        if let Some(shadow_item) = exact {
            rows.push(ShadowRow {
                item_path: target_item.path_str(),
                item_kind: target_item.kind,
                status: ShadowStatus::Covered,
                shadow_item: shadow_item.path_str(),
                drift_confidence: String::new(),
                shadow_elicit_impl: shadow_impl_status(shadow_item, shadow_complete).to_string(),
                shadow_can_be_direct: shadow_can_be_direct(shadow_item, shadow_prereqs),
                shadow_missing_external_traits: shadow_missing_external_traits(
                    shadow_item,
                    shadow_prereqs,
                ),
                shadow_missing_our_traits: shadow_missing_our_traits(shadow_item, shadow_prereqs),
                notes: String::new(),
            });
        } else if let Some((shadow_item, confidence)) =
            find_drift_match(target_item, &shadow_normalized)
        {
            rows.push(ShadowRow {
                item_path: target_item.path_str(),
                item_kind: target_item.kind,
                status: ShadowStatus::Drifted,
                shadow_item: shadow_item.path_str(),
                drift_confidence: format!("{confidence:.2}"),
                shadow_elicit_impl: shadow_impl_status(shadow_item, shadow_complete).to_string(),
                shadow_can_be_direct: shadow_can_be_direct(shadow_item, shadow_prereqs),
                shadow_missing_external_traits: shadow_missing_external_traits(
                    shadow_item,
                    shadow_prereqs,
                ),
                shadow_missing_our_traits: shadow_missing_our_traits(shadow_item, shadow_prereqs),
                notes: "probable rename".to_string(),
            });
        } else {
            rows.push(ShadowRow {
                item_path: target_item.path_str(),
                item_kind: target_item.kind,
                status: ShadowStatus::Missing,
                shadow_item: String::new(),
                drift_confidence: String::new(),
                shadow_elicit_impl: String::new(),
                shadow_can_be_direct: String::new(),
                shadow_missing_external_traits: String::new(),
                shadow_missing_our_traits: String::new(),
                notes: String::new(),
            });
        }
    }

    // Extra items: in shadow but not matched to any target
    let matched_shadow_paths: HashSet<String> = rows
        .iter()
        .filter(|r| matches!(r.status, ShadowStatus::Covered | ShadowStatus::Drifted))
        .map(|r| r.shadow_item.clone())
        .collect();

    for shadow_item in &shadow.items {
        if !counts_toward_shadow_coverage(shadow_item) {
            continue;
        }
        if !matched_shadow_paths.contains(&shadow_item.path_str()) {
            rows.push(ShadowRow {
                item_path: shadow_item.path_str(),
                item_kind: shadow_item.kind,
                status: ShadowStatus::Extra,
                shadow_item: String::new(),
                drift_confidence: String::new(),
                shadow_elicit_impl: String::new(),
                shadow_can_be_direct: String::new(),
                shadow_missing_external_traits: String::new(),
                shadow_missing_our_traits: String::new(),
                notes: "in shadow, not in target".to_string(),
            });
        }
    }

    rows.sort_by(|a, b| a.item_path.cmp(&b.item_path));

    let covered_count = rows
        .iter()
        .filter(|r| r.status == ShadowStatus::Covered)
        .count();
    let missing_count = rows
        .iter()
        .filter(|r| r.status == ShadowStatus::Missing)
        .count();
    let extra_count = rows
        .iter()
        .filter(|r| r.status == ShadowStatus::Extra)
        .count();
    let drifted_count = rows
        .iter()
        .filter(|r| r.status == ShadowStatus::Drifted)
        .count();
    let total_target = target
        .items
        .iter()
        .filter(|item| counts_toward_shadow_coverage(item))
        .count();
    let coverage_pct = if total_target == 0 {
        100.0
    } else {
        (covered_count + drifted_count) as f32 / total_target as f32 * 100.0
    };
    let verification_gap_count = rows.iter().filter(|r| shadow_verification_gap(r)).count();

    tracing::info!(
        covered = covered_count,
        missing = missing_count,
        extra = extra_count,
        drifted = drifted_count,
        pct = coverage_pct,
        verification_gaps = verification_gap_count,
        "built shadow report"
    );

    ShadowReport {
        target_crate: target.crate_name.clone(),
        target_version: target.crate_version.clone(),
        shadow_crate: shadow.crate_name.clone(),
        shadow_version: shadow.crate_version.clone(),
        rows,
        covered_count,
        missing_count,
        extra_count,
        drifted_count,
        coverage_pct,
        verification_gap_count,
    }
}

fn counts_toward_shadow_coverage(item: &Item) -> bool {
    item.kind != ItemKind::Module
}

/// Normalize a type name for **drift detection only** — lowercase + snake_case,
/// but do NOT strip crate-specific prefixes.
///
/// Prefix stripping (`Egui`, `Bevy`, etc.) is intentionally omitted: if a shadow
/// item is named `EguiVec2` when it should be `Vec2`, that is a naming error that
/// should appear as `Missing`/`Extra`, not a drift match.  Drift detection is
/// reserved for genuine upstream renames (e.g. `Vec3A` → `Vec3Aligned`).
fn normalize_name(name: &str) -> String {
    to_snake_case(name).to_lowercase()
}

/// Simple PascalCase → snake_case conversion.
fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.char_indices() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_lowercase().next().unwrap_or(ch));
    }
    out
}

/// Attempt a fuzzy drift match: find a shadow item whose normalized name is
/// within edit distance 2 of the target's normalized name.
fn find_drift_match<'a>(
    target_item: &Item,
    shadow_names: &HashMap<String, Vec<&'a Item>>,
) -> Option<(&'a Item, f32)> {
    let target_norm = normalize_name(&target_item.name);
    let mut best: Option<(&Item, f32)> = None;

    for (shadow_norm, candidates) in shadow_names {
        let dist = edit_distance(&target_norm, shadow_norm);
        let max_len = target_norm.len().max(shadow_norm.len());
        if max_len == 0 {
            continue;
        }
        let confidence = 1.0 - (dist as f32 / max_len as f32);
        if confidence < 0.75 {
            continue;
        }
        for shadow_item in candidates {
            if shadow_item.kind != target_item.kind {
                continue;
            }
            if best.is_none_or(|(_, c)| confidence > c) {
                best = Some((shadow_item, confidence));
            }
        }
    }

    best
}

fn shadow_impl_status(item: &Item, complete: &ElicitCompleteSet) -> ImplStatus {
    if !item.kind.is_type() {
        return ImplStatus::Missing;
    }

    let path = item.path_str();
    if complete.factory.contains(&path) {
        ImplStatus::CompleteFactory
    } else if complete.concrete.contains(&path) {
        ImplStatus::Complete
    } else {
        ImplStatus::Missing
    }
}

fn shadow_can_be_direct(item: &Item, prereqs: &HashMap<String, TraitPrereqs>) -> String {
    if !item.kind.is_type() {
        return String::new();
    }
    prereqs
        .get(&item.path_str())
        .map(|p| p.can_be_direct().to_string())
        .unwrap_or_else(|| "false".to_string())
}

fn shadow_missing_external_traits(item: &Item, prereqs: &HashMap<String, TraitPrereqs>) -> String {
    if !item.kind.is_type() {
        return String::new();
    }
    prereqs
        .get(&item.path_str())
        .map(|p| p.external_blockers_absent().join(";"))
        .unwrap_or_else(|| "Serialize(absent);Deserialize(absent);JsonSchema(absent)".to_string())
}

fn shadow_missing_our_traits(item: &Item, prereqs: &HashMap<String, TraitPrereqs>) -> String {
    if !item.kind.is_type() {
        return String::new();
    }
    prereqs
        .get(&item.path_str())
        .map(|p| p.missing_our_traits().join(";"))
        .unwrap_or_else(|| {
            [
                "Elicitation",
                "ElicitIntrospect",
                "ElicitSpec",
                "ElicitPromptTree",
                "ToCodeLiteral",
            ]
            .join(";")
        })
}

fn shadow_verification_gap(row: &ShadowRow) -> bool {
    if !matches!(row.status, ShadowStatus::Covered | ShadowStatus::Drifted)
        || !row.item_kind.is_type()
    {
        return false;
    }

    row.shadow_elicit_impl != ImplStatus::Complete.to_string()
        && row.shadow_elicit_impl != ImplStatus::CompleteFactory.to_string()
}

/// Simple iterative Levenshtein edit distance.
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j].min(dp[i][j - 1]).min(dp[i - 1][j - 1])
            };
        }
    }
    dp[m][n]
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::collect::ElicitCompleteSet;
    use crate::inventory::{Inventory, Item, ItemKind};

    #[test]
    fn covers_full_public_surface_not_just_types() {
        let target = Inventory {
            crate_name: "upstream".to_string(),
            crate_version: "1.0.0".to_string(),
            items: vec![
                Item {
                    path: vec!["upstream".to_string(), "Widget".to_string()],
                    kind: ItemKind::Struct,
                    name: "Widget".to_string(),
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                },
                Item {
                    path: vec!["upstream".to_string(), "build_widget".to_string()],
                    kind: ItemKind::Function,
                    name: "build_widget".to_string(),
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                },
            ],
        };
        let shadow = Inventory {
            crate_name: "elicit_upstream".to_string(),
            crate_version: "1.0.0".to_string(),
            items: vec![Item {
                path: vec!["elicit_upstream".to_string(), "Widget".to_string()],
                kind: ItemKind::Struct,
                name: "Widget".to_string(),
                is_generic: false,
                lifetime_params: Vec::new(),
                type_params: Vec::new(),
            }],
        };

        let report = build_shadow_report(
            &target,
            &shadow,
            &ElicitCompleteSet::default(),
            &HashMap::new(),
        );

        let function_row = report
            .rows
            .iter()
            .find(|row| row.item_path == "upstream::build_widget")
            .expect("expected function row");
        assert_eq!(function_row.item_kind, ItemKind::Function);
        assert_eq!(function_row.status, ShadowStatus::Missing);
        assert_eq!(report.missing_count, 1);
    }

    #[test]
    fn ignores_modules_in_shadow_coverage() {
        let target = Inventory {
            crate_name: "upstream".to_string(),
            crate_version: "1.0.0".to_string(),
            items: vec![
                Item {
                    path: vec!["upstream".to_string()],
                    kind: ItemKind::Module,
                    name: "upstream".to_string(),
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                },
                Item {
                    path: vec!["upstream".to_string(), "nested".to_string()],
                    kind: ItemKind::Module,
                    name: "nested".to_string(),
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                },
                Item {
                    path: vec!["upstream".to_string(), "Widget".to_string()],
                    kind: ItemKind::Struct,
                    name: "Widget".to_string(),
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                },
            ],
        };
        let shadow = Inventory {
            crate_name: "elicit_upstream".to_string(),
            crate_version: "1.0.0".to_string(),
            items: vec![Item {
                path: vec!["elicit_upstream".to_string(), "Widget".to_string()],
                kind: ItemKind::Struct,
                name: "Widget".to_string(),
                is_generic: false,
                lifetime_params: Vec::new(),
                type_params: Vec::new(),
            }],
        };

        let report = build_shadow_report(
            &target,
            &shadow,
            &ElicitCompleteSet::default(),
            &HashMap::new(),
        );

        assert_eq!(report.covered_count, 1);
        assert_eq!(report.missing_count, 0);
        assert_eq!(report.coverage_pct, 100.0);
        assert!(report.rows.iter().all(|row| row.item_kind != ItemKind::Module));
    }
}
