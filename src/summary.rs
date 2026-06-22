//! Generate the executive summary markdown from all collected reports.

use std::path::Path;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::gaps::{ImplGapEntry, ImplGapKind, ShadowGapEntry, ShadowGapKind};
use crate::impl_coverage::ImplCoverageReport;
use crate::shadow::ShadowReport;

/// Write `summary.md` to `output_path` from all collected reports and gaps.
#[instrument(skip_all, fields(output = %output_path.display()))]
pub fn write_summary_md(
    impl_reports: &[(String, ImplCoverageReport)],
    impl_gaps: &[ImplGapEntry],
    shadow_reports: &[(String, String, ShadowReport)],
    shadow_gaps: &[ShadowGapEntry],
    output_path: &Path,
) -> ElicitDocResult<()> {
    let mut out = String::with_capacity(4096);

    out.push_str("# elicitation Coverage Summary\n\n");
    out.push_str(&format!("_Generated: {}_\n\n", today_string()));
    out.push_str("---\n\n");

    if !impl_reports.is_empty() {
        write_impl_section(&mut out, impl_reports, impl_gaps);
    }

    if !shadow_reports.is_empty() {
        write_shadow_section(&mut out, shadow_reports, shadow_gaps);
    }

    std::fs::write(output_path, out)
        .map_err(|e| ElicitDocError::io(format!("writing {}: {}", output_path.display(), e)))?;

    tracing::info!("wrote summary");
    Ok(())
}

fn write_impl_section(
    out: &mut String,
    reports: &[(String, ImplCoverageReport)],
    gaps: &[ImplGapEntry],
) {
    out.push_str("## Impl Coverage\n\n");
    out.push_str("| Crate | Version | Types | OurTraitsDone | MissingOurTraits | ElicitComplete | ElicitCompleteGap | ExternallyBlocked | Coverage |\n");
    out.push_str("|-------|---------|------:|--------------:|-----------------:|---------------:|------------------:|------------------:|---------:|\n");

    let mut total_types = 0usize;
    let mut total_our_traits_done = 0usize;
    let mut total_missing_our_traits = 0usize;
    let mut total_complete = 0usize;
    let mut total_elicit_complete_gap = 0usize;
    let mut total_externally_blocked = 0usize;

    for (_, r) in reports {
        let types = r.entries.len();
        let our_traits_done = r
            .entries
            .iter()
            .filter(|e| e.effective_our_traits_complete())
            .count();
        let missing_our_traits = types.saturating_sub(our_traits_done);
        let elicit_complete_gap = r
            .entries
            .iter()
            .filter(|e| {
                matches!(e.elicit_impl, crate::impl_coverage::ImplStatus::Missing)
                    && e.effective_our_traits_complete()
                    && e.can_be_direct()
            })
            .count();
        let externally_blocked = r
            .entries
            .iter()
            .filter(|e| {
                matches!(e.elicit_impl, crate::impl_coverage::ImplStatus::Missing)
                    && e.effective_our_traits_complete()
                    && !e.can_be_direct()
                    && !e.lifetime_blocks_elicitation()
            })
            .count();
        let pct = if types == 0 {
            0.0
        } else {
            r.complete_count as f32 / types as f32 * 100.0
        };
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} | {} | {:.1}% |\n",
            r.source_crate,
            r.source_version,
            types,
            our_traits_done,
            missing_our_traits,
            r.complete_count,
            elicit_complete_gap,
            externally_blocked,
            pct
        ));
        total_types += types;
        total_our_traits_done += our_traits_done;
        total_missing_our_traits += missing_our_traits;
        total_complete += r.complete_count;
        total_elicit_complete_gap += elicit_complete_gap;
        total_externally_blocked += externally_blocked;
    }

    let total_pct = if total_types == 0 {
        0.0
    } else {
        total_complete as f32 / total_types as f32 * 100.0
    };
    out.push_str(&format!(
        "| **Total** | | **{}** | **{}** | **{}** | **{}** | **{}** | **{}** | **{:.1}%** |\n",
        total_types,
        total_our_traits_done,
        total_missing_our_traits,
        total_complete,
        total_elicit_complete_gap,
        total_externally_blocked,
        total_pct
    ));
    out.push_str("\n`OurTraitsDone` counts all elicitation-owned traits that are actually implementable for the type. Lifetime-bound types such as `Pixels<'a, R>` are not expected to implement `Elicitation` or `ElicitIntrospect` because `Elicitation` requires `'static`.\n\n");
    out.push_str("`ExternallyBlocked` means the implementable elicitation-owned traits are present, but direct `ElicitComplete` is blocked by missing `Serialize`, `Deserialize`, or `JsonSchema` on the target type.\n\n");

    if !gaps.is_empty() {
        let missing_our = gaps
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::MissingOurTraits)
            .count();
        let ready = gaps
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::ReadyForElicitComplete)
            .count();
        let gated = gaps
            .iter()
            .filter(|e| e.gap_kind == ImplGapKind::FeatureGatedExternal)
            .count();

        out.push_str("### Impl Gaps\n\n");
        out.push_str("| Kind | Count | Notes |\n");
        out.push_str("|------|------:|-------|\n");
        out.push_str(&format!(
            "| MissingOurTraits | {} | Missing one or more elicitation-owned support traits |\n",
            missing_our
        ));
        out.push_str(&format!(
            "| ReadyForElicitComplete | {} | All prerequisites present; only `impl ElicitComplete` is missing |\n",
            ready
        ));
        out.push_str(&format!(
            "| FeatureGatedExternal | {} | Missing external serde/schemars traits may be unlockable with more features |\n",
            gated
        ));
        out.push_str(&format!("| **Total** | **{}** | |\n", gaps.len()));
        out.push('\n');
    }

    out.push_str("---\n\n");
}

fn write_shadow_section(
    out: &mut String,
    reports: &[(String, String, ShadowReport)],
    gaps: &[ShadowGapEntry],
) {
    out.push_str("## Shadow Coverage\n\n");
    out.push_str("| Upstream | Version | Shadow Crate | Covered | Drifted | Total | VerificationGaps | Coverage |\n");
    out.push_str("|----------|---------|-------------|--------:|--------:|------:|-----------------:|---------:|\n");

    for (_, _, r) in reports {
        let total = r.covered_count + r.drifted_count + r.missing_count;
        out.push_str(&format!(
            "| `{}` | {} | `{}` | {} | {} | {} | {} | {:.1}% |\n",
            r.target_crate,
            r.target_version,
            r.shadow_crate,
            r.covered_count,
            r.drifted_count,
            total,
            r.verification_gap_count,
            r.coverage_pct,
        ));
    }
    out.push('\n');

    if !gaps.is_empty() {
        let missing = gaps
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::Missing)
            .count();
        let drifted = gaps
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::Drifted)
            .count();
        let stale = gaps
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::PossiblyStale)
            .count();
        let infra = gaps
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::InfrastructureExtra)
            .count();
        let verification = gaps
            .iter()
            .filter(|e| e.gap_kind == ShadowGapKind::ShadowVerificationGap)
            .count();

        out.push_str("### Shadow Gaps\n\n");
        out.push_str("| Kind | Count | Notes |\n");
        out.push_str("|------|------:|-------|\n");
        out.push_str(&format!(
            "| Missing | {} | Upstream public item not yet shadowed |\n",
            missing
        ));
        out.push_str(&format!(
            "| Drifted | {} | Probable rename or naming drift in the shadow crate |\n",
            drifted
        ));
        out.push_str(&format!(
            "| PossiblyStale | {} | Shadow item with no matching upstream — needs audit |\n",
            stale
        ));
        out.push_str(&format!(
            "| InfrastructureExtra | {} | Shadow-only infrastructure item — expected |\n",
            infra
        ));
        out.push_str(&format!(
            "| ShadowVerificationGap | {} | Matched shadow type exists but is not yet `ElicitComplete`-ready |\n",
            verification
        ));
        out.push_str(&format!("| **Total** | **{}** | |\n", gaps.len()));
        out.push('\n');
    }
}

/// Returns today's date as `YYYY-MM-DD` without any external dependency.
fn today_string() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since 1970-01-01
    let z = (secs / 86400) as u32 + 719468;
    let era = z / 146097;
    let doe = z % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}
