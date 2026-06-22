//! Gaps analysis — cross-crate actionable summary derived from impl and shadow reports.
//!
//! Reads already-built [`ImplCoverageReport`] and [`ShadowReport`] values and
//! produces flat, deduplicated gap lists that answer four questions:
//!
//! 1. **MissingOurTraits** — type lacks one or more elicitation-owned support traits.
//!    Action: add the missing support trait impls.
//!
//! 2. **ReadyForElicitComplete** — type has all prerequisites for `ElicitComplete`
//!    but still lacks the marker impl. Action: add the impl.
//!
//! 3. **FeatureGatedExternal** — external serde/schemars support may be unlockable
//!    by enabling more dependency features. Action: enable the features and re-check.
//!
//! 4. **Shadow gaps** — upstream items missing from a shadow crate (`Missing`),
//!    probably renamed (`Drifted`), or stale extras in the shadow (`Extra`).

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::collect::TraitPrereqs;
use crate::impl_coverage::{ImplCoverageEntry, ImplCoverageReport, ImplStatus};
use crate::inventory::ItemKind;
use crate::shadow::{ShadowReport, ShadowStatus};

// ── Impl gaps ────────────────────────────────────────────────────────────────

/// Why a type lacks an `ElicitComplete` impl.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImplGapKind {
    /// One or more of our five support traits are still missing.
    MissingOurTraits,
    /// All prerequisites are present; only `impl ElicitComplete` is missing.
    ReadyForElicitComplete,
    /// External serde/schemars traits may be unlockable via more dep features.
    FeatureGatedExternal,
}

impl std::fmt::Display for ImplGapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingOurTraits => write!(f, "MissingOurTraits"),
            Self::ReadyForElicitComplete => write!(f, "ReadyForElicitComplete"),
            Self::FeatureGatedExternal => write!(f, "FeatureGatedExternal"),
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
    /// All five elicitation-owned support traits are present.
    pub our_traits_complete: bool,
    /// The external prerequisites are present, so `ElicitComplete` is legal.
    pub can_be_direct: bool,
    /// This type is a real `ElicitComplete` gap: legal to impl directly, but missing it.
    pub elicit_complete_gap: bool,
    /// External support is missing and the dep exposes candidate features that may unlock it.
    pub feature_gated_external: bool,
    /// External support is missing under the current feature set, so the orphan rule blocks
    /// a direct `ElicitComplete` impl.
    pub blocked_by_orphan_rule: bool,
    /// Feature flags available in the dep that could unlock serde/schemars support.
    ///
    /// Only populated when external support is feature-gated.  Semicolon-separated.
    pub candidate_unlock_features: String,
    /// Short recommended action.
    pub action: String,
}

/// Build the consolidated impl gaps list from multiple per-crate reports.
///
/// Only types with `ImplStatus::Missing` are candidates for inclusion. Fully-covered,
/// orphan-blocked "everything but" types are excluded because they are not actionable gaps.
///
/// `available_serde_features` maps crate name → serde/schemars-related feature
/// names available in the dep (feature-graph reachability, from
/// [`crate::collect::collect_dep_serde_features`]).
///
/// `activated_features` maps crate name → the **transitively expanded** feature
/// set our workspace has activated for this dep.  For example, if `geo` declares
/// `use-serde → serde` and we activate `["use-serde"]`, the map contains
/// `["serde", "use-serde", ...]` so that `serde` is correctly counted as activated.
/// The difference (`available - expanded_activated`) helps distinguish
/// `FeatureGatedExternal` from fully orphan-blocked types.
#[instrument(skip(pairs, available_serde_features, activated_features), fields(num_reports = pairs.len()))]
pub fn build_impl_gaps(
    pairs: &[(&str, &ImplCoverageReport)],
    available_serde_features: &std::collections::HashMap<String, Vec<String>>,
    activated_features: &std::collections::HashMap<String, Vec<String>>,
    feature_probe_prereqs: &std::collections::HashMap<
        String,
        std::collections::HashMap<String, TraitPrereqs>,
    >,
) -> Vec<ImplGapEntry> {
    let mut entries = Vec::new();

    for (source_crate, report) in pairs {
        let available = available_serde_features
            .get(*source_crate)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let activated = activated_features
            .get(*source_crate)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        // Features that exist in the dep but we haven't activated yet.
        let unactivated: Vec<String> = available
            .iter()
            .filter(|f| !activated.iter().any(|a| a == *f))
            .cloned()
            .collect();
        for entry in &report.entries {
            if !matches!(entry.elicit_impl, ImplStatus::Missing) {
                continue;
            }

            let probed = feature_probe_prereqs
                .get(*source_crate)
                .and_then(|m| m.get(&entry.type_path));
            if let Some(gap_entry) = classify_impl_gap(source_crate, entry, &unactivated, probed) {
                entries.push(gap_entry);
            }
        }
    }

    // Sort: support-trait gaps first, then true ElicitComplete gaps, then feature-gated externals.
    entries.sort_by(|a, b| {
        gap_kind_order(&a.gap_kind)
            .cmp(&gap_kind_order(&b.gap_kind))
            .then(a.source_crate.cmp(&b.source_crate))
            .then(a.type_path.cmp(&b.type_path))
    });

    tracing::info!(
        total_gaps = entries.len(),
        missing_our_traits = entries
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::MissingOurTraits)
            .count(),
        ready_for_elicit_complete = entries
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::ReadyForElicitComplete)
            .count(),
        feature_gated = entries
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::FeatureGatedExternal)
            .count(),
        "built impl gaps report"
    );

    entries
}

fn gap_kind_order(k: &ImplGapKind) -> u8 {
    match k {
        ImplGapKind::MissingOurTraits => 0,
        ImplGapKind::ReadyForElicitComplete => 1,
        ImplGapKind::FeatureGatedExternal => 2,
    }
}

/// Classify a single missing impl entry.
///
/// `unactivated_serde_features` — serde features the dep offers that our shadow
/// crate has **not** yet activated. When non-empty, missing external support is
/// treated as `FeatureGatedExternal`. When empty, missing external support is a
/// true orphan-rule blocker.
fn classify_impl_gap(
    source_crate: &str,
    entry: &ImplCoverageEntry,
    unactivated_serde_features: &[String],
    probed_prereqs: Option<&TraitPrereqs>,
) -> Option<ImplGapEntry> {
    let p = &entry.prereqs;

    let missing_external = missing_external_traits(p);
    let missing_our = entry.effective_missing_our_traits();
    let our_traits_complete = missing_our.is_empty();
    let can_be_direct = entry.can_be_direct();
    let feature_gated_external = !entry.lifetime_blocks_elicitation()
        && !can_be_direct
        && !unactivated_serde_features.is_empty()
        && probed_prereqs.is_some_and(TraitPrereqs::can_be_direct);
    let blocked_by_orphan_rule =
        !entry.lifetime_blocks_elicitation() && !can_be_direct && !feature_gated_external;
    let elicit_complete_gap = can_be_direct;

    // Fully-covered "everything but" types are expected and should not be surfaced as gaps.
    if our_traits_complete && blocked_by_orphan_rule {
        return None;
    }

    let gap_kind = if !our_traits_complete {
        ImplGapKind::MissingOurTraits
    } else if elicit_complete_gap {
        ImplGapKind::ReadyForElicitComplete
    } else {
        ImplGapKind::FeatureGatedExternal
    };

    let action = match &gap_kind {
        ImplGapKind::MissingOurTraits => build_missing_our_traits_action(
            source_crate,
            &entry.type_path,
            &missing_our,
            &missing_external,
            can_be_direct,
            feature_gated_external,
            unactivated_serde_features,
        ),
        ImplGapKind::ReadyForElicitComplete => format!(
            "Add `impl ElicitComplete for {} {{}}` in elicitation crate",
            entry.type_path
        ),
        ImplGapKind::FeatureGatedExternal => format!(
            "Enable additional `{source_crate}` features in the workspace: {}; \
             then re-check whether `ElicitComplete` becomes legal for `{}`",
            unactivated_serde_features.join(", "),
            entry.type_path
        ),
    };

    Some(ImplGapEntry {
        source_crate: source_crate.to_string(),
        type_path: entry.type_path.clone(),
        type_kind: entry.type_kind,
        is_generic: entry.is_generic,
        gap_kind,
        missing_external_traits: missing_external.join(";"),
        missing_our_traits: missing_our.join(";"),
        our_traits_complete,
        can_be_direct,
        elicit_complete_gap,
        feature_gated_external,
        blocked_by_orphan_rule,
        candidate_unlock_features: unactivated_serde_features.join(";"),
        action,
    })
}

fn missing_external_traits(prereqs: &TraitPrereqs) -> Vec<String> {
    [
        (prereqs.serialize, "Serialize"),
        (prereqs.deserialize, "Deserialize"),
        (prereqs.json_schema, "JsonSchema"),
    ]
    .into_iter()
    .filter(|(present, _)| !present)
    .map(|(_, name)| format!("{name}(absent)"))
    .collect()
}

fn build_missing_our_traits_action(
    source_crate: &str,
    type_path: &str,
    missing_our: &[&str],
    missing_external: &[String],
    can_be_direct: bool,
    feature_gated_external: bool,
    unactivated_serde_features: &[String],
) -> String {
    if can_be_direct {
        format!(
            "Add our traits to `{}`: {}; then add `impl ElicitComplete`",
            type_path,
            missing_our.join(", ")
        )
    } else if feature_gated_external {
        format!(
            "Add our traits to `{}`: {}; also enable `{}` features: {}",
            type_path,
            missing_our.join(", "),
            source_crate,
            unactivated_serde_features.join(", ")
        )
    } else {
        format!(
            "Add our traits to `{}`: {}; external blockers remain: {}",
            type_path,
            missing_our.join(", "),
            missing_external.join(", ")
        )
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
    /// A matched shadow type exists, but it is not yet fully `ElicitComplete`.
    ShadowVerificationGap,
}

impl std::fmt::Display for ShadowGapKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "Missing"),
            Self::Drifted => write!(f, "Drifted"),
            Self::PossiblyStale => write!(f, "PossiblyStale"),
            Self::InfrastructureExtra => write!(f, "InfrastructureExtra"),
            Self::ShadowVerificationGap => write!(f, "ShadowVerificationGap"),
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
    pub shadow_elicit_impl: String,
    pub shadow_can_be_direct: String,
    pub shadow_missing_external_traits: String,
    pub shadow_missing_our_traits: String,
    pub action: String,
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
                shadow_elicit_impl: row.shadow_elicit_impl.clone(),
                shadow_can_be_direct: row.shadow_can_be_direct.clone(),
                shadow_missing_external_traits: row.shadow_missing_external_traits.clone(),
                shadow_missing_our_traits: row.shadow_missing_our_traits.clone(),
                action: String::new(),
                notes: row.notes.clone(),
            });
        }

        for row in &report.rows {
            if !is_shadow_verification_gap(row) {
                continue;
            }

            entries.push(ShadowGapEntry {
                target_crate: target_crate.to_string(),
                shadow_crate: shadow_crate.to_string(),
                item_path: row.item_path.clone(),
                item_kind: row.item_kind,
                gap_kind: ShadowGapKind::ShadowVerificationGap,
                matched_shadow_item: row.shadow_item.clone(),
                drift_confidence: row.drift_confidence.clone(),
                shadow_elicit_impl: row.shadow_elicit_impl.clone(),
                shadow_can_be_direct: row.shadow_can_be_direct.clone(),
                shadow_missing_external_traits: row.shadow_missing_external_traits.clone(),
                shadow_missing_our_traits: row.shadow_missing_our_traits.clone(),
                action: build_shadow_verification_action(row),
                notes: row.notes.clone(),
            });
        }
    }

    // Sort: surface gaps first, then verification gaps.
    entries.sort_by(|a, b| {
        shadow_gap_order(&a.gap_kind)
            .cmp(&shadow_gap_order(&b.gap_kind))
            .then(a.target_crate.cmp(&b.target_crate))
            .then(a.item_path.cmp(&b.item_path))
    });

    tracing::info!(
        total_gaps = entries.len(),
        missing = entries
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::Missing)
            .count(),
        drifted = entries
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::Drifted)
            .count(),
        possibly_stale = entries
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::PossiblyStale)
            .count(),
        infra_extra = entries
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::InfrastructureExtra)
            .count(),
        verification = entries
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::ShadowVerificationGap)
            .count(),
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
        ShadowGapKind::ShadowVerificationGap => 4,
    }
}

fn is_shadow_verification_gap(row: &crate::shadow::ShadowRow) -> bool {
    if !matches!(row.status, ShadowStatus::Covered | ShadowStatus::Drifted)
        || !row.item_kind.is_type()
    {
        return false;
    }

    row.shadow_elicit_impl != ImplStatus::Complete.to_string()
        && row.shadow_elicit_impl != ImplStatus::CompleteFactory.to_string()
}

fn build_shadow_verification_action(row: &crate::shadow::ShadowRow) -> String {
    let missing_our = row.shadow_missing_our_traits.as_str();
    let missing_external = row.shadow_missing_external_traits.as_str();
    let can_be_direct = row.shadow_can_be_direct == "true";

    if !missing_our.is_empty() && can_be_direct {
        format!(
            "Finish `{}` by adding our traits: {}; then add `impl ElicitComplete`",
            row.shadow_item,
            missing_our.replace(';', ", ")
        )
    } else if !missing_our.is_empty() {
        format!(
            "Finish `{}` by adding our traits: {}; also add external traits: {}",
            row.shadow_item,
            missing_our.replace(';', ", "),
            missing_external.replace(';', ", ")
        )
    } else if can_be_direct {
        format!("Add `impl ElicitComplete for {} {{}}`", row.shadow_item)
    } else {
        format!(
            "Add external traits to `{}` so it can support `ElicitComplete`: {}",
            row.shadow_item,
            missing_external.replace(';', ", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::collect::TraitPrereqs;
    use crate::impl_coverage::{ImplCoverageEntry, ImplCoverageReport, ImplStatus, TestStatus};
    use crate::inventory::ItemKind;
    use crate::shadow::{ShadowReport, ShadowRow, ShadowStatus};

    fn impl_report_for(prereqs: TraitPrereqs) -> ImplCoverageReport {
        ImplCoverageReport {
            source_crate: "reqwest".to_string(),
            source_version: "1.0.0".to_string(),
            entries: vec![ImplCoverageEntry {
                type_path: "reqwest::Client".to_string(),
                type_kind: ItemKind::Struct,
                is_generic: false,
                lifetime_params: Vec::new(),
                type_params: Vec::new(),
                elicit_impl: ImplStatus::Missing,
                proof_test: TestStatus::Missing,
                composition_test: TestStatus::Missing,
                prereqs,
                notes: String::new(),
            }],
            complete_count: 0,
            missing_impl_count: 1,
            missing_test_count: 1,
            flagged_concrete_count: 0,
        }
    }

    #[test]
    fn skips_orphan_blocked_everything_but_types() {
        let report = impl_report_for(TraitPrereqs {
            serialize: false,
            deserialize: false,
            json_schema: false,
            elicitation_trait: true,
            elicit_introspect: true,
            elicit_spec: true,
            elicit_prompt_tree: true,
            to_code_literal: true,
        });

        let gaps = build_impl_gaps(
            &[("reqwest", &report)],
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(gaps.is_empty());
    }

    #[test]
    fn flags_missing_our_traits_even_when_direct_impl_is_blocked() {
        let report = impl_report_for(TraitPrereqs {
            serialize: false,
            deserialize: false,
            json_schema: false,
            elicitation_trait: true,
            elicit_introspect: false,
            elicit_spec: true,
            elicit_prompt_tree: false,
            to_code_literal: true,
        });

        let gaps = build_impl_gaps(
            &[("reqwest", &report)],
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].gap_kind, ImplGapKind::MissingOurTraits);
        assert_eq!(
            gaps[0].missing_our_traits,
            "ElicitIntrospect;ElicitPromptTree"
        );
        assert!(gaps[0].blocked_by_orphan_rule);
        assert!(!gaps[0].can_be_direct);
    }

    #[test]
    fn lifetime_bound_types_only_flag_implementable_trait_gaps() {
        let report = ImplCoverageReport {
            source_crate: "georaster".to_string(),
            source_version: "1.0.0".to_string(),
            entries: vec![ImplCoverageEntry {
                type_path: "georaster::geotiff::Pixels".to_string(),
                type_kind: ItemKind::Struct,
                is_generic: true,
                lifetime_params: vec!["'a".to_string()],
                type_params: vec!["R".to_string()],
                elicit_impl: ImplStatus::Missing,
                proof_test: TestStatus::Missing,
                composition_test: TestStatus::Missing,
                prereqs: TraitPrereqs {
                    serialize: true,
                    deserialize: true,
                    json_schema: true,
                    elicitation_trait: false,
                    elicit_introspect: false,
                    elicit_spec: false,
                    elicit_prompt_tree: true,
                    to_code_literal: true,
                },
                notes: String::new(),
            }],
            complete_count: 0,
            missing_impl_count: 1,
            missing_test_count: 1,
            flagged_concrete_count: 0,
        };

        let gaps = build_impl_gaps(
            &[("georaster", &report)],
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].gap_kind, ImplGapKind::MissingOurTraits);
        assert_eq!(gaps[0].missing_our_traits, "ElicitSpec");
        assert!(!gaps[0].can_be_direct);
        assert!(!gaps[0].blocked_by_orphan_rule);
        assert!(!gaps[0].elicit_complete_gap);
    }

    #[test]
    fn flags_ready_for_elicit_complete_when_direct_impl_is_legal() {
        let report = impl_report_for(TraitPrereqs {
            serialize: true,
            deserialize: true,
            json_schema: true,
            elicitation_trait: true,
            elicit_introspect: true,
            elicit_spec: true,
            elicit_prompt_tree: true,
            to_code_literal: true,
        });

        let gaps = build_impl_gaps(
            &[("reqwest", &report)],
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].gap_kind, ImplGapKind::ReadyForElicitComplete);
        assert!(gaps[0].can_be_direct);
        assert!(gaps[0].elicit_complete_gap);
    }

    #[test]
    fn flags_feature_gated_external_only_when_probe_unlocks_direct_impl() {
        let report = impl_report_for(TraitPrereqs {
            serialize: false,
            deserialize: false,
            json_schema: false,
            elicitation_trait: true,
            elicit_introspect: false,
            elicit_spec: true,
            elicit_prompt_tree: false,
            to_code_literal: true,
        });

        let mut available = HashMap::new();
        available.insert("reqwest".to_string(), vec!["json".to_string()]);
        let activated = HashMap::new();
        let mut probe_for_crate = HashMap::new();
        probe_for_crate.insert(
            "reqwest::Client".to_string(),
            TraitPrereqs {
                serialize: true,
                deserialize: true,
                json_schema: true,
                elicitation_trait: true,
                elicit_introspect: false,
                elicit_spec: true,
                elicit_prompt_tree: false,
                to_code_literal: true,
            },
        );
        let mut probed = HashMap::new();
        probed.insert("reqwest".to_string(), probe_for_crate);

        let gaps = build_impl_gaps(&[("reqwest", &report)], &available, &activated, &probed);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].gap_kind, ImplGapKind::MissingOurTraits);
        assert!(gaps[0].feature_gated_external);
        assert!(!gaps[0].blocked_by_orphan_rule);
        assert_eq!(gaps[0].candidate_unlock_features, "json");
    }

    #[test]
    fn emits_shadow_verification_gap_for_matched_but_incomplete_shadow_type() {
        let report = ShadowReport {
            target_crate: "reqwest".to_string(),
            target_version: "1.0.0".to_string(),
            shadow_crate: "elicit_reqwest".to_string(),
            shadow_version: "1.0.0".to_string(),
            rows: vec![ShadowRow {
                item_path: "reqwest::Client".to_string(),
                item_kind: ItemKind::Struct,
                status: ShadowStatus::Covered,
                shadow_item: "elicit_reqwest::Client".to_string(),
                drift_confidence: String::new(),
                shadow_elicit_impl: ImplStatus::Missing.to_string(),
                shadow_can_be_direct: "true".to_string(),
                shadow_missing_external_traits: String::new(),
                shadow_missing_our_traits: String::new(),
                notes: String::new(),
            }],
            covered_count: 1,
            missing_count: 0,
            extra_count: 0,
            drifted_count: 0,
            coverage_pct: 100.0,
            verification_gap_count: 1,
        };

        let gaps = build_shadow_gaps(&[("reqwest", "elicit_reqwest", &report)]);
        let verification = gaps
            .iter()
            .find(|g| g.gap_kind == ShadowGapKind::ShadowVerificationGap)
            .expect("expected verification gap");

        assert_eq!(verification.item_path, "reqwest::Client");
        assert_eq!(verification.matched_shadow_item, "elicit_reqwest::Client");
    }
}
