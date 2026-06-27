use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use elicit_doc::{
    ImplCoverageEntry, ImplCoverageReport, ImplStatus, ItemKind, ShadowReport, TestStatus,
    TraitPrereqs, WrapperCoverage, WrapperCoverageMap, write_summary_md,
};

fn temp_summary_path(test_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("elicit_doc-{test_name}-{nanos}.md"))
}

#[test]
fn summary_sorts_impl_and_shadow_sections_alphabetically() {
    let summary_path = temp_summary_path("summary-order");
    let impl_reports = vec![
        (
            "url".to_string(),
            ImplCoverageReport {
                source_crate: "url".to_string(),
                source_version: "1.0.0".to_string(),
                entries: Vec::new(),
                complete_count: 0,
                missing_impl_count: 0,
                missing_test_count: 0,
                flagged_concrete_count: 0,
            },
        ),
        (
            "chrono".to_string(),
            ImplCoverageReport {
                source_crate: "chrono".to_string(),
                source_version: "1.0.0".to_string(),
                entries: Vec::new(),
                complete_count: 0,
                missing_impl_count: 0,
                missing_test_count: 0,
                flagged_concrete_count: 0,
            },
        ),
        (
            "reqwest".to_string(),
            ImplCoverageReport {
                source_crate: "reqwest".to_string(),
                source_version: "1.0.0".to_string(),
                entries: Vec::new(),
                complete_count: 0,
                missing_impl_count: 0,
                missing_test_count: 0,
                flagged_concrete_count: 0,
            },
        ),
    ];
    let shadow_reports = vec![
        (
            "url".to_string(),
            "elicit_url".to_string(),
            ShadowReport {
                target_crate: "url".to_string(),
                target_version: "1.0.0".to_string(),
                shadow_crate: "elicit_url".to_string(),
                shadow_version: "1.0.0".to_string(),
                rows: Vec::new(),
                covered_count: 0,
                missing_count: 0,
                extra_count: 0,
                drifted_count: 0,
                coverage_pct: 0.0,
                verification_gap_count: 0,
            },
        ),
        (
            "chrono".to_string(),
            "elicit_chrono".to_string(),
            ShadowReport {
                target_crate: "chrono".to_string(),
                target_version: "1.0.0".to_string(),
                shadow_crate: "elicit_chrono".to_string(),
                shadow_version: "1.0.0".to_string(),
                rows: Vec::new(),
                covered_count: 0,
                missing_count: 0,
                extra_count: 0,
                drifted_count: 0,
                coverage_pct: 0.0,
                verification_gap_count: 0,
            },
        ),
        (
            "reqwest".to_string(),
            "elicit_reqwest".to_string(),
            ShadowReport {
                target_crate: "reqwest".to_string(),
                target_version: "1.0.0".to_string(),
                shadow_crate: "elicit_reqwest".to_string(),
                shadow_version: "1.0.0".to_string(),
                rows: Vec::new(),
                covered_count: 0,
                missing_count: 0,
                extra_count: 0,
                drifted_count: 0,
                coverage_pct: 0.0,
                verification_gap_count: 0,
            },
        ),
    ];

    let write_result = write_summary_md(
        &impl_reports,
        &[],
        None,
        &shadow_reports,
        &[],
        &[],
        &summary_path,
    );
    assert!(
        write_result.is_ok(),
        "summary should write: {write_result:?}"
    );

    let summary_result = fs::read_to_string(&summary_path);
    assert!(
        summary_result.is_ok(),
        "summary should be readable: {summary_result:?}"
    );
    let summary = summary_result.unwrap_or_default();
    let impl_chrono = summary.find("| `chrono` | 1.0.0 |");
    let impl_reqwest = summary.find("| `reqwest` | 1.0.0 |");
    let impl_url = summary.find("| `url` | 1.0.0 |");
    assert!(impl_chrono.is_some(), "chrono impl row");
    assert!(impl_reqwest.is_some(), "reqwest impl row");
    assert!(impl_url.is_some(), "url impl row");
    let impl_chrono = impl_chrono.unwrap_or_default();
    let impl_reqwest = impl_reqwest.unwrap_or_default();
    let impl_url = impl_url.unwrap_or_default();
    assert!(impl_chrono < impl_reqwest);
    assert!(impl_reqwest < impl_url);

    let shadow_chrono = summary.find("| `chrono` | 1.0.0 | `elicit_chrono` |");
    let shadow_reqwest = summary.find("| `reqwest` | 1.0.0 | `elicit_reqwest` |");
    let shadow_url = summary.find("| `url` | 1.0.0 | `elicit_url` |");
    assert!(shadow_chrono.is_some(), "chrono shadow row");
    assert!(shadow_reqwest.is_some(), "reqwest shadow row");
    assert!(shadow_url.is_some(), "url shadow row");
    let shadow_chrono = shadow_chrono.unwrap_or_default();
    let shadow_reqwest = shadow_reqwest.unwrap_or_default();
    let shadow_url = shadow_url.unwrap_or_default();
    assert!(shadow_chrono < shadow_reqwest);
    assert!(shadow_reqwest < shadow_url);

    let _ = fs::remove_file(summary_path);
}

#[test]
fn summary_uses_effective_coverage_not_only_direct_elicit_complete() {
    let summary_path = temp_summary_path("summary-effective-coverage");
    let impl_reports = vec![(
        "example".to_string(),
        ImplCoverageReport {
            source_crate: "example".to_string(),
            source_version: "1.0.0".to_string(),
            entries: vec![
                ImplCoverageEntry {
                    type_path: "example::Direct".to_string(),
                    type_kind: ItemKind::Struct,
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                    elicit_impl: ImplStatus::Complete,
                    proof_test: TestStatus::Covered,
                    composition_test: TestStatus::Covered,
                    prereqs: TraitPrereqs {
                        serialize: true,
                        deserialize: true,
                        json_schema: true,
                        elicitation_trait: true,
                        elicit_introspect: true,
                        elicit_spec: true,
                        elicit_prompt_tree: true,
                        to_code_literal: true,
                    },
                    notes: String::new(),
                },
                ImplCoverageEntry {
                    type_path: "example::Blocked".to_string(),
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
                },
                ImplCoverageEntry {
                    type_path: "example::Borrowed".to_string(),
                    type_kind: ItemKind::Struct,
                    is_generic: true,
                    lifetime_params: vec!["'a".to_string()],
                    type_params: Vec::new(),
                    elicit_impl: ImplStatus::Missing,
                    proof_test: TestStatus::Missing,
                    composition_test: TestStatus::Missing,
                    prereqs: TraitPrereqs {
                        serialize: true,
                        deserialize: true,
                        json_schema: true,
                        elicitation_trait: false,
                        elicit_introspect: false,
                        elicit_spec: true,
                        elicit_prompt_tree: true,
                        to_code_literal: true,
                    },
                    notes: String::new(),
                },
                ImplCoverageEntry {
                    type_path: "example::Gap".to_string(),
                    type_kind: ItemKind::Struct,
                    is_generic: false,
                    lifetime_params: Vec::new(),
                    type_params: Vec::new(),
                    elicit_impl: ImplStatus::Missing,
                    proof_test: TestStatus::Missing,
                    composition_test: TestStatus::Missing,
                    prereqs: TraitPrereqs {
                        serialize: true,
                        deserialize: true,
                        json_schema: true,
                        elicitation_trait: true,
                        elicit_introspect: true,
                        elicit_spec: false,
                        elicit_prompt_tree: true,
                        to_code_literal: true,
                    },
                    notes: String::new(),
                },
            ],
            complete_count: 1,
            missing_impl_count: 3,
            missing_test_count: 3,
            flagged_concrete_count: 0,
        },
    )];

    let write_result = write_summary_md(&impl_reports, &[], None, &[], &[], &[], &summary_path);
    assert!(
        write_result.is_ok(),
        "summary should write: {write_result:?}"
    );

    let summary_result = fs::read_to_string(&summary_path);
    assert!(
        summary_result.is_ok(),
        "summary should be readable: {summary_result:?}"
    );
    let summary = summary_result.unwrap_or_default();
    assert!(
        summary.contains("| `example` | 1.0.0 | 4 | 3 | 1 | 1 | 0 | 1 | 75.0% |"),
        "summary should count effective coverage, not only direct ElicitComplete:\n{summary}"
    );
    assert!(
        summary.contains(
            "| **Total** | | **4** | **3** | **1** | **1** | **0** | **1** | **75.0%** |"
        ),
        "total row should use effective coverage:\n{summary}"
    );

    let _ = fs::remove_file(summary_path);
}

#[test]
fn summary_counts_wrapper_coverage_as_done() {
    let summary_path = temp_summary_path("summary-wrapper-coverage");
    let impl_reports = vec![(
        "reqwest".to_string(),
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
                prereqs: TraitPrereqs::default(),
                notes: String::new(),
            }],
            complete_count: 0,
            missing_impl_count: 1,
            missing_test_count: 1,
            flagged_concrete_count: 0,
        },
    )];
    let mut wrapper_coverage = WrapperCoverageMap::new();
    wrapper_coverage.insert(
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

    let write_result = write_summary_md(
        &impl_reports,
        &[],
        Some(&wrapper_coverage),
        &[],
        &[],
        &[],
        &summary_path,
    );
    assert!(
        write_result.is_ok(),
        "summary should write: {write_result:?}"
    );

    let summary_result = fs::read_to_string(&summary_path);
    assert!(
        summary_result.is_ok(),
        "summary should be readable: {summary_result:?}"
    );
    let summary = summary_result.unwrap_or_default();
    assert!(
        summary.contains("| `reqwest` | 1.0.0 | 1 | 1 | 0 | 0 | 0 | 0 | 100.0% |"),
        "summary should treat wrapper coverage as complete:\n{summary}"
    );

    let _ = fs::remove_file(summary_path);
}
