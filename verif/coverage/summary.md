# elicitation Coverage Summary

_Generated: 2026-04-25_

---

## Impl Coverage

| Crate | Version | Types | ElicitComplete | Coverage |
|-------|---------|------:|---------------:|---------:|
| `uuid` | 1.23.0 | 16 | 0 | 0.0% |
| `url` | 2.5.8 | 10 | 0 | 0.0% |
| `geo-types` | 0.7.19 | 26 | 0 | 0.0% |
| `geojson` | 1.0.0 | 20 | 0 | 0.0% |
| `chrono` | 0.4.41 | 43 | 0 | 0.0% |
| `serde_json` | 1.0.149 | 33 | 0 | 0.0% |
| `elicitation` | 0.10.0 | 1201 | 216 | 18.0% |
| **Total** | | **1349** | **216** | **16.0%** |

### Impl Gaps

| Kind | Count | Notes |
|------|------:|-------|
| ReadyNow | 208 | All external traits present — only needs `impl ElicitComplete` |
| FeatureGated | 53 | Traits may appear behind a feature flag |
| NeedsExternalImpl | 872 | Missing `Serialize`, `Deserialize`, or `JsonSchema` |
| **Total** | **1133** | |

---

## Shadow Coverage

| Upstream | Version | Shadow Crate | Covered | Total | Coverage |
|----------|---------|-------------|--------:|------:|---------:|
| `bevy` | 0.18.1 | `elicit_bevy` | 270 | 4058 | 7.8% |
| `wgpu` | 27.0.1 | `elicit_wgpu` | 0 | 1471 | 0.0% |
| `egui` | 0.34.1 | `elicit_egui` | 1 | 241 | 0.8% |
| `winit` | 0.30.13 | `elicit_winit` | 0 | 105 | 0.0% |
| `ratatui` | 0.30.0 | `elicit_ratatui` | 0 | 221 | 1.8% |

### Shadow Gaps

| Kind | Count | Notes |
|------|------:|-------|
| Missing | 5825 | Upstream type not yet shadowed |
| Drifted | 54 | Probable rename — similar name in shadow crate |
| PossiblyStale | 99 | Shadow type with no matching upstream — needs audit |
| InfrastructureExtra | 836 | Our own tool params / plugins / ctx types — expected |
| **Total** | **6814** | |

