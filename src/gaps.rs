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

use crate::collect::{TraitPrereqs, TypeFeatureProbe};
use crate::error::ElicitDocResult;
use crate::impl_coverage::{ImplCoverageEntry, ImplCoverageReport, ImplStatus};
use crate::inventory::ItemKind;
use crate::shadow::{ShadowReport, ShadowStatus};
use crate::trenchcoat::WrapperCoverage;

// ── Impl gaps ────────────────────────────────────────────────────────────────

/// Full per-row worklist state for a core type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImplRowKind {
    /// Type already has `ElicitComplete`.
    Covered,
    /// Missing one or more elicitation-owned support traits.
    MissingOurTraits,
    /// All prerequisites are present; only `impl ElicitComplete` is missing.
    ReadyForElicitComplete,
    /// Missing external serde/schemars traits may be unlockable via features.
    FeatureGatedExternal,
    /// Direct `ElicitComplete` is not legal under the current upstream API.
    ExternallyBlocked,
}

impl std::fmt::Display for ImplRowKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Covered => write!(f, "Covered"),
            Self::MissingOurTraits => write!(f, "MissingOurTraits"),
            Self::ReadyForElicitComplete => write!(f, "ReadyForElicitComplete"),
            Self::FeatureGatedExternal => write!(f, "FeatureGatedExternal"),
            Self::ExternallyBlocked => write!(f, "ExternallyBlocked"),
        }
    }
}

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
    /// All five elicitation-owned support traits are present after direct + wrapper coverage.
    pub our_traits_complete: bool,
    /// Which coverage path currently satisfies the row, if any (`direct`, `wrapper`, `hybrid`).
    pub coverage_provider: String,
    /// Wrapper paths that contribute indirect coverage for this type.
    pub wrapper_paths: String,
    /// The external prerequisites are present, so `ElicitComplete` is legal.
    pub can_be_direct: bool,
    /// This type is a real `ElicitComplete` gap: legal to impl directly, but missing it.
    pub elicit_complete_gap: bool,
    /// External support is missing and the dep exposes candidate features that may unlock it.
    pub feature_gated_external: bool,
    /// External support is missing under the current feature set, so the orphan rule blocks
    /// a direct `ElicitComplete` impl.
    pub blocked_by_orphan_rule: bool,
    /// Cargo package whose features may unlock upstream serde/schemars support.
    pub feature_owner_crate: String,
    /// Feature flags available in the dep that could unlock serde/schemars support.
    ///
    /// Only populated when external support is feature-gated.  Semicolon-separated.
    pub candidate_unlock_features: String,
    /// Short recommended action.
    pub action: String,
}

/// Full diagnosis for one per-crate impl coverage row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplRowAssessment {
    pub row_kind: ImplRowKind,
    pub primary_gap_kind: Option<ImplGapKind>,
    pub missing_external_traits: String,
    pub missing_our_traits: String,
    pub direct_missing_our_traits: String,
    pub our_traits_complete: bool,
    pub direct_our_traits_complete: bool,
    pub covered_indirectly: bool,
    pub coverage_provider: String,
    pub wrapper_paths: String,
    pub can_be_direct: bool,
    pub elicit_complete_gap: bool,
    pub feature_gated_external: bool,
    pub blocked_by_orphan_rule: bool,
    pub blocked_reason: String,
    pub feature_owner_crate: String,
    pub candidate_unlock_features: String,
    pub action: String,
}

/// Build the consolidated impl gaps list from multiple per-crate reports.
///
/// Only types with `ImplStatus::Missing` are candidates for inclusion. Fully-covered,
/// orphan-blocked "everything but" types are excluded because they are not actionable gaps.
///
#[instrument(skip(pairs, type_feature_probes), fields(num_reports = pairs.len()))]
pub fn build_impl_gaps(
    pairs: &[(&str, &ImplCoverageReport)],
    type_feature_probes: &std::collections::HashMap<
        String,
        std::collections::HashMap<String, TypeFeatureProbe>,
    >,
    wrapper_coverage: &std::collections::HashMap<String, Vec<WrapperCoverage>>,
) -> Vec<ImplGapEntry> {
    let mut entries = Vec::new();

    for (source_crate, report) in pairs {
        for entry in &report.entries {
            if !matches!(entry.elicit_impl, ImplStatus::Missing) {
                continue;
            }

            let feature_probe = type_feature_probes
                .get(*source_crate)
                .and_then(|m| m.get(&entry.type_path));
            let wrappers = wrapper_coverage.get(&entry.type_path).map(Vec::as_slice);
            if let Some(gap_entry) = classify_impl_gap(source_crate, entry, feature_probe, wrappers)
            {
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
/// `feature_probe` carries row-specific report-crate feature context for surfaced
/// API types. When the probe shows that additional target-crate features would make
/// the external serde / schemars traits appear, the row is classified as
/// `FeatureGatedExternal` instead of a true orphan-rule blocker.
#[instrument(skip(entry, feature_probe, wrappers), fields(source_crate, type_path = %entry.type_path))]
pub fn assess_impl_entry(
    source_crate: &str,
    entry: &ImplCoverageEntry,
    feature_probe: Option<&TypeFeatureProbe>,
    wrappers: Option<&[WrapperCoverage]>,
) -> ImplRowAssessment {
    let wrapper_paths = wrappers
        .unwrap_or(&[])
        .iter()
        .map(|wrapper| wrapper.wrapper_path.as_str())
        .collect::<Vec<_>>()
        .join(";");
    if matches!(
        entry.elicit_impl,
        ImplStatus::Complete | ImplStatus::CompleteFactory
    ) {
        return ImplRowAssessment {
            row_kind: ImplRowKind::Covered,
            primary_gap_kind: None,
            missing_external_traits: String::new(),
            missing_our_traits: String::new(),
            direct_missing_our_traits: String::new(),
            our_traits_complete: true,
            direct_our_traits_complete: true,
            covered_indirectly: false,
            coverage_provider: "direct".to_string(),
            wrapper_paths,
            can_be_direct: entry.can_be_direct(),
            elicit_complete_gap: false,
            feature_gated_external: false,
            blocked_by_orphan_rule: false,
            blocked_reason: String::new(),
            feature_owner_crate: String::new(),
            candidate_unlock_features: String::new(),
            action: String::new(),
        };
    }

    let p = &entry.prereqs;
    let candidate_unlock_features = feature_probe
        .map(|probe| probe.candidate_unlock_features.as_slice())
        .unwrap_or(&[]);
    let feature_owner_crate = feature_probe
        .map(|probe| probe.feature_crate.clone())
        .unwrap_or_else(|| source_crate.to_string());

    let missing_external = missing_external_traits(p);
    let direct_missing_our = entry.effective_missing_our_traits();
    let direct_our_traits_complete = direct_missing_our.is_empty();
    let covered_indirectly = wrappers.is_some_and(|known| {
        known.iter().any(|wrapper| {
            wrapper.wrapper_elicit_complete || wrapper.wrapper_prereqs.our_traits_complete()
        })
    });
    let missing_our = effective_missing_our_traits(&direct_missing_our, wrappers);
    let our_traits_complete = missing_our.is_empty();
    let can_be_direct = entry.can_be_direct();
    let feature_gated_external = !entry.lifetime_blocks_elicitation()
        && !can_be_direct
        && !candidate_unlock_features.is_empty()
        && feature_probe
            .and_then(|probe| probe.probed_prereqs.as_ref())
            .is_some_and(TraitPrereqs::can_be_direct);
    let blocked_by_orphan_rule =
        !entry.lifetime_blocks_elicitation() && !can_be_direct && !feature_gated_external;
    let indirect_elicit_complete = wrappers
        .unwrap_or(&[])
        .iter()
        .any(|wrapper| wrapper.wrapper_elicit_complete);
    let elicit_complete_gap = can_be_direct && !indirect_elicit_complete;
    let coverage_provider = determine_coverage_provider(
        &entry.elicit_impl,
        direct_our_traits_complete,
        indirect_elicit_complete,
        our_traits_complete,
    );
    let (row_kind, primary_gap_kind) = if indirect_elicit_complete {
        (ImplRowKind::Covered, None)
    } else if !our_traits_complete {
        (
            ImplRowKind::MissingOurTraits,
            Some(ImplGapKind::MissingOurTraits),
        )
    } else if elicit_complete_gap {
        (
            ImplRowKind::ReadyForElicitComplete,
            Some(ImplGapKind::ReadyForElicitComplete),
        )
    } else if feature_gated_external {
        (
            ImplRowKind::FeatureGatedExternal,
            Some(ImplGapKind::FeatureGatedExternal),
        )
    } else {
        (ImplRowKind::ExternallyBlocked, None)
    };

    let blocked_reason = build_impl_blocked_reason(
        entry,
        &missing_external,
        &direct_missing_our,
        &missing_our,
        feature_gated_external,
        blocked_by_orphan_rule,
        &feature_owner_crate,
        candidate_unlock_features,
        wrappers,
    );
    let action = build_impl_action(
        entry,
        &row_kind,
        &missing_our,
        &direct_missing_our,
        &missing_external,
        can_be_direct,
        feature_gated_external,
        &feature_owner_crate,
        candidate_unlock_features,
        wrappers,
    );

    ImplRowAssessment {
        row_kind,
        primary_gap_kind,
        missing_external_traits: missing_external.join(";"),
        missing_our_traits: missing_our.join(";"),
        direct_missing_our_traits: direct_missing_our.join(";"),
        our_traits_complete,
        direct_our_traits_complete,
        covered_indirectly,
        coverage_provider,
        wrapper_paths,
        can_be_direct,
        elicit_complete_gap,
        feature_gated_external,
        blocked_by_orphan_rule,
        blocked_reason,
        feature_owner_crate,
        candidate_unlock_features: if feature_gated_external {
            candidate_unlock_features.join(";")
        } else {
            String::new()
        },
        action,
    }
}

fn classify_impl_gap(
    source_crate: &str,
    entry: &ImplCoverageEntry,
    feature_probe: Option<&TypeFeatureProbe>,
    wrappers: Option<&[WrapperCoverage]>,
) -> Option<ImplGapEntry> {
    let assessment = assess_impl_entry(source_crate, entry, feature_probe, wrappers);
    let gap_kind = assessment.primary_gap_kind.clone()?;

    Some(ImplGapEntry {
        source_crate: source_crate.to_string(),
        type_path: entry.type_path.clone(),
        type_kind: entry.type_kind,
        is_generic: entry.is_generic,
        gap_kind,
        missing_external_traits: assessment.missing_external_traits,
        missing_our_traits: assessment.missing_our_traits,
        our_traits_complete: assessment.our_traits_complete,
        coverage_provider: assessment.coverage_provider,
        wrapper_paths: assessment.wrapper_paths,
        can_be_direct: assessment.can_be_direct,
        elicit_complete_gap: assessment.elicit_complete_gap,
        feature_gated_external: assessment.feature_gated_external,
        blocked_by_orphan_rule: assessment.blocked_by_orphan_rule,
        feature_owner_crate: assessment.feature_owner_crate,
        candidate_unlock_features: assessment.candidate_unlock_features,
        action: assessment.action,
    })
}

fn merge_wrapper_prereqs(wrappers: Option<&[WrapperCoverage]>) -> TraitPrereqs {
    let mut merged = TraitPrereqs::default();
    for wrapper in wrappers.unwrap_or(&[]) {
        merged.merge(&wrapper.wrapper_prereqs);
    }
    merged
}

fn effective_missing_our_traits<'a>(
    direct_missing_our: &'a [&'static str],
    wrappers: Option<&[WrapperCoverage]>,
) -> Vec<&'a str> {
    let merged_wrapper_prereqs = merge_wrapper_prereqs(wrappers);
    direct_missing_our
        .iter()
        .copied()
        .filter(|trait_name| match *trait_name {
            "Elicitation" => !merged_wrapper_prereqs.elicitation_trait,
            "ElicitIntrospect" => !merged_wrapper_prereqs.elicit_introspect,
            "ElicitSpec" => !merged_wrapper_prereqs.elicit_spec,
            "ElicitPromptTree" => !merged_wrapper_prereqs.elicit_prompt_tree,
            "ToCodeLiteral" => !merged_wrapper_prereqs.to_code_literal,
            _ => true,
        })
        .collect()
}

fn determine_coverage_provider(
    impl_status: &ImplStatus,
    direct_our_traits_complete: bool,
    indirect_elicit_complete: bool,
    effective_our_traits_complete: bool,
) -> String {
    let direct_complete = matches!(
        impl_status,
        ImplStatus::Complete | ImplStatus::CompleteFactory
    ) || direct_our_traits_complete;
    match (
        direct_complete,
        indirect_elicit_complete || effective_our_traits_complete && !direct_our_traits_complete,
    ) {
        (true, true) => "hybrid".to_string(),
        (true, false) => "direct".to_string(),
        (false, true) => "wrapper".to_string(),
        (false, false) => String::new(),
    }
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
    type_path: &str,
    missing_our: &[&str],
    direct_missing_our: &[&str],
    missing_external: &[String],
    can_be_direct: bool,
    feature_gated_external: bool,
    feature_owner_crate: &str,
    candidate_unlock_features: &[String],
    wrappers: Option<&[WrapperCoverage]>,
) -> String {
    let wrapper_paths = wrappers
        .unwrap_or(&[])
        .iter()
        .map(|wrapper| wrapper.wrapper_path.as_str())
        .collect::<Vec<_>>();
    let wrapper_note = if wrapper_paths.is_empty() {
        String::new()
    } else {
        format!(
            " Known wrappers for indirect coverage: {}.",
            wrapper_paths.join(", ")
        )
    };

    if !wrapper_paths.is_empty() && missing_our.len() < direct_missing_our.len() {
        format!(
            "Finish indirect coverage for `{}` by completing wrapper-provided traits: {}.{}",
            type_path,
            missing_our.join(", "),
            wrapper_note
        )
    } else if can_be_direct {
        format!(
            "Add direct support traits to `{}`: {}; then add `impl ElicitComplete`{}",
            type_path,
            missing_our.join(", "),
            wrapper_note
        )
    } else if feature_gated_external {
        format!(
            "Add support traits to `{}`: {}; also enable `{}` features: {}.{}",
            type_path,
            missing_our.join(", "),
            feature_owner_crate,
            candidate_unlock_features.join(", "),
            wrapper_note
        )
    } else {
        format!(
            "Add support traits to `{}`: {}; external blockers remain: {}.{}",
            type_path,
            missing_our.join(", "),
            missing_external.join(", "),
            wrapper_note
        )
    }
}

fn build_impl_action(
    entry: &ImplCoverageEntry,
    row_kind: &ImplRowKind,
    missing_our: &[&str],
    direct_missing_our: &[&str],
    missing_external: &[String],
    can_be_direct: bool,
    feature_gated_external: bool,
    feature_owner_crate: &str,
    candidate_unlock_features: &[String],
    wrappers: Option<&[WrapperCoverage]>,
) -> String {
    match row_kind {
        ImplRowKind::Covered => String::new(),
        ImplRowKind::MissingOurTraits => build_missing_our_traits_action(
            &entry.type_path,
            missing_our,
            direct_missing_our,
            missing_external,
            can_be_direct,
            feature_gated_external,
            feature_owner_crate,
            candidate_unlock_features,
            wrappers,
        ),
        ImplRowKind::ReadyForElicitComplete => format!(
            "Add `impl ElicitComplete for {} {{}}` in elicitation crate",
            entry.type_path
        ),
        ImplRowKind::FeatureGatedExternal => format!(
            "Enable additional `{feature_owner_crate}` features in the workspace: {}; \
             then re-check whether `ElicitComplete` becomes legal for `{}`",
            candidate_unlock_features.join(", "),
            entry.type_path
        ),
        ImplRowKind::ExternallyBlocked => format!(
            "`{}` is everything-but complete. Do not flag missing `ElicitComplete`; direct impl is blocked by upstream traits: {}",
            entry.type_path,
            missing_external.join(", ")
        ),
    }
}

fn build_impl_blocked_reason(
    entry: &ImplCoverageEntry,
    missing_external: &[String],
    direct_missing_our: &[&str],
    effective_missing_our: &[&str],
    feature_gated_external: bool,
    blocked_by_orphan_rule: bool,
    feature_owner_crate: &str,
    candidate_unlock_features: &[String],
    wrappers: Option<&[WrapperCoverage]>,
) -> String {
    if !direct_missing_our.is_empty() && effective_missing_our.len() < direct_missing_our.len() {
        let wrapper_paths = wrappers
            .unwrap_or(&[])
            .iter()
            .map(|wrapper| wrapper.wrapper_path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return format!(
            "wrapper coverage reduces direct trait gaps; remaining uncovered traits: {} (wrappers: {})",
            effective_missing_our.join(", "),
            wrapper_paths
        );
    }
    if entry.lifetime_blocks_elicitation() {
        "lifetime-bound type: direct `ElicitComplete` is illegal because `Elicitation` requires `'static`".to_string()
    } else if feature_gated_external {
        format!(
            "workspace is not enabling candidate `{feature_owner_crate}` features that may unlock external traits: {}",
            candidate_unlock_features.join(", ")
        )
    } else if blocked_by_orphan_rule {
        format!(
            "orphan rule blocks direct `ElicitComplete` until upstream provides: {}",
            missing_external.join(", ")
        )
    } else {
        String::new()
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

/// Full per-row worklist state for a shadow coverage row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowCoverageKind {
    Covered,
    Missing,
    Drifted,
    PossiblyStale,
    InfrastructureExtra,
}

impl std::fmt::Display for ShadowCoverageKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Covered => write!(f, "Covered"),
            Self::Missing => write!(f, "Missing"),
            Self::Drifted => write!(f, "Drifted"),
            Self::PossiblyStale => write!(f, "PossiblyStale"),
            Self::InfrastructureExtra => write!(f, "InfrastructureExtra"),
        }
    }
}

/// Full diagnosis for one per-crate shadow coverage row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowRowAssessment {
    pub coverage_kind: ShadowCoverageKind,
    pub primary_gap_kind: Option<ShadowGapKind>,
    pub verification_gap: bool,
    pub verification_ready: bool,
    pub action: String,
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

#[instrument(skip(row), fields(item_path = %row.item_path))]
pub fn assess_shadow_row(row: &crate::shadow::ShadowRow) -> ShadowRowAssessment {
    let coverage_kind = match row.status {
        ShadowStatus::Covered => ShadowCoverageKind::Covered,
        ShadowStatus::Missing => ShadowCoverageKind::Missing,
        ShadowStatus::Drifted => ShadowCoverageKind::Drifted,
        ShadowStatus::Extra => {
            let bare = row.item_path.split("::").last().unwrap_or(&row.item_path);
            if is_infrastructure_name(bare) {
                ShadowCoverageKind::InfrastructureExtra
            } else {
                ShadowCoverageKind::PossiblyStale
            }
        }
    };
    let primary_gap_kind = match coverage_kind {
        ShadowCoverageKind::Covered => None,
        ShadowCoverageKind::Missing => Some(ShadowGapKind::Missing),
        ShadowCoverageKind::Drifted => Some(ShadowGapKind::Drifted),
        ShadowCoverageKind::PossiblyStale => Some(ShadowGapKind::PossiblyStale),
        ShadowCoverageKind::InfrastructureExtra => Some(ShadowGapKind::InfrastructureExtra),
    };
    let verification_gap = is_shadow_verification_gap(row);
    let verification_ready = matches!(
        row.shadow_elicit_impl.as_str(),
        "Complete" | "CompleteFactory"
    );

    let mut action = match coverage_kind {
        ShadowCoverageKind::Covered => String::new(),
        ShadowCoverageKind::Missing => {
            if row.item_kind.is_type() {
                format!(
                    "Add a shadow for upstream `{}` and make the new wrapper `ElicitComplete`",
                    row.item_path
                )
            } else {
                format!(
                    "Add a shadow item for upstream `{}` so the full public API surface is represented",
                    row.item_path
                )
            }
        }
        ShadowCoverageKind::Drifted => format!(
            "Rename or replace `{}` so upstream `{}` is shadowed exactly",
            row.shadow_item, row.item_path
        ),
        ShadowCoverageKind::PossiblyStale => format!(
            "Audit `{}`: remove it if stale, or rename/remap it to an upstream public item",
            row.item_path
        ),
        ShadowCoverageKind::InfrastructureExtra => {
            "Shadow-only infrastructure item; keep unless it should instead map to an upstream API item".to_string()
        }
    };

    if verification_gap {
        let verification_action = build_shadow_verification_action(row);
        if action.is_empty() {
            action = verification_action;
        } else {
            action.push_str("; then ");
            action.push_str(&verification_action);
        }
    }

    ShadowRowAssessment {
        coverage_kind,
        primary_gap_kind,
        verification_gap,
        verification_ready,
        action,
    }
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
pub fn build_shadow_gaps(
    pairs: &[(&str, &str, &ShadowReport)],
) -> ElicitDocResult<Vec<ShadowGapEntry>> {
    let mut entries = Vec::new();

    for (target_crate, shadow_crate, report) in pairs {
        for row in &report.rows {
            let assessment = assess_shadow_row(row);
            let gap_kind = match row.status {
                ShadowStatus::Covered => continue,
                _ => match assessment.primary_gap_kind {
                    Some(gap_kind) => gap_kind,
                    None => {
                        return Err(crate::error::ElicitDocError::invariant(format!(
                            "non-covered shadow row `{}` ({:?}, {:?}) did not classify to a gap kind",
                            row.item_path, row.item_kind, row.status
                        )));
                    }
                },
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

    Ok(entries)
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
    use crate::collect::{TraitPrereqs, TypeFeatureProbe};
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

        let gaps = build_impl_gaps(&[("reqwest", &report)], &HashMap::new(), &HashMap::new());
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

        let gaps = build_impl_gaps(&[("reqwest", &report)], &HashMap::new(), &HashMap::new());
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

        let gaps = build_impl_gaps(&[("georaster", &report)], &HashMap::new(), &HashMap::new());
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

        let gaps = build_impl_gaps(&[("reqwest", &report)], &HashMap::new(), &HashMap::new());
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

        let mut probe_for_crate = HashMap::new();
        probe_for_crate.insert(
            "reqwest::Client".to_string(),
            TypeFeatureProbe {
                feature_crate: "reqwest".to_string(),
                candidate_unlock_features: vec!["json".to_string()],
                probed_prereqs: Some(TraitPrereqs {
                    serialize: true,
                    deserialize: true,
                    json_schema: true,
                    elicitation_trait: true,
                    elicit_introspect: false,
                    elicit_spec: true,
                    elicit_prompt_tree: false,
                    to_code_literal: true,
                }),
            },
        );
        let mut type_probes = HashMap::new();
        type_probes.insert("reqwest".to_string(), probe_for_crate);

        let gaps = build_impl_gaps(&[("reqwest", &report)], &type_probes, &HashMap::new());
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].gap_kind, ImplGapKind::MissingOurTraits);
        assert!(gaps[0].feature_gated_external);
        assert!(!gaps[0].blocked_by_orphan_rule);
        assert_eq!(gaps[0].feature_owner_crate, "reqwest");
        assert_eq!(gaps[0].candidate_unlock_features, "json");
    }

    #[test]
    fn marks_everything_but_rows_as_externally_blocked_not_gap() {
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

        let assessment = assess_impl_entry("reqwest", &report.entries[0], None, None);
        assert_eq!(assessment.row_kind, ImplRowKind::ExternallyBlocked);
        assert!(assessment.primary_gap_kind.is_none());
        assert!(assessment.blocked_by_orphan_rule);
        assert!(assessment.action.contains("everything-but complete"));
    }

    #[test]
    fn feature_gated_foreign_types_point_to_owner_crate() {
        let report = ImplCoverageReport {
            source_crate: "reqwest".to_string(),
            source_version: "1.0.0".to_string(),
            entries: vec![ImplCoverageEntry {
                type_path: "http::header::value::HeaderValue".to_string(),
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
                    elicit_spec: true,
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
        let mut probe_for_crate = HashMap::new();
        probe_for_crate.insert(
            "http::header::value::HeaderValue".to_string(),
            TypeFeatureProbe {
                feature_crate: "reqwest".to_string(),
                candidate_unlock_features: vec!["json".to_string()],
                probed_prereqs: Some(TraitPrereqs {
                    serialize: true,
                    deserialize: true,
                    json_schema: true,
                    elicitation_trait: true,
                    elicit_introspect: true,
                    elicit_spec: true,
                    elicit_prompt_tree: true,
                    to_code_literal: true,
                }),
            },
        );
        let mut type_probes = HashMap::new();
        type_probes.insert("reqwest".to_string(), probe_for_crate);

        let assessment = assess_impl_entry(
            "reqwest",
            &report.entries[0],
            type_probes
                .get("reqwest")
                .and_then(|probes| probes.get("http::header::value::HeaderValue")),
            None,
        );

        assert_eq!(assessment.row_kind, ImplRowKind::FeatureGatedExternal);
        assert_eq!(assessment.feature_owner_crate, "reqwest");
        assert!(assessment.action.contains("`reqwest` features"));
    }

    #[test]
    fn wrapper_elicit_complete_counts_as_effective_coverage() {
        let report = impl_report_for(TraitPrereqs {
            serialize: false,
            deserialize: false,
            json_schema: false,
            elicitation_trait: false,
            elicit_introspect: false,
            elicit_spec: false,
            elicit_prompt_tree: false,
            to_code_literal: false,
        });
        let mut wrappers = HashMap::new();
        wrappers.insert(
            "reqwest::Client".to_string(),
            vec![WrapperCoverage {
                wrapper_path: "elicitation::ReqwestClientCoat".to_string(),
                wrapper_elicit_complete: true,
                wrapper_prereqs: TraitPrereqs {
                    serialize: true,
                    deserialize: true,
                    json_schema: true,
                    elicitation_trait: true,
                    elicit_introspect: true,
                    elicit_spec: true,
                    elicit_prompt_tree: true,
                    to_code_literal: true,
                },
            }],
        );

        let gaps = build_impl_gaps(&[("reqwest", &report)], &HashMap::new(), &wrappers);
        assert!(gaps.is_empty());

        let assessment = assess_impl_entry(
            "reqwest",
            &report.entries[0],
            None,
            wrappers.get("reqwest::Client").map(Vec::as_slice),
        );
        assert_eq!(assessment.row_kind, ImplRowKind::Covered);
        assert!(assessment.covered_indirectly);
        assert_eq!(assessment.coverage_provider, "wrapper");
        assert_eq!(assessment.wrapper_paths, "elicitation::ReqwestClientCoat");
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

        let gaps_result = build_shadow_gaps(&[("reqwest", "elicit_reqwest", &report)]);
        assert!(
            gaps_result.is_ok(),
            "shadow gaps should build: {gaps_result:?}"
        );
        let gaps = gaps_result.unwrap_or_default();
        let verification = gaps
            .iter()
            .find(|g| g.gap_kind == ShadowGapKind::ShadowVerificationGap)
            .cloned();
        assert!(verification.is_some(), "expected verification gap");
        let verification = verification.unwrap_or_else(|| unreachable!());

        assert_eq!(verification.item_path, "reqwest::Client");
        assert_eq!(verification.matched_shadow_item, "elicit_reqwest::Client");
    }

    #[test]
    fn classifies_shadow_extras_between_stale_and_infrastructure() {
        let stale = ShadowRow {
            item_path: "elicit_reqwest::workflow::FetchResult".to_string(),
            item_kind: ItemKind::Struct,
            status: ShadowStatus::Extra,
            shadow_item: String::new(),
            drift_confidence: String::new(),
            shadow_elicit_impl: String::new(),
            shadow_can_be_direct: String::new(),
            shadow_missing_external_traits: String::new(),
            shadow_missing_our_traits: String::new(),
            notes: String::new(),
        };
        let infra = ShadowRow {
            item_path: "elicit_reqwest::client::GetParams".to_string(),
            item_kind: ItemKind::Struct,
            status: ShadowStatus::Extra,
            shadow_item: String::new(),
            drift_confidence: String::new(),
            shadow_elicit_impl: String::new(),
            shadow_can_be_direct: String::new(),
            shadow_missing_external_traits: String::new(),
            shadow_missing_our_traits: String::new(),
            notes: String::new(),
        };

        let stale_assessment = assess_shadow_row(&stale);
        let infra_assessment = assess_shadow_row(&infra);
        assert_eq!(
            stale_assessment.coverage_kind,
            ShadowCoverageKind::PossiblyStale
        );
        assert_eq!(
            infra_assessment.coverage_kind,
            ShadowCoverageKind::InfrastructureExtra
        );
    }
}
