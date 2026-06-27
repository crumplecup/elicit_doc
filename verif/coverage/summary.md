# elicitation Coverage Summary

_Generated: 2026-06-26_

---

## Impl Coverage

| Crate | Version | Types | OurTraitsDone | MissingOurTraits | ElicitComplete | ElicitCompleteGap | ExternallyBlocked | Coverage |
|-------|---------|------:|--------------:|-----------------:|---------------:|------------------:|------------------:|---------:|
| `accesskit` | 0.24.0 | 30 | 27 | 3 | 27 | 0 | 0 | 90.0% |
| `chrono` | 0.4.41 | 20 | 0 | 20 | 0 | 0 | 0 | 0.0% |
| `csv` | 1.4.0 | 5 | 3 | 2 | 0 | 0 | 0 | 60.0% |
| `elicitation` | 0.11.1 | 295 | 142 | 153 | 7 | 0 | 0 | 48.1% |
| `geo` | 0.33.1 | 47 | 0 | 47 | 0 | 0 | 0 | 0.0% |
| `geojson` | 1.0.0 | 12 | 3 | 9 | 0 | 0 | 2 | 25.0% |
| `georaster` | 0.2.0 | 7 | 5 | 2 | 0 | 0 | 4 | 71.4% |
| `jiff` | 0.2.28 | 16 | 0 | 16 | 0 | 0 | 0 | 0.0% |
| `proj` | 0.31.0 | 37 | 0 | 37 | 0 | 0 | 0 | 0.0% |
| `redb` | 4.1.0 | 1 | 0 | 1 | 0 | 0 | 0 | 0.0% |
| `regex` | 1.12.3 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0% |
| `reqwest` | 0.13.4 | 22 | 13 | 9 | 1 | 0 | 9 | 59.1% |
| `rstar` | 0.13.0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.0% |
| `serde_json` | 1.0.150 | 21 | 1 | 20 | 1 | 0 | 0 | 4.8% |
| `time` | 0.3.47 | 35 | 0 | 35 | 0 | 0 | 0 | 0.0% |
| `toml` | 1.1.2+spec-1.1.0 | 12 | 1 | 11 | 0 | 0 | 0 | 8.3% |
| `url` | 2.5.8 | 6 | 1 | 5 | 1 | 0 | 0 | 16.7% |
| `uuid` | 1.23.2 | 10 | 1 | 9 | 1 | 0 | 0 | 10.0% |
| `wkb` | 0.9.2 | 4 | 0 | 4 | 0 | 0 | 0 | 0.0% |
| `wkt` | 0.14.0 | 5 | 3 | 2 | 0 | 0 | 0 | 60.0% |
| **Total** | | **585** | **200** | **385** | **38** | **0** | **15** | **34.2%** |

`OurTraitsDone` counts effective trait coverage. A trait counts when it is satisfied either directly on the target type or indirectly via a wrapper that deductively covers that target. Lifetime-bound types such as `Pixels<'a, R>` are still not expected to implement `Elicitation` or `ElicitIntrospect` directly because `Elicitation` requires `'static`.

`Coverage` uses that same effective-coverage rule. A type counts as covered when every elicitation-owned trait that should exist for that target is present, either directly or through wrapper coverage, even if direct `ElicitComplete` is blocked by lifetimes or the orphan rule.

`ExternallyBlocked` counts true orphan-rule blockers only: the implementable elicitation-owned traits are present, but direct `ElicitComplete` is blocked by missing `Serialize`, `Deserialize`, or `JsonSchema` on the target type. Lifetime-bound rows still count toward `Coverage` when every implementable elicitation-owned trait is present, but they are not counted as external blockers.

### Impl Gaps

| Kind | Count | Notes |
|------|------:|-------|
| MissingOurTraits | 385 | Missing one or more elicitation-owned support traits |
| ReadyForElicitComplete | 0 | All prerequisites present; only `impl ElicitComplete` is missing |
| FeatureGatedExternal | 0 | Missing external serde/schemars traits may be unlockable with more features |
| **Total** | **385** | |

---

## Shadow Coverage

| Upstream | Version | Shadow Crate | Covered | Drifted | Total | VerificationGaps | Coverage |
|----------|---------|-------------|--------:|--------:|------:|-----------------:|---------:|
| `accesskit` | 0.24.0 | `elicit_accesskit` | 1 | 0 | 33 | 1 | 3.0% |
| `axum` | 0.8.9 | `elicit_axum` | 0 | 0 | 123 | 0 | 0.0% |
| `bevy` | 0.19.0 | `elicit_bevy` | 10 | 1 | 137 | 0 | 8.0% |
| `chrono` | 0.4.41 | `elicit_chrono` | 0 | 0 | 26 | 0 | 0.0% |
| `csv` | 1.4.0 | `elicit_csv` | 2 | 2 | 10 | 0 | 40.0% |
| `egui` | 0.34.3 | `elicit_egui` | 4 | 1 | 244 | 5 | 2.0% |
| `geo` | 0.33.1 | `elicit_geo` | 0 | 0 | 149 | 0 | 0.0% |
| `geojson` | 1.0.0 | `elicit_geojson` | 4 | 0 | 35 | 4 | 11.4% |
| `georaster` | 0.2.0 | `elicit_georaster` | 0 | 0 | 7 | 0 | 0.0% |
| `jiff` | 0.2.28 | `elicit_jiff` | 0 | 0 | 60 | 0 | 0.0% |
| `proj` | 0.31.0 | `elicit_proj` | 0 | 0 | 76 | 0 | 0.0% |
| `ratatui` | 0.30.0 | `elicit_ratatui` | 0 | 0 | 2 | 0 | 0.0% |
| `redb` | 4.1.0 | `elicit_redb` | 0 | 0 | 1 | 0 | 0.0% |
| `regex` | 1.12.3 | `elicit_regex` | 0 | 0 | 1 | 0 | 0.0% |
| `reqwest` | 0.13.4 | `elicit_reqwest` | 1 | 0 | 33 | 1 | 3.0% |
| `rstar` | 0.13.0 | `elicit_rstar` | 0 | 0 | 0 | 0 | 100.0% |
| `serde` | 1.0.228 | `elicit_serde` | 0 | 0 | 1 | 0 | 0.0% |
| `serde_json` | 1.0.150 | `elicit_serde_json` | 1 | 0 | 37 | 1 | 2.7% |
| `sqlx` | 0.8.6 | `elicit_sqlx` | 0 | 0 | 23 | 0 | 0.0% |
| `time` | 0.3.47 | `elicit_time` | 0 | 0 | 78 | 0 | 0.0% |
| `tokio` | 1.52.3 | `elicit_tokio` | 0 | 0 | 52 | 0 | 0.0% |
| `toml` | 1.1.2+spec-1.1.0 | `elicit_toml` | 0 | 0 | 20 | 0 | 0.0% |
| `uom` | 0.38.0 | `elicit_uom` | 0 | 0 | 3888 | 0 | 0.0% |
| `url` | 2.5.8 | `elicit_url` | 0 | 0 | 8 | 0 | 0.0% |
| `uuid` | 1.23.2 | `elicit_uuid` | 0 | 0 | 15 | 0 | 0.0% |
| `wgpu` | 29.0.3 | `elicit_wgpu` | 0 | 0 | 119 | 0 | 0.0% |
| `winit` | 0.30.13 | `elicit_winit` | 1 | 0 | 91 | 1 | 1.1% |
| `wkb` | 0.9.2 | `elicit_wkb` | 0 | 0 | 16 | 0 | 0.0% |
| `wkt` | 0.14.0 | `elicit_wkt` | 0 | 1 | 27 | 0 | 3.7% |

### Shadow Gaps

| Kind | Count | Notes |
|------|------:|-------|
| Missing | 5283 | Upstream public item not yet shadowed |
| Drifted | 5 | Probable rename or naming drift in the shadow crate |
| PossiblyStale | 801 | Shadow item with no matching upstream — needs audit |
| InfrastructureExtra | 458 | Shadow-only infrastructure item — expected |
| ShadowVerificationGap | 13 | Matched shadow type exists but is not yet `ElicitComplete`-ready |
| **Total** | **6560** | |

### Skipped Shadow Crates

| Upstream | Shadow Crate | Error |
|----------|--------------|-------|
| `clap` | `elicit_clap` | elicit_doc: cargo invocation failed: cargo rustdoc for dep clap exited with exit status: 101 at src/collect.rs:515 |
| `geo-types` | `elicit_geo_types` | elicit_doc: cargo invocation failed: resolved dependency edge `geo-types` for `elicit_geo_types` not found while locating `geo-types` at src/collect.rs:1114 |
| `leptos` | `elicit_leptos` | elicit_doc: cargo invocation failed: dependency 'leptos' not found in `elicit_leptos` package metadata at src/collect.rs:1103 |
| `polars` | `elicit_polars` | elicit_doc: cargo invocation failed: cargo rustdoc for dep polars exited with exit status: 101 at src/collect.rs:515 |
| `surrealdb-types` | `elicit_surrealdb` | elicit_doc: cargo invocation failed: workspace package `elicit_surrealdb` not found in cargo metadata at src/collect.rs:1080 |
| `tower` | `elicit_tower` | elicit_doc: cargo invocation failed: cargo rustdoc for dep tower exited with exit status: 101 at src/collect.rs:515 |

