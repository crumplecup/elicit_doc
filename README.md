# elicit_doc

Coverage and drift analysis for the [elicitation](https://github.com/crumplecup/elicitation)
workspace.

---

## Background

The elicitation framework makes Rust types **agent-native** and **formally verifiable**.
The full capability is bundled in one trait: `ElicitComplete`.

A type that implements `ElicitComplete` can be:

- elicited interactively at runtime (structured prompt trees driven by JSON Schema)
- exposed as an MCP tool so AI agents can construct valid values directly
- used as a field in a Verified State Machine (VSM), enabling `#[derive(Kani)]`,
  `#[derive(Creusot)]`, and `#[derive(Verus)]` on the containing type

`ElicitComplete` is a supertrait of five traits:

| Trait | What it provides |
|---|---|
| `Elicitation` | Async interactive prompting |
| `ElicitIntrospect` | Runtime introspection and metadata |
| `ElicitSpec` | JSON Schema generation (`schemars::JsonSchema`) |
| `ElicitPromptTree` | Structured prompt tree construction |
| `ToCodeLiteral` | Round-trip to valid Rust source code |

Implementing all five requires that the type also implement `serde::Serialize`,
`serde::Deserialize`, and `schemars::JsonSchema`. For types we own, all of this can be
derived. For **foreign types** — types from third-party crates — the orphan rule prevents
us from adding those impls directly.

### Trenchcoats

A *trenchcoat* is an elicitation-owned wrapper type that provides the external traits
(`Serialize + Deserialize + JsonSchema`) for a foreign type, making the foreign type
transitively reachable from a fully verified VSM. The wrapper is detected structurally:
any `impl From<ForeignType> for ElicitationWrapper` where the wrapper is in the
`elicitation` namespace qualifies. This covers both `select_trenchcoat!`-generated
newtypes and hand-written wrappers.

### Shadow Crates

For large upstream crates (`bevy`, `egui`, `tokio`, etc.), elicitation maintains a
*shadow crate* — a parallel crate with one `ElicitComplete` wrapper per upstream public
type. Shadow coverage tracks how much of the upstream API surface is reachable from the
elicitation framework.

---

## What `elicit_doc` produces

Running `elicit_doc run` builds rustdoc JSON for every tracked crate and writes CSV
reports to `verif/coverage/`. All reports are committed to the repository as a
durable audit trail.

### Per-crate impl coverage (`<crate>.csv`, `internal.csv`)

One row per public type in the crate, recording:

| Column | Meaning |
|---|---|
| `type_path` | Canonical type path |
| `type_kind` | `Struct`, `Enum`, `TypeAlias`, … |
| `is_generic` | Whether the type has type parameters |
| `elicit_impl` | `Present` / `Missing` |
| `proof_test` | Whether a Kani proof harness exists |
| `has_serialize` / `has_deserialize` / `has_json_schema` | External trait status |
| `has_elicitation` … `has_to_code_literal` | Our 5 traits individually |
| `can_be_direct` | True if all external traits are present (no trenchcoat needed) |
| `external_blockers` | Which external traits are missing and why |

`internal.csv` covers elicitation's own types. Per-crate files (e.g. `chrono.csv`,
`url.csv`) cover third-party deps tracked as elicitation features.

### Impl gaps (`gaps-impl.csv`)

Consolidated, prioritised list of types without `ElicitComplete`. Each row includes a
short recommended action. Sorted highest-priority first:

| `gap_kind` | Meaning | Action |
|---|---|---|
| `MissingOurTraits` | One or more elicitation-owned support traits are missing | Add the missing support trait impls |
| `ReadyForElicitComplete` | All prerequisites are present, but `impl ElicitComplete` is still missing | Add the impl |
| `FeatureGatedExternal` | External serde/schemars traits may unlock behind more dep features | Enable features and re-check |

The CSV now separates the important axes explicitly:

- `our_traits_complete` tells you whether the type already has the five elicitation-owned traits
- `can_be_direct` tells you whether direct `ElicitComplete` is legal
- `blocked_by_orphan_rule` identifies types that are fully "everything but" and therefore should not be treated as direct `ElicitComplete` gaps

Fully-covered orphan-blocked types are intentionally omitted from `gaps-impl.csv`. They are covered enough for elicitation-core work, but still need a trenchcoat or shadow when formal verification requires a fully `ElicitComplete` type.

### Shadow coverage (`shadow-<crate>.csv`, `gaps-shadow.csv`)

Per-upstream-item status for each shadow crate:

| `status` | Meaning |
|---|---|
| `Covered` | Upstream public item has a matching shadow item |
| `Missing` | Upstream public item is not yet represented in the shadow crate |
| `Drifted` | Probable rename — similar name found in the shadow crate |
| `Extra` | Shadow crate has an item with no upstream match |

`gaps-shadow.csv` consolidates across all shadow pairs. `Extra` rows are further
classified as `InfrastructureExtra` (our own `*Params`, `*Plugin`, `*Ctx` types — expected)
or `PossiblyStale` (unexpected — may need removal or renaming). Matched type rows can
also produce `ShadowVerificationGap` when the shadow exists but is still not
`ElicitComplete`-ready.

### Trenchcoat inventory (`trenchcoats.csv`)

Structural inventory of all `impl From<ForeignType> for ElicitationWrapper` pairs.
Sorted: incomplete wrappers first.

| Column | Meaning |
|---|---|
| `foreign_crate` | Source crate of the wrapped type |
| `foreign_type` | Wrapped type (e.g. `url::SyntaxViolation`) |
| `wrapper_path` | Our wrapper (e.g. `elicitation::SyntaxViolationSelect`) |
| `wrapper_elicit_complete` | Whether the wrapper has `impl ElicitComplete` |
| `wrapper_missing_our_traits` | Which of our 5 traits the wrapper still lacks |
| `foreign_missing_our_traits` | Which of our 5 traits the foreign type still lacks |

This report is the primary tool for gap analysis on trenchcoats: an `ElicitComplete`
entry under a foreign type means that type is fully accessible from any VSM that uses it.

### Executive summary (`summary.md`)

High-level table summarising impl and shadow coverage across all tracked crates.
The impl summary distinguishes:

- support-trait completeness
- true direct `ElicitComplete` gaps
- orphan-blocked "everything but" coverage

The shadow summary uses the full public API surface as its denominator and also
surfaces matched shadow types that still fail verification readiness.

---

## Usage

```sh
# Full run — all impl and shadow reports
elicit_doc run

# Impl reports only
elicit_doc run --only impls

# Shadow reports only
elicit_doc run --only shadows

# Single third-party crate
elicit_doc run --crate-name url

# Point at a different elicitation workspace
elicit_doc --workspace /path/to/elicitation run
```

### Configuration

| Flag | Env var | Default |
|---|---|---|
| `--workspace` | `ELICITATION_WORKSPACE` | `../elicitation` relative to this repo |
| `--output-dir` | — | `verif/coverage/` inside this repo |

---

## Interpreting the data for next steps

1. **Start with `gaps-impl.csv`, sorted by `gap_kind`.**
   `MissingOurTraits` rows are the first correctness problem: they indicate public
   target types that elicitation-core still does not fully model with its own traits.

2. **`ReadyForElicitComplete` rows** are the next easiest wins. The type already
   satisfies all prerequisites, so you only need to add `impl ElicitComplete for Type {}`.

3. **Treat fully-covered orphan-blocked types as coverage, not false-positive gaps.**
   If a type has all five of our traits but still lacks external serde/schemars support,
   it will show up in the summary as externally blocked rather than as an impl gap.

4. **Check `trenchcoats.csv` for incomplete wrappers.**
   A wrapper that exists but has `wrapper_elicit_complete = false` is a trenchcoat that
   didn't go all the way. The `wrapper_missing_our_traits` column shows exactly what
   remains.

5. **`FeatureGatedExternal` rows** may resolve automatically once the dep's serde/schemars
   feature flags are enabled in Cargo.toml. The per-dep feature lists in `cli.rs`
   (`THIRD_PARTY_CRATES`) control what `elicit_doc` tries; update them if a dep gains
   new optional serde support.

6. **`gaps-shadow.csv` `Missing` rows** represent upstream public API surface that is
   not yet shadowed. `ShadowVerificationGap` rows are the follow-on queue for matched
   shadow types that still cannot participate in formal verification.

---

## Architecture

```
elicit_doc
├── collect.rs       — rustdoc JSON parsing, dep builds, trait prereq extraction
│                      collect_dep_inventory() — 3-step build: all-features →
│                        preferred-features → default-features
│                      collect_trenchcoat_pairs() — From<T> structural scan
├── impl_coverage.rs — per-type coverage entries and reports
├── gaps.rs          — ImplGapEntry / ShadowGapEntry classification
├── shadow.rs        — shadow crate drift analysis
├── trenchcoat.rs    — trenchcoat inventory and missing-trait analysis
├── report.rs        — CSV writers
├── summary.rs       — summary.md writer
├── inventory.rs     — type inventory primitives
└── cli.rs           — CLI parsing and orchestration
```

Reports are generated in two passes:

1. **Impl pass** — builds elicitation with all features, then builds each tracked
   third-party dep to extract trait presence. Produces per-crate CSVs, `gaps-impl.csv`,
   and `trenchcoats.csv`.

2. **Shadow pass** — builds each upstream crate and its shadow crate, then diffs
   the public type surfaces. Produces per-pair shadow CSVs and `gaps-shadow.csv`.
