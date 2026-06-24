# elicitation Coverage Summary

_Generated: 2026-06-23_

---

## Impl Coverage

| Crate | Version | Types | OurTraitsDone | MissingOurTraits | ElicitComplete | ElicitCompleteGap | ExternallyBlocked | Coverage |
|-------|---------|------:|--------------:|-----------------:|---------------:|------------------:|------------------:|---------:|
| `chrono` | 0.4.41 | 43 | 6 | 37 | 6 | 0 | 1 | 14.0% |
| `time` | 0.3.47 | 73 | 2 | 71 | 0 | 0 | 2 | 0.0% |
| `jiff` | 0.2.28 | 90 | 3 | 87 | 3 | 0 | 0 | 3.3% |
| `uuid` | 1.23.2 | 14 | 1 | 13 | 1 | 0 | 0 | 7.1% |
| `url` | 2.5.8 | 10 | 2 | 8 | 1 | 0 | 1 | 10.0% |
| `regex` | 1.12.3 | 364 | 0 | 364 | 0 | 0 | 0 | 0.0% |
| `serde_json` | 1.0.150 | 32 | 1 | 31 | 1 | 0 | 0 | 3.1% |
| `toml` | 1.1.2+spec-1.1.0 | 72 | 0 | 72 | 0 | 0 | 0 | 0.0% |
| `geo-types` | 0.7.19 | 20 | 0 | 20 | 0 | 0 | 0 | 0.0% |
| `geo` | 0.33.1 | 102 | 0 | 102 | 0 | 0 | 0 | 0.0% |
| `geojson` | 1.0.0 | 20 | 6 | 14 | 0 | 0 | 6 | 0.0% |
| `georaster` | 0.2.0 | 5 | 5 | 0 | 0 | 0 | 4 | 0.0% |
| `rstar` | 0.8.4 | 17 | 0 | 17 | 0 | 0 | 0 | 0.0% |
| `proj` | 0.31.0 | 48 | 0 | 48 | 0 | 0 | 0 | 0.0% |
| `wkt` | 0.14.0 | 13 | 0 | 13 | 0 | 0 | 0 | 0.0% |
| `wkb` | 0.9.2 | 16 | 0 | 16 | 0 | 0 | 0 | 0.0% |
| `redb` | 4.1.0 | 41 | 0 | 41 | 0 | 0 | 0 | 0.0% |
| `csv` | 1.4.0 | 43 | 0 | 43 | 0 | 0 | 0 | 0.0% |
| `accesskit` | 0.24.0 | 35 | 32 | 3 | 32 | 0 | 0 | 91.4% |
| `reqwest` | 0.12.28 | 31 | 4 | 27 | 0 | 0 | 4 | 0.0% |
| `elicitation` | 0.11.1 | 1278 | 416 | 862 | 416 | 0 | 0 | 32.6% |
| **Total** | | **2367** | **478** | **1889** | **460** | **0** | **18** | **19.4%** |

`OurTraitsDone` counts all elicitation-owned traits that are actually implementable for the type. Lifetime-bound types such as `Pixels<'a, R>` are not expected to implement `Elicitation` or `ElicitIntrospect` because `Elicitation` requires `'static`.

`ExternallyBlocked` means the implementable elicitation-owned traits are present, but direct `ElicitComplete` is blocked by missing `Serialize`, `Deserialize`, or `JsonSchema` on the target type.

### Impl Gaps

| Kind | Count | Notes |
|------|------:|-------|
| MissingOurTraits | 1888 | Missing one or more elicitation-owned support traits |
| ReadyForElicitComplete | 0 | All prerequisites present; only `impl ElicitComplete` is missing |
| FeatureGatedExternal | 0 | Missing external serde/schemars traits may be unlockable with more features |
| **Total** | **1888** | |

---

## Shadow Coverage

| Upstream | Version | Shadow Crate | Covered | Drifted | Total | VerificationGaps | Coverage |
|----------|---------|-------------|--------:|--------:|------:|-----------------:|---------:|
| `bevy` | 0.19.0 | `elicit_bevy` | 280 | 46 | 4083 | 14 | 8.0% |
| `wgpu` | 29.0.3 | `elicit_wgpu` | 0 | 0 | 1415 | 0 | 0.0% |
| `egui` | 0.34.3 | `elicit_egui` | 1 | 1 | 331 | 2 | 0.6% |
| `winit` | 0.30.13 | `elicit_winit` | 1 | 0 | 129 | 1 | 0.8% |
| `ratatui` | 0.30.0 | `elicit_ratatui` | 26 | 4 | 229 | 3 | 13.1% |
| `tokio` | 1.52.3 | `elicit_tokio` | 0 | 1 | 244 | 1 | 0.4% |
| `axum` | 0.8.9 | `elicit_axum` | 0 | 0 | 201 | 0 | 0.0% |
| `reqwest` | 0.12.28 | `elicit_reqwest` | 5 | 0 | 44 | 0 | 11.4% |
| `serde` | 1.0.228 | `elicit_serde` | 0 | 2 | 201 | 0 | 1.0% |
| `serde_json` | 1.0.150 | `elicit_serde_json` | 0 | 0 | 50 | 0 | 0.0% |
| `toml` | 1.1.2+spec-1.1.0 | `elicit_toml` | 0 | 0 | 89 | 0 | 0.0% |
| `csv` | 1.4.0 | `elicit_csv` | 0 | 0 | 44 | 0 | 0.0% |
| `chrono` | 0.4.41 | `elicit_chrono` | 1 | 0 | 93 | 0 | 1.1% |
| `time` | 0.3.47 | `elicit_time` | 2 | 0 | 136 | 0 | 1.5% |
| `jiff` | 0.2.28 | `elicit_jiff` | 2 | 1 | 134 | 1 | 2.2% |
| `uuid` | 1.23.2 | `elicit_uuid` | 1 | 0 | 27 | 0 | 3.7% |
| `url` | 2.5.8 | `elicit_url` | 1 | 0 | 10 | 0 | 10.0% |
| `regex` | 1.12.3 | `elicit_regex` | 4 | 0 | 367 | 0 | 1.1% |
| `sqlx` | 0.8.6 | `elicit_sqlx` | 4 | 3 | 363 | 0 | 1.9% |
| `redb` | 4.1.0 | `elicit_redb` | 15 | 0 | 52 | 15 | 28.8% |
| `geo-types` | 0.7.19 | `elicit_geo_types` | 12 | 0 | 28 | 0 | 42.9% |
| `geo` | 0.33.1 | `elicit_geo` | 0 | 0 | 233 | 0 | 0.0% |
| `geojson` | 1.0.0 | `elicit_geojson` | 6 | 0 | 38 | 0 | 15.8% |
| `georaster` | 0.2.0 | `elicit_georaster` | 5 | 0 | 5 | 0 | 100.0% |
| `rstar` | 0.8.4 | `elicit_rstar` | 0 | 0 | 25 | 0 | 0.0% |
| `proj` | 0.31.0 | `elicit_proj` | 0 | 0 | 90 | 0 | 0.0% |
| `wkt` | 0.14.0 | `elicit_wkt` | 8 | 1 | 35 | 0 | 25.7% |
| `wkb` | 0.9.2 | `elicit_wkb` | 30 | 0 | 39 | 7 | 76.9% |
| `uom` | 0.38.0 | `elicit_uom` | 0 | 0 | 3877 | 0 | 0.0% |
| `accesskit` | 0.24.0 | `elicit_accesskit` | 30 | 0 | 38 | 30 | 78.9% |

### Shadow Gaps

| Kind | Count | Notes |
|------|------:|-------|
| Missing | 12157 | Upstream public item not yet shadowed |
| Drifted | 59 | Probable rename or naming drift in the shadow crate |
| PossiblyStale | 681 | Shadow item with no matching upstream — needs audit |
| InfrastructureExtra | 1482 | Shadow-only infrastructure item — expected |
| ShadowVerificationGap | 74 | Matched shadow type exists but is not yet `ElicitComplete`-ready |
| **Total** | **14453** | |

