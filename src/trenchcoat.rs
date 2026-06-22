//! Trenchcoat analysis — tracks wrapper types that give foreign types ElicitComplete coverage.
//!
//! A "trenchcoat" is an elicitation-owned type that wraps a foreign type to provide
//! `Serialize + Deserialize + JsonSchema` (which the orphan rule prevents implementing
//! directly on the foreign type), thereby allowing both the wrapper AND, transitively,
//! the foreign type to reach full `ElicitComplete` coverage.
//!
//! Detection is structural: we look for `impl From<ForeignType> for OurWrapper` in the
//! elicitation rustdoc JSON where `OurWrapper` is in the `elicitation` namespace and
//! `ForeignType` is not.  This captures both `select_trenchcoat!`-generated wrappers
//! and hand-written owned types (e.g. `BevyColor`, `EguiColor32`).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::collect::{ElicitCompleteSet, TraitPrereqs};

/// The 5 elicitation-owned traits that we can always implement for foreign types.
type OurTraitChecker = fn(&TraitPrereqs) -> bool;
const OUR_TRAITS: &[(&str, OurTraitChecker)] = &[
    ("Elicitation", |p| p.elicitation_trait),
    ("ElicitIntrospect", |p| p.elicit_introspect),
    ("ElicitSpec", |p| p.elicit_spec),
    ("ElicitPromptTree", |p| p.elicit_prompt_tree),
    ("ToCodeLiteral", |p| p.to_code_literal),
];

/// Return semicolon-separated names of whichever OUR_TRAITS are missing from `prereqs`.
///
/// If `prereqs` is `None` (type not found in any inventory), all 5 are listed as missing.
fn missing_our_traits(prereqs: Option<&TraitPrereqs>) -> String {
    match prereqs {
        None => OUR_TRAITS
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(";"),
        Some(p) => OUR_TRAITS
            .iter()
            .filter(|(_, f)| !f(p))
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(";"),
    }
}

/// One row in the trenchcoat report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrenchcoatEntry {
    /// Source crate of the foreign type (first path segment, e.g. `"url"`).
    pub foreign_crate: String,
    /// The foreign type being wrapped (e.g. `url::SyntaxViolation`).
    pub foreign_type: String,
    /// Our elicitation wrapper type (e.g. `elicitation::SyntaxViolationSelect`).
    pub wrapper_path: String,
    /// Whether the wrapper has `impl ElicitComplete`.
    pub wrapper_elicit_complete: bool,
    /// Of our 5 own traits, which does the wrapper still lack? (semicolon-separated, empty if none)
    pub wrapper_missing_our_traits: String,
    /// Of our 5 own traits, which does the foreign type still lack? (semicolon-separated, empty if none)
    pub foreign_missing_our_traits: String,
}

/// Build the trenchcoat report from structural `From<ForeignType>` pairs.
///
/// - `pairs` — `(foreign_type_path, wrapper_path)` from [`crate::collect::collect_trenchcoat_pairs`].
/// - `complete_paths` — the `ElicitComplete` impl set from elicitation JSON.
/// - `wrapper_prereqs` — trait prereqs for elicitation-namespace types (from elicitation JSON).
/// - `foreign_prereqs` — combined trait prereqs for all foreign types (merged from dep reports).
#[instrument(skip(pairs, complete_paths, wrapper_prereqs, foreign_prereqs))]
pub fn build_trenchcoat_report(
    pairs: &[(String, String)],
    complete_paths: &ElicitCompleteSet,
    wrapper_prereqs: &HashMap<String, TraitPrereqs>,
    foreign_prereqs: &HashMap<String, TraitPrereqs>,
) -> Vec<TrenchcoatEntry> {
    let mut entries: Vec<TrenchcoatEntry> = pairs
        .iter()
        .map(|(foreign, wrapper)| {
            let foreign_crate = foreign.split("::").next().unwrap_or(foreign).to_string();

            let wrapper_complete = complete_paths.concrete.contains(wrapper.as_str());
            let wrapper_missing = missing_our_traits(wrapper_prereqs.get(wrapper.as_str()));
            let foreign_missing = missing_our_traits(foreign_prereqs.get(foreign.as_str()));

            TrenchcoatEntry {
                foreign_crate,
                foreign_type: foreign.clone(),
                wrapper_path: wrapper.clone(),
                wrapper_elicit_complete: wrapper_complete,
                wrapper_missing_our_traits: wrapper_missing,
                foreign_missing_our_traits: foreign_missing,
            }
        })
        .collect();

    // Sort: incomplete wrappers first (false < true), then by foreign crate, then type.
    entries.sort_by(|a, b| {
        a.wrapper_elicit_complete
            .cmp(&b.wrapper_elicit_complete)
            .then(a.foreign_crate.cmp(&b.foreign_crate))
            .then(a.foreign_type.cmp(&b.foreign_type))
    });

    tracing::info!(
        total = entries.len(),
        complete = entries.iter().filter(|e| e.wrapper_elicit_complete).count(),
        incomplete = entries
            .iter()
            .filter(|e| !e.wrapper_elicit_complete)
            .count(),
        "built trenchcoat report"
    );

    entries
}
