//! Collection layer: invoke `cargo rustdoc` and parse the JSON output into
//! an [`Inventory`], and scan proof harness test files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::impl_coverage::ProofHarness;
use crate::inventory::{Inventory, Item, ItemKind};

/// Short names of all 8 `ElicitComplete` supertraits, as they appear in rustdoc JSON.
const ELICIT_COMPLETE_SUPERTRAITS: &[&str] = &[
    "Serialize",
    "Deserialize",
    "JsonSchema",
    "Elicitation",
    "ElicitIntrospect",
    "ElicitSpec",
    "ElicitPromptTree",
    "ToCodeLiteral",
];

/// Which of the 8 [`ElicitComplete`] supertraits a type already implements.
///
/// The three external traits (`Serialize`, `Deserialize`, `JsonSchema`) are the
/// critical ones: the orphan rule prevents us from adding them for external types,
/// so if they are missing the type cannot become `ElicitComplete` without a
/// trenchcoat wrapper.  Our own four traits (`Elicitation`, `ElicitIntrospect`,
/// `ElicitSpec`, `ElicitPromptTree`, `ToCodeLiteral`) can always be implemented
/// for any external type since the traits are defined in our crate.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TraitPrereqs {
    // ── External traits (orphan rule blocks us from adding these) ──
    pub serialize: bool,
    pub deserialize: bool,
    pub json_schema: bool,
    // ── Our own traits (we can always add these for external types) ──
    pub elicitation_trait: bool,
    pub elicit_introspect: bool,
    pub elicit_spec: bool,
    pub elicit_prompt_tree: bool,
    pub to_code_literal: bool,
}

impl TraitPrereqs {
    /// Merge another set of prereqs into this one (logical OR on every field).
    pub fn merge(&mut self, other: &TraitPrereqs) {
        self.serialize |= other.serialize;
        self.deserialize |= other.deserialize;
        self.json_schema |= other.json_schema;
        self.elicitation_trait |= other.elicitation_trait;
        self.elicit_introspect |= other.elicit_introspect;
        self.elicit_spec |= other.elicit_spec;
        self.elicit_prompt_tree |= other.elicit_prompt_tree;
        self.to_code_literal |= other.to_code_literal;
    }

    /// True when `Serialize + Deserialize + JsonSchema` are all satisfied —
    /// the three traits we cannot add ourselves for external types.
    pub fn can_be_direct(&self) -> bool {
        self.serialize && self.deserialize && self.json_schema
    }

    /// Short names of the external traits that are still missing.
    pub fn external_blockers(&self) -> Vec<&'static str> {
        let mut b = Vec::new();
        if !self.serialize {
            b.push("Serialize");
        }
        if !self.deserialize {
            b.push("Deserialize");
        }
        if !self.json_schema {
            b.push("JsonSchema");
        }
        b
    }

    /// External traits that are absent, annotated for the gaps report.
    pub fn external_blockers_absent(&self) -> Vec<String> {
        let mut b = Vec::new();
        if !self.serialize {
            b.push("Serialize(absent)".to_string());
        }
        if !self.deserialize {
            b.push("Deserialize(absent)".to_string());
        }
        if !self.json_schema {
            b.push("JsonSchema(absent)".to_string());
        }
        b
    }
}

/// Scan a rustdoc JSON file and return a map from canonical type path to
/// [`TraitPrereqs`] recording which of the 8 `ElicitComplete` supertraits each
/// type already implements.
///
/// Pass the same `own_crate` / `prefix_match` arguments used for the companion
/// [`parse_rustdoc_json`] call so that only items in scope are considered.
#[instrument(skip(json_path), fields(path = %json_path.display(), own_crate))]
pub fn collect_trait_prereqs(
    json_path: &Path,
    own_crate: &str,
    prefix_match: bool,
) -> ElicitDocResult<HashMap<String, TraitPrereqs>> {
    let content =
        std::fs::read_to_string(json_path).map_err(|e| ElicitDocError::io(e.to_string()))?;
    let krate: rustdoc_types::Crate =
        serde_json::from_str(&content).map_err(|e| ElicitDocError::rustdoc_parse(e.to_string()))?;

    let own_crate_normalized = own_crate.replace('-', "_");
    let own_crate_key = own_crate_normalized.as_str();

    let mut map: HashMap<String, TraitPrereqs> = HashMap::new();

    for item in krate.index.values() {
        let rustdoc_types::ItemEnum::Impl(impl_item) = &item.inner else {
            continue;
        };

        let Some(trait_) = &impl_item.trait_ else {
            continue;
        };

        let trait_short = trait_.path.split("::").last().unwrap_or("");
        if !ELICIT_COMPLETE_SUPERTRAITS.contains(&trait_short) {
            continue;
        }

        // Resolve the implementing type to its canonical path.
        let rustdoc_types::Type::ResolvedPath(rp) = &impl_item.for_ else {
            continue;
        };
        let Some(summary) = krate.paths.get(&rp.id) else {
            continue;
        };

        // Apply the same crate-name filter used in extract_items.
        let first = summary.path.first().map(String::as_str).unwrap_or("");
        let in_scope = if prefix_match {
            first.starts_with(own_crate_key)
        } else {
            first == own_crate_key
        };
        if !in_scope {
            continue;
        }

        let type_path = summary.path.join("::");
        let prereqs = map.entry(type_path).or_default();
        match trait_short {
            "Serialize" => prereqs.serialize = true,
            "Deserialize" => prereqs.deserialize = true,
            "JsonSchema" => prereqs.json_schema = true,
            "Elicitation" => prereqs.elicitation_trait = true,
            "ElicitIntrospect" => prereqs.elicit_introspect = true,
            "ElicitSpec" => prereqs.elicit_spec = true,
            "ElicitPromptTree" => prereqs.elicit_prompt_tree = true,
            "ToCodeLiteral" => prereqs.to_code_literal = true,
            _ => {}
        }
    }

    tracing::debug!(
        types_with_prereqs = map.len(),
        "collected trait prereqs from JSON"
    );

    Ok(map)
}

/// The set of types that have `impl ElicitComplete for T` in the elicitation crate,
/// extracted directly from its rustdoc JSON (not from the inventory heuristic).
#[derive(Debug, Clone, Default)]
pub struct ElicitCompleteSet {
    /// Full paths of concrete `ElicitComplete` impls, e.g.:
    /// - `"std::sync::atomic::AtomicBool"` (external type, direct impl)
    /// - `"elicitation::AlignSelect"` (internal type; `crate::` normalized)
    pub concrete: HashSet<String>,
    /// Full paths of factory (generic) `ElicitComplete` impls, e.g.:
    /// - `"elicitation::Tuple3"` (impl over generic params)
    pub factory: HashSet<String>,
}

/// Scan the elicitation rustdoc JSON and return the set of types that have an
/// `impl ElicitComplete for T` block, split into concrete and factory impls.
///
/// `json_path` should point to `{workspace}/target/doc/elicitation.json`.
///
/// Paths are resolved via the rustdoc ID→path map so that elicitation-internal
/// types are stored with their canonical module path (e.g.
/// `"elicitation::primitives::tower_types::handles::TowerBalanceHandle"`)
/// matching what [`parse_rustdoc_json`] produces for the source inventory.
#[instrument(skip(json_path), fields(path = %json_path.display()))]
pub fn collect_elicit_complete_paths(json_path: &Path) -> ElicitDocResult<ElicitCompleteSet> {
    let content =
        std::fs::read_to_string(json_path).map_err(|e| ElicitDocError::io(e.to_string()))?;

    let krate: rustdoc_types::Crate =
        serde_json::from_str(&content).map_err(|e| ElicitDocError::rustdoc_parse(e.to_string()))?;

    let mut concrete: HashSet<String> = HashSet::new();
    let mut factory: HashSet<String> = HashSet::new();

    for item in krate.index.values() {
        let rustdoc_types::ItemEnum::Impl(impl_item) = &item.inner else {
            continue;
        };

        // Only care about `impl ElicitComplete for T`
        let is_elicit_complete = impl_item
            .trait_
            .as_ref()
            .map(|t| t.path == "ElicitComplete")
            .unwrap_or(false);

        if !is_elicit_complete {
            continue;
        }

        // Factory: impl has at least one type-parameter
        let is_factory = impl_item.generics.params.iter().any(|p| {
            matches!(p.kind, rustdoc_types::GenericParamDefKind::Type { .. })
        });

        // Resolve the canonical path via the ID→path map. This handles re-exports
        // correctly: `for_` might say `crate::TowerBalanceHandle` (re-exported),
        // but the paths map gives the canonical full path used in the inventory.
        let path = match &impl_item.for_ {
            rustdoc_types::Type::ResolvedPath(p) => {
                if let Some(summary) = krate.paths.get(&p.id) {
                    // Use the canonical path from the paths map (same as parse_rustdoc_json)
                    summary.path.join("::")
                } else {
                    // Fallback: normalize crate-relative path
                    p.path.replace("crate::", "elicitation::")
                }
            }
            rustdoc_types::Type::Primitive(name) => name.clone(),
            _ => continue,
        };

        if is_factory {
            factory.insert(path);
        } else {
            concrete.insert(path);
        }
    }

    tracing::debug!(
        concrete = concrete.len(),
        factory = factory.len(),
        "collected ElicitComplete paths from JSON"
    );

    Ok(ElicitCompleteSet { concrete, factory })
}

/// Invoke `cargo rustdoc --output-format json` for a **workspace member** crate
/// at `workspace_root`, then parse the resulting JSON into an [`Inventory`].
///
/// Pass `features` as extra feature flags, e.g. `&["uuid", "chrono"]`.
#[instrument(skip(workspace_root), fields(crate_name, workspace_root = %workspace_root.display()))]
pub fn collect_inventory(
    workspace_root: &Path,
    crate_name: &str,
    features: &[&str],
) -> ElicitDocResult<Inventory> {
    let json_path = run_cargo_rustdoc(workspace_root, crate_name, features)?;
    // Workspace members: exact name match (no transitive dep bleed)
    parse_rustdoc_json(&json_path, crate_name, false)
}

/// Collect the [`Inventory`] for an **external dependency** (not a workspace
/// member) by locating it via `cargo metadata` in `reference_workspace` and
/// running `cargo rustdoc` directly against its manifest.
///
/// `activated_features` are the exact features our workspace has declared for
/// this dep (e.g. `&["json", "cookies", "stream"]` for reqwest).  The build
/// uses exactly those features — no `--all-features` guessing.  This gives an
/// accurate picture of what trait impls are available in the context we actually
/// use the crate in.
#[instrument(skip(reference_workspace), fields(crate_name))]
pub fn collect_dep_inventory(
    reference_workspace: &Path,
    crate_name: &str,
    activated_features: &[&str],
) -> ElicitDocResult<Inventory> {
    let manifest = find_dep_manifest(reference_workspace, crate_name)?;
    let crate_dir = manifest
        .parent()
        .ok_or_else(|| ElicitDocError::cargo_invocation(format!("no parent dir for {manifest:?}")))?;

    // Use a shared target dir under elicit_doc so we don't write into the
    // registry cache, and reuse build artefacts across multiple dep runs.
    let own_target = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(crate_dir)
        .arg("+nightly")
        .arg("rustdoc");
    if !activated_features.is_empty() {
        cmd.arg("--features").arg(activated_features.join(","));
    }
    cmd.arg("--target-dir")
        .arg(&own_target)
        .arg("--")
        .arg("--output-format")
        .arg("json")
        .arg("-Z")
        .arg("unstable-options");

    tracing::debug!(
        manifest = %manifest.display(),
        features = ?activated_features,
        "running cargo rustdoc on dep"
    );
    let status = cmd
        .status()
        .map_err(|e| ElicitDocError::cargo_invocation(e.to_string()))?;

    if !status.success() {
        return Err(ElicitDocError::cargo_invocation(format!(
            "cargo rustdoc for dep {crate_name} exited with {status}"
        )));
    }

    let normalized = crate_name.replace('-', "_");
    let json_path = own_target
        .join("doc")
        .join(format!("{normalized}.json"));

    if !json_path.exists() {
        return Err(ElicitDocError::rustdoc_missing(
            json_path.display().to_string(),
        ));
    }

    tracing::debug!(path = %json_path.display(), "dep rustdoc JSON produced");
    // Deps: prefix match so umbrella crates like `bevy` include `bevy_ecs::*` etc.
    parse_rustdoc_json(&json_path, crate_name, true)
}

/// Scan the elicitation rustdoc JSON and return all `(foreign_type, wrapper)` pairs
/// found in `impl From<ForeignType> for OurWrapper` blocks where:
/// - `OurWrapper` is in the `elicitation` namespace
/// - `ForeignType` is not in the `elicitation`, `std`, `core`, or `alloc` namespaces
///
/// This captures both `select_trenchcoat!`-generated newtypes and hand-written
/// owned wrappers (e.g. `BevyColor`, `EguiColor32`) without requiring either to
/// already be `ElicitComplete`, making it suitable for gap analysis.
#[instrument(skip(json_path), fields(path = %json_path.display()))]
pub fn collect_trenchcoat_pairs(json_path: &Path) -> ElicitDocResult<Vec<(String, String)>> {
    let content =
        std::fs::read_to_string(json_path).map_err(|e| ElicitDocError::io(e.to_string()))?;
    let krate: rustdoc_types::Crate =
        serde_json::from_str(&content).map_err(|e| ElicitDocError::rustdoc_parse(e.to_string()))?;

    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for item in krate.index.values() {
        let rustdoc_types::ItemEnum::Impl(impl_item) = &item.inner else {
            continue;
        };

        let Some(trait_) = &impl_item.trait_ else {
            continue;
        };

        // Only care about `From` (match by last segment to handle full paths like core::convert::From)
        let trait_short = trait_.path.split("::").last().unwrap_or("");
        if trait_short != "From" {
            continue;
        }

        // `for_` must be an elicitation-namespace type (our wrapper)
        let rustdoc_types::Type::ResolvedPath(wrapper_rp) = &impl_item.for_ else {
            continue;
        };
        let wrapper_path = if let Some(summary) = krate.paths.get(&wrapper_rp.id) {
            summary.path.join("::")
        } else {
            let p = wrapper_rp.path.replace("crate::", "elicitation::");
            if !p.starts_with("elicitation::") {
                continue;
            }
            p
        };
        if !wrapper_path.starts_with("elicitation::") {
            continue;
        }

        // Extract T from `From<T>` via the trait's angle-bracket args
        let Some(generic_args) = trait_.args.as_deref() else {
            continue;
        };
        let rustdoc_types::GenericArgs::AngleBracketed { args: angle_args, .. } = generic_args
        else {
            continue;
        };
        let Some(rustdoc_types::GenericArg::Type(inner_ty)) = angle_args.first() else {
            continue;
        };
        let rustdoc_types::Type::ResolvedPath(foreign_rp) = inner_ty else {
            continue; // primitives, references, tuples, etc. are not trenchcoat targets
        };

        // Resolve the foreign type's canonical path
        let foreign_path = if let Some(summary) = krate.paths.get(&foreign_rp.id) {
            summary.path.join("::")
        } else {
            continue; // can't identify the foreign type reliably — skip
        };

        // Skip elicitation-internal types (From<OurType> for OurType conversions)
        if foreign_path.starts_with("elicitation::") {
            continue;
        }
        // Skip std/core/alloc — From<String>, From<u32>, etc. are not trenchcoats
        if foreign_path.starts_with("std::")
            || foreign_path.starts_with("core::")
            || foreign_path.starts_with("alloc::")
        {
            continue;
        }

        let pair = (foreign_path, wrapper_path);
        if seen.insert(pair.clone()) {
            pairs.push(pair);
        }
    }

    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    tracing::debug!(count = pairs.len(), "found trenchcoat pairs from From<T> impls");
    Ok(pairs)
}

/// Scan `cargo metadata` for a dep and return the names of features that are likely
/// to unlock `serde`, `schemars`, or `serde_json` support.
///
/// A feature is included if its name OR any of the items it enables (direct deps or
/// sub-features) mentions `"serde"`, `"schemars"`, or `"json"` (case-insensitive).
///
/// This is used to populate `candidate_unlock_features` on [`crate::gaps::ImplGapEntry`]
/// rows where the dep build fell back to default features — giving an actionable list
/// of feature flags to add to `Cargo.toml` rather than a vague "feature_gated" label.
///
/// Uses `--all-features` on the workspace so that optional deps (like `reqwest`,
/// which is behind a feature gate in elicitation) are included in the resolved graph.
#[instrument(skip(reference_workspace), fields(crate_name))]
pub fn collect_dep_serde_features(
    reference_workspace: &Path,
    crate_name: &str,
) -> ElicitDocResult<Vec<String>> {
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(reference_workspace.join("Cargo.toml"))
        .features(cargo_metadata::CargoOpt::AllFeatures)
        .exec()
        .map_err(|e| ElicitDocError::cargo_metadata(e.to_string()))?;

    let normalized = crate_name.replace('-', "_");
    let pkg = meta
        .packages
        .iter()
        .find(|p| p.name.as_str() == crate_name || p.name.replace('-', "_") == normalized)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "package '{crate_name}' not found in workspace metadata"
            ))
        })?;

    const KEYWORDS: &[&str] = &["serde", "schemars", "schema", "json"];

    let mut candidates: Vec<String> = pkg
        .features
        .iter()
        .filter(|(name, enables)| {
            let name_lc = name.to_lowercase();
            KEYWORDS.iter().any(|kw| name_lc.contains(kw))
                || enables.iter().any(|dep| {
                    let dep_lc = dep.to_lowercase();
                    KEYWORDS.iter().any(|kw| dep_lc.contains(kw))
                })
        })
        .map(|(name, _)| name.clone())
        .collect();

    candidates.sort();
    tracing::debug!(
        crate_name,
        count = candidates.len(),
        ?candidates,
        "found serde-related features"
    );
    Ok(candidates)
}

/// Find the `Cargo.toml` path for a dependency named `crate_name` as seen from
/// the workspace at `reference_workspace`.
#[instrument(skip(reference_workspace), fields(crate_name))]
fn find_dep_manifest(reference_workspace: &Path, crate_name: &str) -> ElicitDocResult<PathBuf> {
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(reference_workspace.join("Cargo.toml"))
        .exec()
        .map_err(|e| ElicitDocError::cargo_metadata(e.to_string()))?;

    meta.packages
        .iter()
        .find(|p| p.name.as_str() == crate_name)
        .map(|p| PathBuf::from(p.manifest_path.as_std_path()))
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "dependency '{crate_name}' not found in cargo metadata for {reference_workspace:?}"
            ))
        })
}

/// Run `cargo rustdoc -p <crate> --output-format json` and return the path
/// to the generated JSON file.
#[instrument(skip(workspace_root), fields(crate_name))]
fn run_cargo_rustdoc(
    workspace_root: &Path,
    crate_name: &str,
    features: &[&str],
) -> ElicitDocResult<PathBuf> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root)
        .arg("+nightly")
        .arg("rustdoc")
        .arg("-p")
        .arg(crate_name);

    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }

    cmd.arg("--")
        .arg("--output-format")
        .arg("json")
        .arg("-Z")
        .arg("unstable-options");

    tracing::debug!("running cargo rustdoc");
    let status = cmd
        .status()
        .map_err(|e| ElicitDocError::cargo_invocation(e.to_string()))?;

    if !status.success() {
        return Err(ElicitDocError::cargo_invocation(format!(
            "cargo rustdoc for {crate_name} exited with {status}"
        )));
    }

    // Rustdoc writes to target/doc/<crate_name>.json (underscores, not hyphens)
    let normalized = crate_name.replace('-', "_");
    let json_path = workspace_root
        .join("target")
        .join("doc")
        .join(format!("{normalized}.json"));

    if !json_path.exists() {
        return Err(ElicitDocError::rustdoc_missing(
            json_path.display().to_string(),
        ));
    }

    tracing::debug!(path = %json_path.display(), "rustdoc JSON produced");
    Ok(json_path)
}

/// Parse a rustdoc JSON file into an [`Inventory`].
#[instrument(skip(json_path), fields(path = %json_path.display()))]
fn parse_rustdoc_json(
    json_path: &Path,
    crate_name: &str,
    prefix_match: bool,
) -> ElicitDocResult<Inventory> {
    let content =
        std::fs::read_to_string(json_path).map_err(|e| ElicitDocError::io(e.to_string()))?;

    let krate: rustdoc_types::Crate =
        serde_json::from_str(&content).map_err(|e| ElicitDocError::rustdoc_parse(e.to_string()))?;

    let version = krate
        .crate_version
        .clone()
        .unwrap_or_else(|| "unknown".to_string());

    tracing::debug!(
        item_count = krate.index.len(),
        version = %version,
        "parsed rustdoc JSON"
    );

    let items = extract_items(&krate, crate_name, prefix_match);

    Ok(Inventory {
        crate_name: crate_name.to_string(),
        crate_version: version,
        items,
    })
}

/// Extract all public items from a rustdoc [`Crate`] into our flat [`Item`] list.
///
/// For re-exporting umbrella crates (like `bevy`) the `index` only contains
/// a handful of module items while all re-exported items live in `paths`.
/// We therefore build the inventory from `paths` and look up the `index` entry
/// only for additional generics detail when available.
///
/// `prefix_match`: when `true`, items are accepted if their first path segment
/// **starts with** `own_crate` (e.g. `"bevy"` accepts `bevy_ecs::*`, `bevy_math::*`).
/// When `false`, the first segment must equal `own_crate` exactly.
fn extract_items(krate: &rustdoc_types::Crate, own_crate: &str, prefix_match: bool) -> Vec<Item> {
    let mut items = Vec::new();
    // Rustdoc JSON paths always use underscores even when the Cargo.toml package
    // name is hyphenated (e.g. "geo-types" → "geo_types").
    let own_crate_normalized = own_crate.replace('-', "_");
    let own_crate_key = own_crate_normalized.as_str();

    for (id, summary) in &krate.paths {
        // Filter to items in this crate's namespace (exact or prefix match).
        let matches = summary
            .path
            .first()
            .map(|s| {
                if prefix_match {
                    s.starts_with(own_crate_key)
                } else {
                    s.as_str() == own_crate_key
                }
            })
            .unwrap_or(false);

        if !matches {
            continue;
        }

        let kind = match summary.kind {
            rustdoc_types::ItemKind::Struct => ItemKind::Struct,
            rustdoc_types::ItemKind::Enum => ItemKind::Enum,
            rustdoc_types::ItemKind::Trait => ItemKind::Trait,
            rustdoc_types::ItemKind::TypeAlias => ItemKind::TypeAlias,
            rustdoc_types::ItemKind::Function => ItemKind::Function,
            rustdoc_types::ItemKind::Macro => ItemKind::Macro,
            rustdoc_types::ItemKind::Constant => ItemKind::Constant,
            rustdoc_types::ItemKind::Module => ItemKind::Module,
            _ => continue, // skip primitives, unions, impls, etc.
        };

        let path = summary.path.clone();
        let name = path
            .last()
            .cloned()
            .unwrap_or_default();

        if name.is_empty() {
            continue;
        }

        // Attempt to read generics from the index entry (only present when
        // the item is defined in this crate, not re-exported from a subcrate).
        let (is_generic, type_params) = krate
            .index
            .get(id)
            .map(|item| {
                let (_, g, tp) = classify_item(item);
                (g, tp)
            })
            .unwrap_or((false, vec![]));

        items.push(Item {
            path,
            kind,
            name,
            is_generic,
            type_params,
        });
    }

    items.sort_by(|a, b| a.path.cmp(&b.path));
    tracing::debug!(count = items.len(), "extracted items");
    items
}

/// Map a rustdoc item to our [`ItemKind`], and extract generics info.
fn classify_item(item: &rustdoc_types::Item) -> (ItemKind, bool, Vec<String>) {
    match &item.inner {
        rustdoc_types::ItemEnum::Struct(s) => {
            let params = extract_generic_params(&s.generics);
            let is_generic = !params.is_empty();
            (ItemKind::Struct, is_generic, params)
        }
        rustdoc_types::ItemEnum::Enum(e) => {
            let params = extract_generic_params(&e.generics);
            let is_generic = !params.is_empty();
            (ItemKind::Enum, is_generic, params)
        }
        rustdoc_types::ItemEnum::Trait(t) => {
            let params = extract_generic_params(&t.generics);
            let is_generic = !params.is_empty();
            (ItemKind::Trait, is_generic, params)
        }
        rustdoc_types::ItemEnum::TypeAlias(t) => {
            let params = extract_generic_params(&t.generics);
            let is_generic = !params.is_empty();
            (ItemKind::TypeAlias, is_generic, params)
        }
        rustdoc_types::ItemEnum::Function(_) => (ItemKind::Function, false, vec![]),
        rustdoc_types::ItemEnum::Macro(_) => (ItemKind::Macro, false, vec![]),
        rustdoc_types::ItemEnum::Constant { .. } => (ItemKind::Constant, false, vec![]),
        rustdoc_types::ItemEnum::Module(_) => (ItemKind::Module, false, vec![]),
        _ => (ItemKind::Other, false, vec![]),
    }
}

/// Extract type parameter names from a [`Generics`] block.
fn extract_generic_params(generics: &rustdoc_types::Generics) -> Vec<String> {
    generics
        .params
        .iter()
        .filter_map(|p| match &p.kind {
            rustdoc_types::GenericParamDefKind::Type { .. } => Some(p.name.clone()),
            _ => None,
        })
        .collect()
}

/// Scan a proof harness test file and return a [`ProofHarness`] containing the
/// set of type names found in `assert_proofs_non_empty::<T>()` and
/// `assert_kani_contains::<Outer, Inner>()` calls.
///
/// Uses regex-free text scanning — the patterns are regular enough that simple
/// string extraction is sufficient and avoids a `syn` dependency.
#[instrument(skip(path), fields(path = %path.display()))]
pub fn collect_proof_harness(path: &Path) -> ElicitDocResult<ProofHarness> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| ElicitDocError::io(format!("{}: {e}", path.display())))?;

    let mut non_empty_types: HashSet<String> = HashSet::new();
    let mut composition_pairs: Vec<(String, String)> = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();

        // assert_proofs_non_empty::<SomeType>(...) or ::<SomeType<Param>>(...)
        if let Some(ty) = extract_turbofish_arg(trimmed, "assert_proofs_non_empty") {
            non_empty_types.insert(ty);
        }

        // assert_kani_contains::<Outer, Inner>(...)
        if let Some((outer, inner)) = extract_kani_contains(trimmed) {
            composition_pairs.push((outer, inner));
        }
    }

    tracing::debug!(
        non_empty = non_empty_types.len(),
        composition = composition_pairs.len(),
        "scanned proof harness"
    );

    Ok(ProofHarness {
        non_empty_types,
        composition_pairs,
    })
}

/// Extract the type argument from a `fn_name::<TYPE>(...)` turbofish call.
/// Returns `None` if the pattern is not present.
fn extract_turbofish_arg(line: &str, fn_name: &str) -> Option<String> {
    let prefix = format!("{fn_name}::<");
    let start = line.find(&prefix)? + prefix.len();
    let rest = &line[start..];
    // Find matching `>` — handle nested generics by counting angle brackets
    let end = find_matching_angle(rest)?;
    let ty = rest[..end].trim().to_string();
    if ty.is_empty() { None } else { Some(ty) }
}

/// Extract `(Outer, Inner)` from `assert_kani_contains::<Outer, Inner>(...)`.
fn extract_kani_contains(line: &str) -> Option<(String, String)> {
    let prefix = "assert_kani_contains::<";
    let start = line.find(prefix)? + prefix.len();
    let rest = &line[start..];
    let end = find_matching_angle(rest)?;
    let inner_str = &rest[..end];
    // Split on the comma separating Outer and Inner (respecting nested `<>`)
    let comma = find_top_level_comma(inner_str)?;
    let outer = inner_str[..comma].trim().to_string();
    let inner = inner_str[comma + 1..].trim().to_string();
    if outer.is_empty() || inner.is_empty() {
        None
    } else {
        Some((outer, inner))
    }
}

/// Find the index of the closing `>` that matches the first `<` already consumed.
/// Handles nesting: `HashMap<String, Vec<bool>>` → returns index of final `>`.
fn find_matching_angle(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the index of a `,` at depth 0 (not inside nested `<>`).
fn find_top_level_comma(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth -= 1,
            ',' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}
