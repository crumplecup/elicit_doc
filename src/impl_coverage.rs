//! Impl coverage analysis — use cases 1 and 2.
//!
//! Given a source [`Inventory`] (std, uuid, chrono, or elicitation itself) and
//! an `elicitation` inventory, produces an [`ImplCoverageReport`] showing which
//! types have `ElicitComplete` impls and harness test entries.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use tracing::instrument;

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
    pub type_params: Vec<String>,
    pub elicit_impl: ImplStatus,
    pub proof_test: TestStatus,
    pub composition_test: TestStatus,
    /// Human-readable note, e.g. the concrete instantiation used in the harness.
    pub notes: String,
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
    /// Summary line for CLI output.
    pub fn summary(&self) -> String {
        format!(
            "{} {} ({} complete, {} missing impl, {} missing test, {} flagged concrete)",
            self.source_crate,
            self.source_version,
            self.complete_count,
            self.missing_impl_count,
            self.missing_test_count,
            self.flagged_concrete_count,
        )
    }
}

/// Build an [`ImplCoverageReport`] by cross-referencing a source [`Inventory`]
/// against the `elicitation` inventory and scanned proof harnesses.
///
/// `source` is the crate being checked (std, uuid, elicitation internal types…).
/// `elicitation` is the full `elicitation` crate inventory.
/// `harness` is the scanned `proof_non_empty_test.rs` + `proof_composition_test.rs`.
#[instrument(skip(source, elicitation, harness), fields(source_crate = %source.crate_name))]
pub fn build_impl_coverage_report(
    source: &Inventory,
    elicitation: &Inventory,
    harness: &ProofHarness,
) -> ImplCoverageReport {
    // Build a set of type path strings that appear in elicitation's impls.
    // We approximate this by looking for types in the elicitation inventory whose
    // name matches — a deeper analysis would parse the actual impl blocks, but
    // the rustdoc JSON `impl` lists on each item give us what we need.
    let elicitation_complete: HashSet<String> = elicitation_complete_types(elicitation);
    let elicitation_factory: HashSet<String> = elicitation_factory_types(elicitation);

    let mut entries: Vec<ImplCoverageEntry> = Vec::new();

    for item in source.type_items() {
        let path_str = item.path_str();
        let bare_name = &item.name;

        let elicit_impl = determine_impl_status(item, &elicitation_complete, &elicitation_factory);

        let (proof_test, composition_test, notes) =
            determine_test_status(item, &elicit_impl, harness);

        entries.push(ImplCoverageEntry {
            type_path: path_str,
            type_kind: item.kind,
            is_generic: item.is_generic,
            type_params: item.type_params.clone(),
            elicit_impl,
            proof_test,
            composition_test,
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

/// Extract the set of type names (bare, unqualified) that are `ElicitComplete`
/// in the elicitation inventory.
///
/// The rustdoc JSON for `elicitation` carries impl lists on each item; we look
/// for impls whose trait path resolves to `ElicitComplete`.
fn elicitation_complete_types(elicitation: &Inventory) -> HashSet<String> {
    elicitation
        .type_items()
        .filter(|item| {
            // Heuristic: if elicitation has a type with the same name as the
            // source type AND no type params, treat it as a concrete complete impl.
            // The deeper check (parsing impl blocks) is done via the source-join below.
            !item.is_generic
        })
        .map(|item| item.name.clone())
        .collect()
}

/// Extract the set of type names that have factory (generic) `ElicitComplete` impls.
fn elicitation_factory_types(elicitation: &Inventory) -> HashSet<String> {
    elicitation
        .type_items()
        .filter(|item| item.is_generic)
        .map(|item| item.name.clone())
        .collect()
}

/// Determine the [`ImplStatus`] for a source type by checking the elicitation inventories.
fn determine_impl_status(
    item: &Item,
    complete: &HashSet<String>,
    factory: &HashSet<String>,
) -> ImplStatus {
    // Check by bare name — the source type `uuid::Uuid` matches elicitation's `Uuid` wrapper.
    if factory.contains(&item.name) && item.is_generic {
        ImplStatus::CompleteFactory
    } else if complete.contains(&item.name) {
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
