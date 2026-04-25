//! Gaps analysis — cross-crate actionable summary derived from impl and shadow reports.
//!
//! Reads already-built [`ImplCoverageReport`] and [`ShadowReport`] values and
//! produces flat, deduplicated gap lists that answer four questions:
//!
//! 1. **ReadyNow** — types that have all external prerequisites (`Serialize +
//!    `Deserialize` + `JsonSchema`) but still lack an `ElicitComplete` impl.
//!    Action: add the impl.
//!
//! 2. **FeatureGated** — types where at least one external trait could not be
//!    confirmed because the dep build fell back to default features.
//!    Action: enable the dep's serde/schemars feature flags and re-check.
//!
//! 3. **NeedsExternalImpl** — types confirmed (via `--all-features` build) to be
//!    missing `Serialize`, `Deserialize`, or `JsonSchema` with no known feature fix.
//!    Action: add a trenchcoat wrapper, or wait for upstream crate support.
//!
//! 4. **Shadow gaps** — upstream types missing from a shadow crate (`Missing`),
//!    probably renamed (`Drifted`), or stale extras in the shadow (`Extra`).

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::impl_coverage::{ImplCoverageEntry, ImplCoverageReport, ImplStatus};
use crate::inventory::ItemKind;
use crate::shadow::{ShadowReport, ShadowStatus};

// ── Impl gaps ────────────────────────────────────────────────────────────────

/// Why a type lacks an `ElicitComplete` impl.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImplGapKind {
    /// All external traits are `present`; only needs `impl ElicitComplete` added.
    ReadyNow,
    /// At least one external trait is `feature_gated` (dep built with default
    /// features only due to a build failure).  May be unlockable.
    FeatureGated,
    /// All missing external traits are confirmed `absent` (all-features build
    /// succeeded).  Needs a trenchcoat or upstream schemars/serde support.
    NeedsExternalImpl,
}

impl std::fmt::Display for ImplGapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadyNow => write!(f, "ReadyNow"),
            Self::FeatureGated => write!(f, "FeatureGated"),
            Self::NeedsExternalImpl => write!(f, "NeedsExternalImpl"),
        }
    }
}

/// One row in the consolidated impl gaps report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplGapEntry {
    /// Source crate the type lives in (`uuid`, `url`, `elicitation`, …).
    pub source_crate: String,
    /// Canonical type path (e.g. `uuid::Uuid`).
    pub type_path: String,
    pub type_kind: ItemKind,
    pub is_generic: bool,
    /// Primary gap classification.
    pub gap_kind: ImplGapKind,
    /// Which of `Serialize`, `Deserialize`, `JsonSchema` are missing,
    /// annotated with `(absent)` or `(feature_gated)`.
    pub missing_external_traits: String,
    /// Which of our own 5 traits are still absent for this type.
    pub missing_our_traits: String,
    /// Short recommended action.
    pub action: String,
}

/// Build the consolidated impl gaps list from multiple per-crate reports.
///
/// Only types with `ImplStatus::Missing` are included — types that already have
/// `ElicitComplete` are not gaps.
#[instrument(skip(pairs), fields(num_reports = pairs.len()))]
pub fn build_impl_gaps(pairs: &[(&str, &ImplCoverageReport)]) -> Vec<ImplGapEntry> {
    let mut entries = Vec::new();

    for (source_crate, report) in pairs {
        for entry in &report.entries {
            if !matches!(entry.elicit_impl, ImplStatus::Missing) {
                continue;
            }

            let gap_entry = classify_impl_gap(source_crate, entry);
            entries.push(gap_entry);
        }
    }

    // Sort: ReadyNow first (highest priority), then FeatureGated, then NeedsExternalImpl.
    entries.sort_by(|a, b| {
        gap_kind_order(&a.gap_kind)
            .cmp(&gap_kind_order(&b.gap_kind))
            .then(a.source_crate.cmp(&b.source_crate))
            .then(a.type_path.cmp(&b.type_path))
    });

    tracing::info!(
        total_gaps = entries.len(),
        ready_now = entries.iter().filter(|e| e.gap_kind == ImplGapKind::ReadyNow).count(),
        feature_gated = entries.iter().filter(|e| e.gap_kind == ImplGapKind::FeatureGated).count(),
        needs_external = entries.iter().filter(|e| e.gap_kind == ImplGapKind::NeedsExternalImpl).count(),
        "built impl gaps report"
    );

    entries
}

fn gap_kind_order(k: &ImplGapKind) -> u8 {
    match k {
        ImplGapKind::ReadyNow => 0,
        ImplGapKind::FeatureGated => 1,
        ImplGapKind::NeedsExternalImpl => 2,
    }
}

fn classify_impl_gap(source_crate: &str, entry: &ImplCoverageEntry) -> ImplGapEntry {
    let p = &entry.prereqs;
    let all_features = entry.all_features_build;

    // Classify each missing external trait.
    let mut missing_external: Vec<String> = Vec::new();
    let mut any_feature_gated = false;

    for (present, name) in [
        (p.serialize, "Serialize"),
        (p.deserialize, "Deserialize"),
        (p.json_schema, "JsonSchema"),
    ] {
        if !present {
            let label = if all_features { "absent" } else { "feature_gated" };
            missing_external.push(format!("{name}({label})"));
            if !all_features {
                any_feature_gated = true;
            }
        }
    }

    // Classify each missing internal trait.
    let mut missing_our: Vec<&str> = Vec::new();
    for (present, name) in [
        (p.elicitation_trait, "Elicitation"),
        (p.elicit_introspect, "ElicitIntrospect"),
        (p.elicit_spec, "ElicitSpec"),
        (p.elicit_prompt_tree, "ElicitPromptTree"),
        (p.to_code_literal, "ToCodeLiteral"),
    ] {
        if !present {
            missing_our.push(name);
        }
    }

    let gap_kind = if missing_external.is_empty() {
        ImplGapKind::ReadyNow
    } else if any_feature_gated {
        ImplGapKind::FeatureGated
    } else {
        ImplGapKind::NeedsExternalImpl
    };

    let action = match &gap_kind {
        ImplGapKind::ReadyNow => format!(
            "Add `impl ElicitComplete for {} {{}}` in elicitation crate",
            entry.type_path
        ),
        ImplGapKind::FeatureGated => format!(
            "Enable serde/schemars features for `{source_crate}` dep and re-check; \
             missing: {}",
            missing_external.join(", ")
        ),
        ImplGapKind::NeedsExternalImpl => format!(
            "Add trenchcoat wrapper for `{}`; missing: {}",
            entry.type_path,
            missing_external.join(", ")
        ),
    };

    ImplGapEntry {
        source_crate: source_crate.to_string(),
        type_path: entry.type_path.clone(),
        type_kind: entry.type_kind,
        is_generic: entry.is_generic,
        gap_kind,
        missing_external_traits: missing_external.join(";"),
        missing_our_traits: missing_our.join(";"),
        action,
    }
}

// ── Shadow gaps ───────────────────────────────────────────────────────────────

/// Why a shadow crate item is flagged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowGapKind {
    /// Upstream type not present in shadow crate.  Needs to be added.
    Missing,
    /// Probable rename — shadow has a type with a similar but different name.
    Drifted,
    /// Shadow has a type not present in upstream, and it doesn't match any
    /// known infrastructure naming convention.  May be stale or misnamed.
    PossiblyStale,
    /// Shadow has a type not present in upstream, but it follows an
    /// infrastructure naming convention (`*Params`, `*Plugin`, `*Ctx`, etc.).
    /// These are expected additions — tool parameter structs, plugin wrappers,
    /// context objects — and are NOT gaps.
    InfrastructureExtra,
}

impl std::fmt::Display for ShadowGapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "Missing"),
            Self::Drifted => write!(f, "Drifted"),
            Self::PossiblyStale => write!(f, "PossiblyStale"),
            Self::InfrastructureExtra => write!(f, "InfrastructureExtra"),
        }
    }
}

/// Bare-name suffixes that identify our own shadow-crate infrastructure types.
///
/// These are not shadows of upstream types — they're tool params structs,
/// plugin wrappers, context objects, etc. that we add deliberately.
const INFRA_SUFFIXES: &[&str] = &[
    "Params",
    "ParamsStyle",
    "Plugin",
    "Ctx",
    "Descriptor",
    "Factory",
    "Hook",
    "Json",
];

/// Returns `true` when the bare name of an "extra" shadow item matches a known
/// infrastructure naming convention — i.e., it's one of our own additions, not
/// a misnamed shadow of an upstream type.
fn is_infrastructure_name(bare_name: &str) -> bool {
    INFRA_SUFFIXES.iter().any(|sfx| bare_name.ends_with(sfx))
}

/// One row in the consolidated shadow gaps report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowGapEntry {
    pub target_crate: String,
    pub shadow_crate: String,
    pub item_path: String,
    pub item_kind: ItemKind,
    pub gap_kind: ShadowGapKind,
    /// For `Drifted` rows: the shadow item that was matched.
    pub matched_shadow_item: String,
    /// Drift confidence score (0.0–1.0), empty for non-drifted rows.
    pub drift_confidence: String,
    pub notes: String,
}

/// Build the consolidated shadow gaps list from multiple per-pair reports.
///
/// `Covered` rows are excluded.  `Extra` rows from the per-crate shadow report
/// are further split into `InfrastructureExtra` (our own tool params/plugins/etc.)
/// and `PossiblyStale` (unexpected non-infrastructure types that may be wrong).
#[instrument(skip(pairs), fields(num_reports = pairs.len()))]
pub fn build_shadow_gaps(pairs: &[(&str, &str, &ShadowReport)]) -> Vec<ShadowGapEntry> {
    let mut entries = Vec::new();

    for (target_crate, shadow_crate, report) in pairs {
        for row in &report.rows {
            let gap_kind = match row.status {
                ShadowStatus::Covered => continue,
                ShadowStatus::Missing => ShadowGapKind::Missing,
                ShadowStatus::Drifted => ShadowGapKind::Drifted,
                ShadowStatus::Extra => {
                    // Split Extra into infrastructure vs possibly stale.
                    let bare = row.item_path.split("::").last().unwrap_or(&row.item_path);
                    if is_infrastructure_name(bare) {
                        ShadowGapKind::InfrastructureExtra
                    } else {
                        ShadowGapKind::PossiblyStale
                    }
                }
            };
            entries.push(ShadowGapEntry {
                target_crate: target_crate.to_string(),
                shadow_crate: shadow_crate.to_string(),
                item_path: row.item_path.clone(),
                item_kind: row.item_kind,
                gap_kind,
                matched_shadow_item: row.shadow_item.clone(),
                drift_confidence: row.drift_confidence.clone(),
                notes: row.notes.clone(),
            });
        }
    }

    // Sort: Missing first, then Drifted, PossiblyStale, InfrastructureExtra.
    entries.sort_by(|a, b| {
        shadow_gap_order(&a.gap_kind)
            .cmp(&shadow_gap_order(&b.gap_kind))
            .then(a.target_crate.cmp(&b.target_crate))
            .then(a.item_path.cmp(&b.item_path))
    });

    tracing::info!(
        total_gaps = entries.len(),
        missing = entries.iter().filter(|e| e.gap_kind == ShadowGapKind::Missing).count(),
        drifted = entries.iter().filter(|e| e.gap_kind == ShadowGapKind::Drifted).count(),
        possibly_stale = entries.iter().filter(|e| e.gap_kind == ShadowGapKind::PossiblyStale).count(),
        infra_extra = entries.iter().filter(|e| e.gap_kind == ShadowGapKind::InfrastructureExtra).count(),
        "built shadow gaps report"
    );

    entries
}

fn shadow_gap_order(k: &ShadowGapKind) -> u8 {
    match k {
        ShadowGapKind::Missing => 0,
        ShadowGapKind::Drifted => 1,
        ShadowGapKind::PossiblyStale => 2,
        ShadowGapKind::InfrastructureExtra => 3,
    }
}
