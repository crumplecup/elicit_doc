# elicitation Coverage Summary

_Generated: 2026-04-25_

---

## Impl Coverage

| Crate | Version | Types | ElicitComplete | Coverage |
|-------|---------|------:|---------------:|---------:|
| `chrono` | 0.4.41 | 43 | 0 | 0.0% |
| `time` | 0.3.47 | 74 | 0 | 0.0% |
| `jiff` | 0.2.23 | 91 | 0 | 0.0% |
| `uuid` | 1.23.0 | 16 | 0 | 0.0% |
| `url` | 2.5.8 | 10 | 0 | 0.0% |
| `regex` | 1.12.3 | 401 | 0 | 0.0% |
| `serde_json` | 1.0.149 | 33 | 0 | 0.0% |
| `toml` | 1.1.2+spec-1.1.0 | 74 | 0 | 0.0% |
| `geo-types` | 0.7.19 | 26 | 0 | 0.0% |
| `geo` | 0.33.0 | 126 | 0 | 0.0% |
| `geojson` | 1.0.0 | 20 | 0 | 0.0% |
| `georaster` | 0.2.0 | 5 | 0 | 0.0% |
| `rstar` | 0.8.4 | 17 | 0 | 0.0% |
| `proj` | 0.31.0 | 48 | 0 | 0.0% |
| `wkt` | 0.14.0 | 13 | 0 | 0.0% |
| `wkb` | 0.9.2 | 16 | 0 | 0.0% |
| `redb` | 4.1.0 | 41 | 0 | 0.0% |
| `csv` | 1.4.0 | 43 | 0 | 0.0% |
| `accesskit` | 0.21.1 | 32 | 0 | 0.0% |
| `reqwest` | 0.13.2 | 30 | 0 | 0.0% |
| `elicitation` | 0.10.0 | 1201 | 216 | 18.0% |
| **Total** | | **2360** | **216** | **9.2%** |

### Impl Gaps

| Kind | Count | Notes |
|------|------:|-------|
| ReadyNow | 239 | All external traits present — only needs `impl ElicitComplete` |
| FeatureGated | 131 | Traits may appear behind a feature flag |
| NeedsExternalImpl | 1774 | Missing `Serialize`, `Deserialize`, or `JsonSchema` |
| **Total** | **2144** | |

---

## Shadow Coverage

| Upstream | Version | Shadow Crate | Covered | Total | Coverage |
|----------|---------|-------------|--------:|------:|---------:|
| `bevy` | 0.18.1 | `elicit_bevy` | 270 | 4058 | 7.8% |
| `wgpu` | 27.0.1 | `elicit_wgpu` | 0 | 1471 | 0.0% |
| `egui` | 0.34.1 | `elicit_egui` | 1 | 241 | 0.8% |
| `winit` | 0.30.13 | `elicit_winit` | 0 | 105 | 0.0% |
| `ratatui` | 0.30.0 | `elicit_ratatui` | 0 | 221 | 1.8% |
| `tokio` | 1.51.1 | `elicit_tokio` | 0 | 2 | 0.0% |
| `tower` | 0.5.3 | `elicit_tower` | 0 | 5 | 0.0% |
| `axum` | 0.8.8 | `elicit_axum` | 0 | 137 | 0.0% |
| `reqwest` | 0.13.2 | `elicit_reqwest` | 5 | 30 | 16.7% |
| `serde` | 1.0.228 | `elicit_serde` | 0 | 177 | 0.0% |
| `serde_json` | 1.0.149 | `elicit_serde_json` | 0 | 33 | 0.0% |
| `toml` | 1.1.2+spec-1.1.0 | `elicit_toml` | 0 | 74 | 0.0% |
| `csv` | 1.4.0 | `elicit_csv` | 0 | 43 | 0.0% |
| `chrono` | 0.4.41 | `elicit_chrono` | 1 | 43 | 2.3% |
| `time` | 0.3.47 | `elicit_time` | 2 | 74 | 2.7% |
| `jiff` | 0.2.23 | `elicit_jiff` | 2 | 90 | 3.3% |
| `uuid` | 1.23.0 | `elicit_uuid` | 1 | 16 | 6.2% |
| `url` | 2.5.8 | `elicit_url` | 1 | 10 | 10.0% |
| `regex` | 1.12.3 | `elicit_regex` | 5 | 401 | 1.2% |
| `sqlx` | 0.8.6 | `elicit_sqlx` | 4 | 74 | 5.4% |
| `redb` | 4.1.0 | `elicit_redb` | 0 | 41 | 0.0% |
| `geo-types` | 0.7.19 | `elicit_geo_types` | 12 | 26 | 46.2% |
| `geo` | 0.33.0 | `elicit_geo` | 0 | 126 | 0.0% |
| `geojson` | 1.0.0 | `elicit_geojson` | 6 | 20 | 30.0% |
| `georaster` | 0.2.0 | `elicit_georaster` | 5 | 5 | 100.0% |
| `rstar` | 0.8.4 | `elicit_rstar` | 0 | 17 | 0.0% |
| `proj` | 0.31.0 | `elicit_proj` | 0 | 48 | 0.0% |
| `wkt` | 0.14.0 | `elicit_wkt` | 8 | 13 | 61.5% |
| `wkb` | 0.9.2 | `elicit_wkb` | 7 | 16 | 43.8% |
| `uom` | 0.38.0 | `elicit_uom` | 0 | 5737 | 0.0% |
| `accesskit` | 0.21.1 | `elicit_accesskit` | 27 | 32 | 84.4% |

### Shadow Gaps

| Kind | Count | Notes |
|------|------:|-------|
| Missing | 13029 | Upstream type not yet shadowed |
| Drifted | 55 | Probable rename — similar name in shadow crate |
| PossiblyStale | 392 | Shadow type with no matching upstream — needs audit |
| InfrastructureExtra | 1401 | Our own tool params / plugins / ctx types — expected |
| **Total** | **14877** | |

