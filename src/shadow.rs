//! Shadow crate coverage analysis — use cases 3 and 4.
//!
//! Compares a target crate's [`Inventory`] against its shadow crate inventory
//! to produce a [`ShadowReport`] showing coverage, extras, and probable drifts.

use serde::{Deserialize, Serialize};
use tracing::instrument;

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
#[instrument(
    skip(target, shadow),
    fields(target = %target.crate_name, shadow = %shadow.crate_name)
)]
pub fn build_shadow_report(target: &Inventory, shadow: &Inventory) -> ShadowReport {
    // Normalize names for matching
    let shadow_names: std::collections::HashMap<String, &Item> = shadow
        .type_items()
        .map(|i| (normalize_name(&i.name), i))
        .collect();

    let mut rows: Vec<ShadowRow> = Vec::new();

    for target_item in target.type_items() {
        let norm = normalize_name(&target_item.name);

        if let Some(shadow_item) = shadow_names.get(&norm) {
            rows.push(ShadowRow {
                item_path: target_item.path_str(),
                item_kind: target_item.kind,
                status: ShadowStatus::Covered,
                shadow_item: shadow_item.path_str(),
                drift_confidence: String::new(),
                notes: String::new(),
            });
        } else {
            // Try fuzzy drift match
            if let Some((shadow_item, confidence)) = find_drift_match(target_item, &shadow_names) {
                rows.push(ShadowRow {
                    item_path: target_item.path_str(),
                    item_kind: target_item.kind,
                    status: ShadowStatus::Drifted,
                    shadow_item: shadow_item.path_str(),
                    drift_confidence: format!("{confidence:.2}"),
                    notes: "probable rename".to_string(),
                });
            } else {
                rows.push(ShadowRow {
                    item_path: target_item.path_str(),
                    item_kind: target_item.kind,
                    status: ShadowStatus::Missing,
                    shadow_item: String::new(),
                    drift_confidence: String::new(),
                    notes: String::new(),
                });
            }
        }
    }

    // Extra items: in shadow but not matched to any target
    let matched_shadow_paths: std::collections::HashSet<String> = rows
        .iter()
        .filter(|r| matches!(r.status, ShadowStatus::Covered | ShadowStatus::Drifted))
        .map(|r| r.shadow_item.clone())
        .collect();

    for shadow_item in shadow.type_items() {
        if !matched_shadow_paths.contains(&shadow_item.path_str()) {
            rows.push(ShadowRow {
                item_path: shadow_item.path_str(),
                item_kind: shadow_item.kind,
                status: ShadowStatus::Extra,
                shadow_item: String::new(),
                drift_confidence: String::new(),
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
    let total_target = target.type_items().count();
    let coverage_pct = if total_target == 0 {
        100.0
    } else {
        (covered_count + drifted_count) as f32 / total_target as f32 * 100.0
    };

    tracing::info!(
        covered = covered_count,
        missing = missing_count,
        extra = extra_count,
        drifted = drifted_count,
        pct = coverage_pct,
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
    }
}

/// Normalize a type name for comparison: lowercase, strip common crate prefixes,
/// convert to snake_case.
fn normalize_name(name: &str) -> String {
    // Strip common prefixes like Bevy, Wgpu, Elicit, Egui, Winit, Ratatui…
    let prefixes = [
        "Bevy", "Wgpu", "Egui", "Winit", "Ratatui", "Elicit", "Geo", "Proj", "Rstar", "Csv",
        "Toml", "Axum", "Reqwest", "Tower",
    ];
    let mut s = name.to_string();
    for prefix in prefixes {
        if let Some(rest) = s.strip_prefix(prefix)
            && !rest.is_empty()
        {
            s = rest.to_string();
            break;
        }
    }
    to_snake_case(&s).to_lowercase()
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
    shadow_names: &std::collections::HashMap<String, &'a Item>,
) -> Option<(&'a Item, f32)> {
    let target_norm = normalize_name(&target_item.name);
    let mut best: Option<(&Item, f32)> = None;

    for (shadow_norm, shadow_item) in shadow_names {
        // Only match same kind
        if shadow_item.kind != target_item.kind {
            continue;
        }
        let dist = edit_distance(&target_norm, shadow_norm);
        let max_len = target_norm.len().max(shadow_norm.len());
        if max_len == 0 {
            continue;
        }
        let confidence = 1.0 - (dist as f32 / max_len as f32);
        // Only surface high-confidence matches (>= 0.75)
        if confidence >= 0.75
            && best.is_none_or(|(_, c)| confidence > c)
        {
            best = Some((shadow_item, confidence));
        }
    }

    best
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
