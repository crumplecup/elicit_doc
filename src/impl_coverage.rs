//! Impl coverage analysis — use cases 1 and 2.
//!
//! Given a source [`Inventory`] (std, uuid, chrono, or elicitation itself) and
//! an `elicitation` inventory, produces an [`ImplCoverageReport`] showing which
//! types have `ElicitComplete` impls and harness test entries.

use std::collections::HashMap;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::collect::{ElicitCompleteSet, TraitPrereqs};
use crate::inventory::{Inventory, Item, ItemKind};

/// Scanned contents of the proof harness test files.
#[derive(Debug, Clone, Default)]
pub struct ProofHarness {
    /// Type name strings found in `assert_proofs_non_empty::<T>()` calls.
    pub non_empty_types: HashSet<String>,
    /// `(Outer, Inner)` pairs from `assert_kani_contains::<Outer, Inner>()`.
    pub composition_pairs: Vec<(String, String)>,
}

/// Whether a source type has an `ElicitComplete` impl in `elicitation`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImplStatus {
    /// Concrete `impl ElicitComplete for T {}` present.
    Complete,
    /// Generic factory impl: `impl<T: ElicitComplete> ElicitComplete for Wrapper<T>`.
    CompleteFactory,
    /// Some sub-traits implemented but `ElicitComplete` marker absent.
    Partial,
    /// No impl found.
    Missing,
}

impl std::fmt::Display for ImplStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete => write!(f, "Complete"),
            Self::CompleteFactory => write!(f, "CompleteFactory"),
            Self::Partial => write!(f, "Partial"),
            Self::Missing => write!(f, "Missing"),
        }
    }
}

/// Whether a type has a proof harness test entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestStatus {
    /// `assert_proofs_non_empty::<T>()` call found (with matching type).
    Covered,
    /// Factory impl exists but only a concrete instantiation is tested.
    /// The `instantiation` field records what was found, e.g. `"VecNonEmpty<bool>"`.
    CoveredConcrete { instantiation: String },
    /// No harness entry found.
    Missing,
}

impl std::fmt::Display for TestStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Covered => write!(f, "Covered"),
            Self::CoveredConcrete { instantiation } => {
                write!(f, "CoveredConcrete({})", instantiation)
            }
            Self::Missing => write!(f, "Missing"),
        }
    }
}

/// One row in an impl coverage report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplCoverageEntry {
    pub type_path: String,
    pub type_kind: ItemKind,
    pub is_generic: bool,
    pub lifetime_params: Vec<String>,
    pub type_params: Vec<String>,
    pub elicit_impl: ImplStatus,
    pub proof_test: TestStatus,
    pub composition_test: TestStatus,
    /// Which of the 8 `ElicitComplete` supertraits this type already has.
    pub prereqs: TraitPrereqs,
    /// Human-readable note, e.g. the concrete instantiation used in the harness.
    pub notes: String,
}

impl ImplCoverageEntry {
    /// `Elicitation` requires `'static`, so lifetime-parameterized types can never
    /// directly implement it or the `ElicitIntrospect: Elicitation` supertrait.
    pub fn lifetime_blocks_elicitation(&self) -> bool {
        !self.lifetime_params.is_empty()
    }

    /// Missing elicitation-owned traits after removing those blocked by the type's shape.
    pub fn effective_missing_our_traits(&self) -> Vec<&'static str> {
        self.prereqs
            .missing_our_traits()
            .into_iter()
            .filter(|trait_name| {
                !self.lifetime_blocks_elicitation()
                    || !matches!(*trait_name, "Elicitation" | "ElicitIntrospect")
            })
            .collect()
    }

    /// True when every elicitation-owned trait that can be implemented is present.
    pub fn effective_our_traits_complete(&self) -> bool {
        self.effective_missing_our_traits().is_empty()
    }

    /// True when this type can legally receive a direct `ElicitComplete` impl.
    pub fn can_be_direct(&self) -> bool {
        !self.lifetime_blocks_elicitation() && self.prereqs.can_be_direct()
    }
}

/// Full impl coverage report for one source crate.
#[derive(Debug, Clone)]
pub struct ImplCoverageReport {
    pub source_crate: String,
    pub source_version: String,
    pub entries: Vec<ImplCoverageEntry>,
    pub complete_count: usize,
    pub missing_impl_count: usize,
    pub missing_test_count: usize,
    pub flagged_concrete_count: usize,
}

impl ImplCoverageReport {
    /// Count entries whose implementable elicitation-owned traits are all present.
    pub fn coverage_complete_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.effective_our_traits_complete())
            .count()
    }

    /// Coverage percentage based on implementable traits, not only direct `ElicitComplete`.
    pub fn coverage_pct(&self) -> f32 {
        let total = self.entries.len();
        if total == 0 {
            0.0
        } else {
            self.coverage_complete_count() as f32 / total as f32 * 100.0
        }
    }

    /// Summary line for CLI output.
    pub fn summary(&self) -> String {
        let coverage_complete = self.coverage_complete_count();
        format!(
            "{} {} ({} coverage-complete, {} direct ElicitComplete, {} missing impl, {} missing test, {} flagged concrete)",
            self.source_crate,
            self.source_version,
            coverage_complete,
            self.complete_count,
            self.missing_impl_count,
            self.missing_test_count,
            self.flagged_concrete_count,
        )
    }
}

/// Build an [`ImplCoverageReport`] by cross-referencing a source [`Inventory`]
/// against the real `ElicitComplete` impl set and scanned proof harnesses.
///
/// `source` is the crate being checked (std, uuid, chrono, elicitation internal…).
/// `complete` is the set extracted from elicitation's rustdoc JSON — use
/// [`crate::collect::collect_elicit_complete_paths`] to build it.
/// `harness` is the scanned `proof_non_empty_test.rs` + `proof_composition_test.rs`.
/// `prereqs_map` maps canonical type paths to their existing trait impls; built by
/// [`crate::collect::collect_trait_prereqs`] from the dep's and elicitation's JSON.
/// `all_features_build` should be `true` when the dep was documented with
/// `--all-features`; when `false`, missing traits may be feature-gated.
#[instrument(skip(source, complete, harness, prereqs_map), fields(source_crate = %source.crate_name))]
pub fn build_impl_coverage_report(
    source: &Inventory,
    complete: &ElicitCompleteSet,
    harness: &ProofHarness,
    prereqs_map: &HashMap<String, TraitPrereqs>,
) -> ImplCoverageReport {
    let mut entries: Vec<ImplCoverageEntry> = Vec::new();
    let unique_type_names = collect_unique_type_names(source);

    for item in source.type_items() {
        let path_str = item.path_str();
        let bare_name = &item.name;

        let elicit_impl = determine_impl_status(item, complete);

        let (proof_test, composition_test, notes) =
            determine_test_status(item, &elicit_impl, harness);

        let prereqs = lookup_trait_prereqs(item, source, prereqs_map, &unique_type_names)
            .cloned()
            .unwrap_or_default();

        entries.push(ImplCoverageEntry {
            type_path: path_str,
            type_kind: item.kind,
            is_generic: item.is_generic,
            lifetime_params: item.lifetime_params.clone(),
            type_params: item.type_params.clone(),
            elicit_impl,
            proof_test,
            composition_test,
            prereqs,
            notes,
        });

        let _ = bare_name; // used via item.name above
    }

    entries.sort_by(|a, b| a.type_path.cmp(&b.type_path));

    let complete_count = entries
        .iter()
        .filter(|e| {
            matches!(
                e.elicit_impl,
                ImplStatus::Complete | ImplStatus::CompleteFactory
            )
        })
        .count();
    let missing_impl_count = entries
        .iter()
        .filter(|e| matches!(e.elicit_impl, ImplStatus::Missing))
        .count();
    let missing_test_count = entries
        .iter()
        .filter(|e| matches!(e.proof_test, TestStatus::Missing))
        .count();
    let flagged_concrete_count = entries
        .iter()
        .filter(|e| matches!(e.proof_test, TestStatus::CoveredConcrete { .. }))
        .count();

    tracing::info!(
        source = %source.crate_name,
        complete = complete_count,
        missing_impl = missing_impl_count,
        missing_test = missing_test_count,
        flagged = flagged_concrete_count,
        "built impl coverage report"
    );

    ImplCoverageReport {
        source_crate: source.crate_name.clone(),
        source_version: source.crate_version.clone(),
        entries,
        complete_count,
        missing_impl_count,
        missing_test_count,
        flagged_concrete_count,
    }
}

fn collect_unique_type_names(source: &Inventory) -> HashMap<String, usize> {
    let mut counts = HashMap::new();

    for item in source.type_items() {
        *counts.entry(item.name.clone()).or_insert(0) += 1;
    }

    counts
}

fn lookup_trait_prereqs<'a>(
    item: &Item,
    source: &Inventory,
    prereqs_map: &'a HashMap<String, TraitPrereqs>,
    unique_type_names: &HashMap<String, usize>,
) -> Option<&'a TraitPrereqs> {
    let canonical_path = item.path_str();
    if let Some(prereqs) = prereqs_map.get(&canonical_path) {
        return Some(prereqs);
    }

    let root_alias = root_reexport_alias(item, source, unique_type_names)?;
    prereqs_map.get(&root_alias)
}

fn root_reexport_alias(
    item: &Item,
    source: &Inventory,
    unique_type_names: &HashMap<String, usize>,
) -> Option<String> {
    if unique_type_names.get(&item.name).copied() != Some(1) {
        return None;
    }

    let crate_root = source.crate_name.replace('-', "_");
    if item.path.first().map(String::as_str) != Some(crate_root.as_str()) {
        return None;
    }

    let alias = format!("{crate_root}::{}", item.name);
    (alias != item.path_str()).then_some(alias)
}

/// Determine the [`ImplStatus`] for a source type by checking the exact
/// `ElicitComplete` impl paths extracted from elicitation's rustdoc JSON.
///
/// Matching is by full qualified path (e.g. `"uuid::Uuid"`, `"elicitation::AlignSelect"`).
fn determine_impl_status(item: &Item, complete: &ElicitCompleteSet) -> ImplStatus {
    let full_path = item.path_str();

    if complete.factory.contains(&full_path) {
        ImplStatus::CompleteFactory
    } else if complete.concrete.contains(&full_path) {
        ImplStatus::Complete
    } else {
        ImplStatus::Missing
    }
}

/// Determine [`TestStatus`] by scanning the harness for this type's name.
fn determine_test_status(
    item: &Item,
    impl_status: &ImplStatus,
    harness: &ProofHarness,
) -> (TestStatus, TestStatus, String) {
    let name = &item.name;

    // Check for exact match in the non-empty harness
    if harness.non_empty_types.contains(name.as_str()) {
        let composition = check_composition_test(name, harness);
        return (TestStatus::Covered, composition, String::new());
    }

    // Check for a concrete instantiation (factory types tested as Wrapper<bool> etc.)
    if matches!(impl_status, ImplStatus::CompleteFactory) {
        let concrete = harness
            .non_empty_types
            .iter()
            .find(|t| t.starts_with(name.as_str()) && t.contains('<'));
        if let Some(instantiation) = concrete {
            let composition = check_composition_test(name, harness);
            return (
                TestStatus::CoveredConcrete {
                    instantiation: instantiation.clone(),
                },
                composition,
                format!("tested as {instantiation}"),
            );
        }
    }

    // Also check qualified paths: `url::Url` in harness matches source name `Url`
    let qualified_match = harness
        .non_empty_types
        .iter()
        .any(|t| t.ends_with(&format!("::{name}")) || t == name.as_str());
    if qualified_match {
        let composition = check_composition_test(name, harness);
        return (TestStatus::Covered, composition, String::new());
    }

    let composition = check_composition_test(name, harness);
    (TestStatus::Missing, composition, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::TraitPrereqs;
    use crate::inventory::ItemKind;

    fn inventory_with_items(items: Vec<Item>) -> Inventory {
        Inventory {
            crate_name: "reqwest".to_string(),
            crate_version: "0.12.28".to_string(),
            items,
        }
    }

    fn struct_item(path: &[&str]) -> Item {
        assert!(!path.is_empty(), "test helper requires a non-empty path");
        Item {
            path: path.iter().map(|segment| segment.to_string()).collect(),
            kind: ItemKind::Struct,
            name: path[path.len() - 1].to_string(),
            is_generic: false,
            lifetime_params: Vec::new(),
            type_params: Vec::new(),
        }
    }

    #[test]
    fn build_impl_coverage_uses_unique_root_reexport_alias_for_trait_prereqs() {
        let source = inventory_with_items(vec![struct_item(&[
            "reqwest",
            "async_impl",
            "response",
            "Response",
        ])]);
        let mut prereqs_map = HashMap::new();
        prereqs_map.insert(
            "reqwest::Response".to_string(),
            TraitPrereqs {
                elicitation_trait: true,
                elicit_introspect: true,
                elicit_spec: true,
                elicit_prompt_tree: true,
                to_code_literal: true,
                ..TraitPrereqs::default()
            },
        );

        let report = build_impl_coverage_report(
            &source,
            &ElicitCompleteSet::default(),
            &ProofHarness::default(),
            &prereqs_map,
        );

        assert_eq!(report.entries.len(), 1, "expected a single coverage entry");
        let entry = &report.entries[0];
        assert!(entry.prereqs.our_traits_complete());
    }

    #[test]
    fn build_impl_coverage_does_not_guess_from_non_unique_bare_names() {
        let source = inventory_with_items(vec![
            struct_item(&["reqwest", "alpha", "Shared"]),
            struct_item(&["reqwest", "beta", "Shared"]),
        ]);
        let mut prereqs_map = HashMap::new();
        prereqs_map.insert(
            "reqwest::Shared".to_string(),
            TraitPrereqs {
                to_code_literal: true,
                ..TraitPrereqs::default()
            },
        );

        let report = build_impl_coverage_report(
            &source,
            &ElicitCompleteSet::default(),
            &ProofHarness::default(),
            &prereqs_map,
        );

        assert!(
            report
                .entries
                .iter()
                .all(|entry| !entry.prereqs.to_code_literal)
        );
    }
}

/// Check whether a type appears in the composition harness.
fn check_composition_test(name: &str, harness: &ProofHarness) -> TestStatus {
    let found = harness
        .composition_pairs
        .iter()
        .any(|(outer, _inner)| outer.starts_with(name) || outer == name);
    if found {
        TestStatus::Covered
    } else {
        TestStatus::Missing
    }
}
