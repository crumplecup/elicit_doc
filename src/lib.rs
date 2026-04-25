//! `elicit_doc` — coverage and drift analysis for the elicitation ecosystem.
//!
//! Produces CSV reports in `verif/coverage/` tracking:
//!
//! - **Impl coverage**: which std / third-party types have `ElicitComplete` impls
//!   in `elicitation`, and which are covered by the proof harness tests.
//! - **Internal coverage**: elicitation's own wrapper/contract types.
//! - **Shadow coverage**: how much of an upstream crate's API surface is covered
//!   by the corresponding `elicit_*` shadow crate.

mod collect;
mod error;
mod gaps;
mod impl_coverage;
mod inventory;
mod report;
mod shadow;
mod summary;

#[cfg(feature = "cli")]
pub mod cli;

pub use collect::{collect_inventory, collect_proof_harness, collect_trait_prereqs, TraitPrereqs};
pub use error::{ElicitDocError, ElicitDocErrorKind, ElicitDocResult};
pub use gaps::{
    ImplGapEntry, ImplGapKind, ShadowGapEntry, ShadowGapKind,
    build_impl_gaps, build_shadow_gaps,
};
pub use impl_coverage::{
    ImplCoverageEntry, ImplCoverageReport, ImplStatus, ProofHarness, TestStatus,
    build_impl_coverage_report,
};
pub use inventory::{Inventory, Item, ItemKind};
pub use report::{write_impl_coverage_csv, write_impl_gaps_csv, write_shadow_csv, write_shadow_gaps_csv};
pub use shadow::{DriftPair, ShadowReport, build_shadow_report};
pub use summary::write_summary_md;
