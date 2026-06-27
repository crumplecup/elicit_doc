//! Collection layer: invoke `cargo rustdoc` and parse the JSON output into
//! an [`Inventory`], and scan proof harness test files.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::{debug, instrument};

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
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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

    /// Short names of our five elicitation-owned traits that are still missing.
    pub fn missing_our_traits(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.elicitation_trait {
            missing.push("Elicitation");
        }
        if !self.elicit_introspect {
            missing.push("ElicitIntrospect");
        }
        if !self.elicit_spec {
            missing.push("ElicitSpec");
        }
        if !self.elicit_prompt_tree {
            missing.push("ElicitPromptTree");
        }
        if !self.to_code_literal {
            missing.push("ToCodeLiteral");
        }
        missing
    }

    /// True when all five elicitation-owned support traits are present.
    pub fn our_traits_complete(&self) -> bool {
        self.missing_our_traits().is_empty()
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

/// How the reference workspace currently depends on an upstream crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DepBuildConfig {
    /// Exact upstream features enabled by the workspace dependency declaration.
    pub activated_features: Vec<String>,
    /// Whether the workspace leaves the dependency's default feature set enabled.
    pub uses_default_features: bool,
}

/// Per-type feature probe result used for actionable impl-gap reporting.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypeFeatureProbe {
    /// Cargo package whose feature namespace should be used for unlock guidance.
    pub feature_crate: String,
    /// Candidate features that are available upstream but not active here.
    pub candidate_unlock_features: Vec<String>,
    /// Trait prereqs observed when probing the report crate with candidate features enabled.
    pub probed_prereqs: Option<TraitPrereqs>,
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
    let own_crate_normalized = own_crate.replace('-', "_");
    let own_crate_key = own_crate_normalized.as_str();
    collect_trait_prereqs_matching(json_path, |path| {
        let first = path.first().map(String::as_str).unwrap_or("");
        if prefix_match {
            first.starts_with(own_crate_key)
        } else {
            first == own_crate_key
        }
    })
}

/// Scan a rustdoc JSON file and collect trait prereqs for the concrete type paths
/// tracked in `inventory`, including foreign types pulled in from public signatures.
#[instrument(skip(json_path, inventory), fields(path = %json_path.display(), crate_name = %inventory.crate_name))]
pub fn collect_trait_prereqs_for_inventory(
    json_path: &Path,
    inventory: &Inventory,
) -> ElicitDocResult<HashMap<String, TraitPrereqs>> {
    let tracked_paths = inventory_trait_match_paths(inventory);
    collect_trait_prereqs_matching(json_path, |path| tracked_paths.contains(&path.join("::")))
}

fn collect_trait_prereqs_matching<F>(
    json_path: &Path,
    mut include_path: F,
) -> ElicitDocResult<HashMap<String, TraitPrereqs>>
where
    F: FnMut(&[String]) -> bool,
{
    let content =
        std::fs::read_to_string(json_path).map_err(|e| ElicitDocError::io(e.to_string()))?;
    let krate: rustdoc_types::Crate =
        serde_json::from_str(&content).map_err(|e| ElicitDocError::rustdoc_parse(e.to_string()))?;

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
        if !include_path(&summary.path) {
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

fn inventory_trait_match_paths(inventory: &Inventory) -> HashSet<String> {
    let mut paths: HashSet<String> = inventory.type_items().map(Item::path_str).collect();
    let crate_root = inventory.crate_name.replace('-', "_");
    let mut name_counts: HashMap<&str, usize> = HashMap::new();

    for item in inventory.type_items() {
        *name_counts.entry(item.name.as_str()).or_insert(0) += 1;
    }

    for item in inventory.type_items() {
        if item.path.first().map(String::as_str) != Some(crate_root.as_str()) {
            continue;
        }
        if name_counts.get(item.name.as_str()).copied() != Some(1) {
            continue;
        }
        paths.insert(format!("{crate_root}::{}", item.name));
    }

    paths
}

/// The set of types that have `impl ElicitComplete for T` in a local crate,
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

/// Scan a local crate rustdoc JSON and return the set of types that have an
/// `impl ElicitComplete for T` block, split into concrete and factory impls.
///
/// `json_path` should point to `{workspace}/target/doc/<crate>.json`.
/// `local_crate_name` is the crate whose `crate::` paths should be normalized.
///
/// Paths are resolved via the rustdoc ID→path map so that local-crate
/// types are stored with their canonical module path (e.g.
/// `"elicitation::primitives::tower_types::handles::TowerBalanceHandle"`)
/// matching what [`parse_rustdoc_json`] produces for the source inventory.
#[instrument(skip(json_path), fields(path = %json_path.display()))]
pub fn collect_elicit_complete_paths(
    json_path: &Path,
    local_crate_name: &str,
) -> ElicitDocResult<ElicitCompleteSet> {
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
        let is_factory = impl_item
            .generics
            .params
            .iter()
            .any(|p| matches!(p.kind, rustdoc_types::GenericParamDefKind::Type { .. }));

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
                    p.path.replace("crate::", &format!("{local_crate_name}::"))
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
    let (inventory, _) = collect_inventory_with_json_path(workspace_root, crate_name, features)?;
    Ok(inventory)
}

/// Invoke `cargo rustdoc --output-format json` for a **workspace member** crate
/// and return both the parsed [`Inventory`] and the exact JSON path that rustdoc produced.
///
/// This is useful when follow-on analyses need to read the same JSON again
/// (for example to inspect impl blocks) without re-guessing where cargo placed it.
#[instrument(skip(workspace_root), fields(crate_name, workspace_root = %workspace_root.display()))]
pub fn collect_inventory_with_json_path(
    workspace_root: &Path,
    crate_name: &str,
    features: &[&str],
) -> ElicitDocResult<(Inventory, PathBuf)> {
    let json_path = run_cargo_rustdoc(workspace_root, crate_name, features)?;
    // Workspace members: exact name match (no transitive dep bleed)
    let inventory = parse_rustdoc_json(&json_path, crate_name, false)?;
    Ok((inventory, json_path))
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
    uses_default_features: bool,
) -> ElicitDocResult<Inventory> {
    collect_member_dep_inventory(
        reference_workspace,
        "elicitation",
        crate_name,
        activated_features,
        uses_default_features,
    )
}

/// Collect the [`Inventory`] for an external dependency as resolved from a
/// specific workspace member.
#[instrument(skip(reference_workspace), fields(member_crate_name, crate_name))]
pub fn collect_member_dep_inventory(
    reference_workspace: &Path,
    member_crate_name: &str,
    crate_name: &str,
    activated_features: &[&str],
    uses_default_features: bool,
) -> ElicitDocResult<Inventory> {
    let (inventory, _) = collect_member_dep_inventory_with_json_path(
        reference_workspace,
        member_crate_name,
        crate_name,
        activated_features,
        uses_default_features,
    )?;
    Ok(inventory)
}

/// Collect the [`Inventory`] for an external dependency and return the exact rustdoc JSON
/// path that was produced for the build.
#[instrument(skip(reference_workspace), fields(crate_name))]
pub fn collect_dep_inventory_with_json_path(
    reference_workspace: &Path,
    crate_name: &str,
    activated_features: &[&str],
    uses_default_features: bool,
) -> ElicitDocResult<(Inventory, PathBuf)> {
    collect_member_dep_inventory_with_json_path(
        reference_workspace,
        "elicitation",
        crate_name,
        activated_features,
        uses_default_features,
    )
}

/// Collect the [`Inventory`] for an external dependency as resolved from a
/// specific workspace member and return the exact rustdoc JSON path.
#[instrument(skip(reference_workspace), fields(member_crate_name, crate_name))]
pub fn collect_member_dep_inventory_with_json_path(
    reference_workspace: &Path,
    member_crate_name: &str,
    crate_name: &str,
    activated_features: &[&str],
    uses_default_features: bool,
) -> ElicitDocResult<(Inventory, PathBuf)> {
    let json_path = run_dep_cargo_rustdoc(
        reference_workspace,
        member_crate_name,
        crate_name,
        activated_features,
        uses_default_features,
    )?;
    let inventory = parse_rustdoc_json(&json_path, crate_name, true)?;
    Ok((inventory, json_path))
}

fn run_dep_cargo_rustdoc(
    reference_workspace: &Path,
    member_crate_name: &str,
    crate_name: &str,
    activated_features: &[&str],
    uses_default_features: bool,
) -> ElicitDocResult<PathBuf> {
    let manifest = find_dep_manifest(reference_workspace, member_crate_name, crate_name)?;
    let crate_dir = manifest.parent().ok_or_else(|| {
        ElicitDocError::cargo_invocation(format!("no parent dir for {manifest:?}"))
    })?;

    // Use a shared target dir under elicit_doc so we don't write into the
    // registry cache, and reuse build artefacts across multiple dep runs.
    let own_target = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(crate_dir).arg("+nightly").arg("rustdoc");
    if !uses_default_features {
        cmd.arg("--no-default-features");
    }
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
    let json_path = own_target.join("doc").join(format!("{normalized}.json"));

    if !json_path.exists() {
        return Err(ElicitDocError::rustdoc_missing(
            json_path.display().to_string(),
        ));
    }

    tracing::debug!(path = %json_path.display(), "dep rustdoc JSON produced");
    Ok(json_path)
}

/// Resolve the dependency features and `default-features` setting the `elicitation`
/// crate currently uses for an upstream dep.
#[instrument(skip(reference_workspace), fields(crate_name))]
pub fn collect_dep_build_config(
    reference_workspace: &Path,
    crate_name: &str,
) -> ElicitDocResult<DepBuildConfig> {
    collect_member_dep_build_config(reference_workspace, "elicitation", crate_name)
}

/// Resolve the dependency features and `default-features` setting a specific
/// workspace member currently uses for an upstream dep.
#[instrument(skip(reference_workspace), fields(member_crate_name, crate_name))]
pub fn collect_member_dep_build_config(
    reference_workspace: &Path,
    member_crate_name: &str,
    crate_name: &str,
) -> ElicitDocResult<DepBuildConfig> {
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(reference_workspace.join("Cargo.toml"))
        .exec()
        .map_err(|e| ElicitDocError::cargo_metadata(e.to_string()))?;

    let member_pkg = meta
        .packages
        .iter()
        .find(|pkg| pkg.name == member_crate_name)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "workspace package `{member_crate_name}` not found in cargo metadata"
            ))
        })?;

    let normalized = crate_name.replace('-', "_");
    let dep = member_pkg
        .dependencies
        .iter()
        .find(|dep| {
            dep.name == crate_name
                || dep.name.replace('-', "_") == normalized
                || dep.rename.as_deref().is_some_and(|rename| {
                    rename == crate_name || rename.replace('-', "_") == normalized
                })
        })
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "dependency '{crate_name}' not found in `{member_crate_name}` package metadata"
            ))
        })?;

    let mut activated_features = dep.features.clone();
    activated_features.sort();
    activated_features.dedup();

    Ok(DepBuildConfig {
        activated_features,
        uses_default_features: dep.uses_default_features,
    })
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

    Ok(collect_trenchcoat_pairs_from_crate(&krate))
}

fn collect_trenchcoat_pairs_from_crate(krate: &rustdoc_types::Crate) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for item in krate.index.values() {
        let rustdoc_types::ItemEnum::Impl(impl_item) = &item.inner else {
            continue;
        };

        collect_trenchcoat_pairs_from_from_impl(krate, impl_item, &mut seen, &mut pairs);
        collect_trenchcoat_pairs_from_wrapper_methods(krate, impl_item, &mut seen, &mut pairs);
    }

    pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

    tracing::debug!(
        count = pairs.len(),
        "found trenchcoat pairs from wrapper structure"
    );
    pairs
}

fn collect_trenchcoat_pairs_from_from_impl(
    krate: &rustdoc_types::Crate,
    impl_item: &rustdoc_types::Impl,
    seen: &mut HashSet<(String, String)>,
    pairs: &mut Vec<(String, String)>,
) {
    let Some(trait_) = &impl_item.trait_ else {
        return;
    };

    // Only care about `From` (match by last segment to handle full paths like core::convert::From)
    let trait_short = trait_.path.split("::").last().unwrap_or("");
    if trait_short != "From" {
        return;
    }

    let Some(wrapper_path) = impl_target_path(krate, &impl_item.for_) else {
        return;
    };
    if !is_elicitation_wrapper_path(&wrapper_path) {
        return;
    }

    // Extract T from `From<T>` via the trait's angle-bracket args
    let Some(generic_args) = trait_.args.as_deref() else {
        return;
    };
    let rustdoc_types::GenericArgs::AngleBracketed {
        args: angle_args, ..
    } = generic_args
    else {
        return;
    };
    let Some(rustdoc_types::GenericArg::Type(inner_ty)) = angle_args.first() else {
        return;
    };
    let rustdoc_types::Type::ResolvedPath(foreign_rp) = inner_ty else {
        return;
    };

    let Some(summary) = krate.paths.get(&foreign_rp.id) else {
        return;
    };

    record_trenchcoat_pair(seen, pairs, summary.path.join("::"), wrapper_path);
}

fn collect_trenchcoat_pairs_from_wrapper_methods(
    krate: &rustdoc_types::Crate,
    impl_item: &rustdoc_types::Impl,
    seen: &mut HashSet<(String, String)>,
    pairs: &mut Vec<(String, String)>,
) {
    if impl_item.trait_.is_some() {
        return;
    }

    let Some(wrapper_path) = impl_target_path(krate, &impl_item.for_) else {
        return;
    };
    if !is_elicitation_wrapper_path(&wrapper_path) {
        return;
    }

    for method_id in &impl_item.items {
        let Some(method_item) = krate.index.get(method_id) else {
            continue;
        };
        if !item_is_public(method_item) {
            continue;
        }

        let Some(method_name) = method_item.name.as_deref() else {
            continue;
        };
        if !matches!(method_name, "build_raw" | "into_inner") {
            continue;
        }

        let rustdoc_types::ItemEnum::Function(function) = &method_item.inner else {
            continue;
        };
        let Some(output) = &function.sig.output else {
            continue;
        };

        let mut foreign_paths = HashSet::new();
        collect_foreign_paths_from_type(krate, output, &mut foreign_paths);
        for foreign_path in foreign_paths {
            record_trenchcoat_pair(seen, pairs, foreign_path, wrapper_path.clone());
        }
    }
}

fn impl_target_path(krate: &rustdoc_types::Crate, ty: &rustdoc_types::Type) -> Option<String> {
    let rustdoc_types::Type::ResolvedPath(resolved) = ty else {
        return None;
    };

    if let Some(summary) = krate.paths.get(&resolved.id) {
        Some(summary.path.join("::"))
    } else {
        let normalized = resolved.path.replace("crate::", "elicitation::");
        normalized
            .starts_with("elicitation::")
            .then_some(normalized)
    }
}

fn is_elicitation_wrapper_path(path: &str) -> bool {
    path.starts_with("elicitation::")
}

fn is_foreign_trenchcoat_target(path: &str) -> bool {
    !(path.starts_with("elicitation::")
        || path.starts_with("std::")
        || path.starts_with("core::")
        || path.starts_with("alloc::"))
}

fn record_trenchcoat_pair(
    seen: &mut HashSet<(String, String)>,
    pairs: &mut Vec<(String, String)>,
    foreign_path: String,
    wrapper_path: String,
) {
    if !is_foreign_trenchcoat_target(&foreign_path) {
        return;
    }

    let pair = (foreign_path, wrapper_path);
    if seen.insert(pair.clone()) {
        pairs.push(pair);
    }
}

fn collect_foreign_paths_from_type(
    krate: &rustdoc_types::Crate,
    ty: &rustdoc_types::Type,
    foreign_paths: &mut HashSet<String>,
) {
    match ty {
        rustdoc_types::Type::ResolvedPath(resolved) => {
            if let Some(summary) = krate.paths.get(&resolved.id) {
                let path = summary.path.join("::");
                if is_foreign_trenchcoat_target(&path) {
                    let _ = foreign_paths.insert(path);
                }
            }
            if let Some(args) = &resolved.args {
                collect_foreign_paths_from_generic_args(krate, args, foreign_paths);
            }
        }
        rustdoc_types::Type::BorrowedRef { type_, .. }
        | rustdoc_types::Type::Slice(type_)
        | rustdoc_types::Type::RawPointer { type_, .. } => {
            collect_foreign_paths_from_type(krate, type_, foreign_paths);
        }
        rustdoc_types::Type::Tuple(types) => {
            for inner in types {
                collect_foreign_paths_from_type(krate, inner, foreign_paths);
            }
        }
        rustdoc_types::Type::Array { type_, .. } => {
            collect_foreign_paths_from_type(krate, type_, foreign_paths);
        }
        rustdoc_types::Type::QualifiedPath {
            self_type,
            trait_,
            args,
            ..
        } => {
            collect_foreign_paths_from_type(krate, self_type, foreign_paths);
            if let Some(trait_) = trait_ {
                collect_foreign_paths_from_path(krate, trait_, foreign_paths);
            }
            if let Some(args) = args {
                collect_foreign_paths_from_generic_args(krate, args, foreign_paths);
            }
        }
        rustdoc_types::Type::FunctionPointer(function_pointer) => {
            for (_, input) in &function_pointer.sig.inputs {
                collect_foreign_paths_from_type(krate, input, foreign_paths);
            }
            if let Some(output) = &function_pointer.sig.output {
                collect_foreign_paths_from_type(krate, output, foreign_paths);
            }
        }
        rustdoc_types::Type::DynTrait(dyn_trait) => {
            for poly_trait in &dyn_trait.traits {
                collect_foreign_paths_from_path(krate, &poly_trait.trait_, foreign_paths);
            }
        }
        rustdoc_types::Type::ImplTrait(bounds) => {
            for bound in bounds {
                if let rustdoc_types::GenericBound::TraitBound { trait_, .. } = bound {
                    collect_foreign_paths_from_path(krate, trait_, foreign_paths);
                }
            }
        }
        rustdoc_types::Type::Generic(_)
        | rustdoc_types::Type::Primitive(_)
        | rustdoc_types::Type::Infer => {}
        _ => {}
    }
}

fn collect_foreign_paths_from_path(
    krate: &rustdoc_types::Crate,
    path: &rustdoc_types::Path,
    foreign_paths: &mut HashSet<String>,
) {
    if let Some(summary) = krate.paths.get(&path.id) {
        let resolved = summary.path.join("::");
        if is_foreign_trenchcoat_target(&resolved) {
            let _ = foreign_paths.insert(resolved);
        }
    }
    if let Some(args) = &path.args {
        collect_foreign_paths_from_generic_args(krate, args, foreign_paths);
    }
}

fn collect_foreign_paths_from_generic_args(
    krate: &rustdoc_types::Crate,
    args: &rustdoc_types::GenericArgs,
    foreign_paths: &mut HashSet<String>,
) {
    match args {
        rustdoc_types::GenericArgs::AngleBracketed { args, constraints } => {
            for arg in args {
                if let rustdoc_types::GenericArg::Type(ty) = arg {
                    collect_foreign_paths_from_type(krate, ty, foreign_paths);
                }
            }
            for constraint in constraints {
                if let Some(args) = &constraint.args {
                    collect_foreign_paths_from_generic_args(krate, args, foreign_paths);
                }
                match &constraint.binding {
                    rustdoc_types::AssocItemConstraintKind::Equality(term) => {
                        collect_foreign_paths_from_term(krate, term, foreign_paths);
                    }
                    rustdoc_types::AssocItemConstraintKind::Constraint(bounds) => {
                        for bound in bounds {
                            if let rustdoc_types::GenericBound::TraitBound { trait_, .. } = bound {
                                collect_foreign_paths_from_path(krate, trait_, foreign_paths);
                            }
                        }
                    }
                }
            }
        }
        rustdoc_types::GenericArgs::Parenthesized { inputs, output } => {
            for input in inputs {
                collect_foreign_paths_from_type(krate, input, foreign_paths);
            }
            if let Some(output) = output {
                collect_foreign_paths_from_type(krate, output, foreign_paths);
            }
        }
        rustdoc_types::GenericArgs::ReturnTypeNotation => {}
    }
}

fn collect_foreign_paths_from_term(
    krate: &rustdoc_types::Crate,
    term: &rustdoc_types::Term,
    foreign_paths: &mut HashSet<String>,
) {
    match term {
        rustdoc_types::Term::Type(ty) => collect_foreign_paths_from_type(krate, ty, foreign_paths),
        rustdoc_types::Term::Constant(_) => {}
    }
}

/// Returns `(available_serde_features, expanded_activated_features)`.
///
/// `available_serde_features` — feature names whose transitive closure reaches
/// serde / schemars / serde_json-related deps or sibling features. This catches
/// alias features and oddly-named gates rather than relying only on the feature's
/// own spelling.
///
/// `expanded_activated_features` — the transitive closure of `activated` through
/// the package's own feature graph.  For example, if `geo` has `use-serde → serde`,
/// and we activate `["use-serde"]`, the expanded set is `{"use-serde", "serde"}`.
/// This prevents spurious `FeatureGated` classification when an alias feature name
/// is used (e.g. `use-serde` activates `serde` indirectly).
#[instrument(
    skip(reference_workspace, activated),
    fields(crate_name, uses_default_features)
)]
pub fn collect_dep_serde_features(
    reference_workspace: &Path,
    crate_name: &str,
    activated: &[&str],
    uses_default_features: bool,
) -> ElicitDocResult<(Vec<String>, Vec<String>)> {
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(reference_workspace.join("Cargo.toml"))
        .features(cargo_metadata::CargoOpt::AllFeatures)
        .exec()
        .map_err(|e| ElicitDocError::cargo_metadata(e.to_string()))?;

    let normalized = crate_name.replace('-', "_");
    let pkg = meta
        .packages
        .iter()
        .find(|pkg| pkg.name == crate_name || pkg.name.replace('-', "_") == normalized)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "package '{crate_name}' not found in workspace metadata"
            ))
        })?;

    let available = collect_available_serde_features(&pkg.features);
    let mut activated_owned: Vec<String> = activated
        .iter()
        .map(|feature| (*feature).to_string())
        .collect();
    if uses_default_features && pkg.features.contains_key("default") {
        activated_owned.push("default".to_string());
    }
    let expanded_activated = expand_same_package_features(&pkg.features, &activated_owned);

    tracing::debug!(
        crate_name,
        available_count = available.len(),
        ?available,
        expanded_activated_count = expanded_activated.len(),
        ?expanded_activated,
        "collected dep serde features"
    );
    Ok((available, expanded_activated))
}

#[instrument(skip(features))]
fn collect_available_serde_features(
    features: &std::collections::BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    const KEYWORDS: &[&str] = &["serde", "schemars", "schema", "json"];
    let mut available: Vec<String> = features
        .keys()
        .filter(|name| {
            name.as_str() != "default" && feature_reaches_external_support(name, features, KEYWORDS)
        })
        .cloned()
        .collect();
    available.sort();
    available
}

#[instrument(skip(features, activated))]
fn expand_same_package_features(
    features: &std::collections::BTreeMap<String, Vec<String>>,
    activated: &[String],
) -> Vec<String> {
    let mut expanded: std::collections::HashSet<String> = activated.iter().cloned().collect();
    let mut queue: std::collections::VecDeque<String> = activated.iter().cloned().collect();

    while let Some(feature) = queue.pop_front() {
        let Some(edges) = features.get(&feature) else {
            continue;
        };

        for edge in edges {
            if !edge.contains(':') && !edge.contains('/') && expanded.insert(edge.clone()) {
                queue.push_back(edge.clone());
            }
        }
    }

    let mut expanded_features: Vec<String> = expanded.into_iter().collect();
    expanded_features.sort();
    expanded_features
}

fn feature_reaches_external_support(
    root: &str,
    features: &std::collections::BTreeMap<String, Vec<String>>,
    keywords: &[&str],
) -> bool {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: std::collections::VecDeque<String> = std::iter::once(root.to_string()).collect();

    while let Some(feature) = queue.pop_front() {
        if !seen.insert(feature.clone()) {
            continue;
        }

        let feature_lc = feature.to_lowercase();
        if keywords.iter().any(|kw| feature_lc.contains(kw)) {
            return true;
        }

        let Some(edges) = features.get(&feature) else {
            continue;
        };

        for edge in edges {
            let edge_lc = edge.to_lowercase();
            if keywords.iter().any(|kw| edge_lc.contains(kw)) {
                return true;
            }

            // Same-package feature edge.
            if !edge.contains(':') && !edge.contains('/') {
                queue.push_back(edge.clone());
                continue;
            }

            // dep:serde / foo?/serde / foo/serde_with kinds of edges.
            let normalized = edge
                .trim_start_matches("dep:")
                .replace('?', "/")
                .replace(':', "/");
            if normalized.split('/').any(|segment| {
                keywords
                    .iter()
                    .any(|kw| segment.to_lowercase().contains(kw))
            }) {
                return true;
            }
        }
    }

    false
}

/// Find the `Cargo.toml` path for a dependency named `crate_name` as seen from
/// the workspace at `reference_workspace`.
#[instrument(skip(reference_workspace), fields(crate_name))]
fn find_dep_manifest(
    reference_workspace: &Path,
    member_crate_name: &str,
    crate_name: &str,
) -> ElicitDocResult<PathBuf> {
    let meta = cargo_metadata::MetadataCommand::new()
        .manifest_path(reference_workspace.join("Cargo.toml"))
        .exec()
        .map_err(|e| ElicitDocError::cargo_metadata(e.to_string()))?;

    let member_pkg = meta
        .packages
        .iter()
        .find(|pkg| pkg.name == member_crate_name)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "workspace package `{member_crate_name}` not found in cargo metadata"
            ))
        })?;
    let resolve = meta.resolve.as_ref().ok_or_else(|| {
        ElicitDocError::cargo_invocation(format!(
            "cargo metadata resolve graph missing while locating dependency `{crate_name}` for `{member_crate_name}`"
        ))
    })?;
    let member_node = resolve
        .nodes
        .iter()
        .find(|node| node.id == member_pkg.id)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "resolve node for workspace package `{member_crate_name}` not found"
            ))
        })?;
    let dependency = member_pkg
        .dependencies
        .iter()
        .find(|dep| dependency_matches_requested_crate(dep, crate_name))
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "dependency '{crate_name}' not found in `{member_crate_name}` package metadata"
            ))
        })?;
    let edge_name = dependency_edge_name(dependency);
    let dep_pkg_id = member_node
        .deps
        .iter()
        .find(|dep| dep.name == edge_name)
        .map(|dep| &dep.pkg)
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "resolved dependency edge `{edge_name}` for `{member_crate_name}` not found while locating `{crate_name}`"
            ))
        })?;

    meta.packages
        .iter()
        .find(|pkg| pkg.id == *dep_pkg_id)
        .map(|pkg| PathBuf::from(pkg.manifest_path.as_std_path()))
        .ok_or_else(|| {
            ElicitDocError::cargo_invocation(format!(
                "resolved package for dependency '{crate_name}' from `{member_crate_name}` not found in cargo metadata"
            ))
        })
}

fn dependency_matches_requested_crate(
    dependency: &cargo_metadata::Dependency,
    crate_name: &str,
) -> bool {
    let normalized = crate_name.replace('-', "_");
    dependency.name == crate_name
        || dependency.name.replace('-', "_") == normalized
        || dependency
            .rename
            .as_deref()
            .is_some_and(|rename| rename == crate_name || rename.replace('-', "_") == normalized)
}

fn dependency_edge_name(dependency: &cargo_metadata::Dependency) -> String {
    dependency
        .rename
        .clone()
        .unwrap_or_else(|| dependency.name.clone())
}

/// Run `cargo rustdoc -p <crate> --output-format json` and return the path
/// to the generated JSON file.
#[instrument(skip(workspace_root), fields(crate_name))]
fn run_cargo_rustdoc(
    workspace_root: &Path,
    crate_name: &str,
    features: &[&str],
) -> ElicitDocResult<PathBuf> {
    // Reuse a target dir under elicit_doc so report generation never writes into
    // the sibling elicitation workspace.
    let own_target = std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("target");

    let mut cmd = Command::new("cargo");
    cmd.current_dir(workspace_root)
        .arg("+nightly")
        .arg("rustdoc")
        .arg("-p")
        .arg(crate_name);

    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }

    cmd.arg("--target-dir")
        .arg(&own_target)
        .arg("--")
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
    let json_path = own_target.join("doc").join(format!("{normalized}.json"));

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
#[instrument(skip(krate), fields(own_crate, prefix_match))]
fn extract_items(krate: &rustdoc_types::Crate, own_crate: &str, prefix_match: bool) -> Vec<Item> {
    let mut items = Vec::new();
    let mut seen_paths = HashSet::new();
    // Rustdoc JSON paths always use underscores even when the Cargo.toml package
    // name is hyphenated (e.g. "geo-types" → "geo_types").
    let own_crate_normalized = own_crate.replace('-', "_");
    let own_crate_key = own_crate_normalized.as_str();
    let public_reexport_aliases =
        collect_public_same_crate_reexport_aliases(krate, own_crate_key, prefix_match);
    let public_module_paths = collect_public_module_paths(krate, own_crate_key, prefix_match);

    for item in public_reexport_aliases.values() {
        seen_paths.insert(item.path_str());
        items.push(item.clone());
    }

    for (id, summary) in &krate.paths {
        if !path_matches_scope(&summary.path, own_crate_key, prefix_match) {
            continue;
        }
        if public_reexport_aliases.contains_key(id) {
            debug!(
                target_path = %summary.path.join("::"),
                "skipping canonical same-crate path in favor of public reexport alias"
            );
            continue;
        }

        let Some(item) = build_inventory_item(krate, id, summary) else {
            continue;
        };
        if !item_path_is_publicly_reachable(&item, &public_module_paths) {
            debug!(
                item_path = %item.path_str(),
                "skipping non-publicly-reachable canonical path"
            );
            continue;
        }
        seen_paths.insert(item.path_str());
        items.push(item);
    }

    for item in
        collect_public_reexport_dependency_items(krate, own_crate_key, prefix_match, &seen_paths)
    {
        if seen_paths.insert(item.path_str()) {
            items.push(item);
        }
    }

    for item in
        collect_public_signature_dependency_items(krate, own_crate_key, prefix_match, &seen_paths)
    {
        if seen_paths.insert(item.path_str()) {
            items.push(item);
        }
    }

    items.sort_by(|a, b| a.path.cmp(&b.path));
    tracing::debug!(count = items.len(), "extracted items");
    items
}

#[instrument(skip(krate), fields(own_crate_key, prefix_match))]
fn collect_public_same_crate_reexport_aliases(
    krate: &rustdoc_types::Crate,
    own_crate_key: &str,
    prefix_match: bool,
) -> HashMap<rustdoc_types::Id, Item> {
    let mut aliases: HashMap<rustdoc_types::Id, Item> = HashMap::new();

    for (id, item) in &krate.index {
        let rustdoc_types::ItemEnum::Use(use_item) = &item.inner else {
            continue;
        };
        if !item_is_public(item) {
            continue;
        }

        let Some(use_summary) = krate.paths.get(id) else {
            continue;
        };
        if !path_matches_scope(&use_summary.path, own_crate_key, prefix_match) {
            continue;
        }

        let Some(target_id) = &use_item.id else {
            continue;
        };
        let Some(target_summary) = krate.paths.get(target_id) else {
            continue;
        };
        if !path_matches_scope(&target_summary.path, own_crate_key, prefix_match) {
            continue;
        }

        let Some(alias_item) = build_inventory_item_with_path(
            krate,
            target_id,
            target_summary.kind,
            use_summary.path.clone(),
        ) else {
            continue;
        };

        match aliases.entry(*target_id) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                debug!(
                    target_path = %target_summary.path.join("::"),
                    alias_path = %alias_item.path_str(),
                    "recorded same-crate public reexport alias"
                );
                slot.insert(alias_item);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if item_path_preferred_over(&alias_item.path, &slot.get().path) {
                    debug!(
                        target_path = %target_summary.path.join("::"),
                        previous_alias = %slot.get().path_str(),
                        alias_path = %alias_item.path_str(),
                        "replaced same-crate public reexport alias with shorter public path"
                    );
                    slot.insert(alias_item);
                }
            }
        }
    }

    aliases
}

#[instrument(skip(krate, existing_paths), fields(own_crate_key, prefix_match, existing_count = existing_paths.len()))]
fn collect_public_reexport_dependency_items(
    krate: &rustdoc_types::Crate,
    own_crate_key: &str,
    prefix_match: bool,
    existing_paths: &HashSet<String>,
) -> Vec<Item> {
    let mut discovered = Vec::new();
    let mut seen = existing_paths.clone();

    for (id, item) in &krate.index {
        let rustdoc_types::ItemEnum::Use(use_item) = &item.inner else {
            continue;
        };
        if !item_is_public(item) {
            continue;
        }
        let Some(use_summary) = krate.paths.get(id) else {
            continue;
        };
        if !path_matches_scope(&use_summary.path, own_crate_key, prefix_match) {
            continue;
        }
        let Some(target_id) = &use_item.id else {
            continue;
        };
        let Some(target_summary) = krate.paths.get(target_id) else {
            continue;
        };

        let target_path = target_summary.path.join("::");
        if path_matches_scope(&target_summary.path, own_crate_key, prefix_match)
            || target_path.starts_with("std::")
            || target_path.starts_with("core::")
            || target_path.starts_with("alloc::")
        {
            continue;
        }

        if seen.insert(target_path) {
            if let Some(item) = build_inventory_item(krate, target_id, target_summary) {
                discovered.push(item);
            }
        }
    }

    discovered
}

fn path_matches_scope(path: &[String], own_crate_key: &str, prefix_match: bool) -> bool {
    path.first()
        .map(|segment| {
            if prefix_match {
                segment.starts_with(own_crate_key)
            } else {
                segment == own_crate_key
            }
        })
        .unwrap_or(false)
}

fn build_inventory_item(
    krate: &rustdoc_types::Crate,
    id: &rustdoc_types::Id,
    summary: &rustdoc_types::ItemSummary,
) -> Option<Item> {
    build_inventory_item_with_path(krate, id, summary.kind, summary.path.clone())
}

fn build_inventory_item_with_path(
    krate: &rustdoc_types::Crate,
    id: &rustdoc_types::Id,
    kind: rustdoc_types::ItemKind,
    path: Vec<String>,
) -> Option<Item> {
    let kind = match kind {
        rustdoc_types::ItemKind::Struct => ItemKind::Struct,
        rustdoc_types::ItemKind::Enum => ItemKind::Enum,
        rustdoc_types::ItemKind::Trait => ItemKind::Trait,
        rustdoc_types::ItemKind::TypeAlias => ItemKind::TypeAlias,
        rustdoc_types::ItemKind::Function => ItemKind::Function,
        rustdoc_types::ItemKind::Macro => ItemKind::Macro,
        rustdoc_types::ItemKind::Constant => ItemKind::Constant,
        rustdoc_types::ItemKind::Module => ItemKind::Module,
        _ => return None,
    };

    let name = path.last().cloned().unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    let (is_generic, lifetime_params, type_params) = krate
        .index
        .get(id)
        .map(|item| {
            let (_, g, lp, tp) = classify_item(item);
            (g, lp, tp)
        })
        .unwrap_or((false, vec![], vec![]));

    Some(Item {
        path,
        kind,
        name,
        is_generic,
        lifetime_params,
        type_params,
    })
}

#[instrument(skip(krate), fields(own_crate_key, prefix_match))]
fn collect_public_module_paths(
    krate: &rustdoc_types::Crate,
    own_crate_key: &str,
    prefix_match: bool,
) -> HashSet<String> {
    krate
        .paths
        .iter()
        .filter_map(|(id, summary)| {
            if !path_matches_scope(&summary.path, own_crate_key, prefix_match)
                || summary.kind != rustdoc_types::ItemKind::Module
            {
                return None;
            }
            let item = krate.index.get(id)?;
            item_is_public(item).then_some(summary.path.join("::"))
        })
        .collect()
}

fn item_path_is_publicly_reachable(item: &Item, public_module_paths: &HashSet<String>) -> bool {
    if item.path.len() <= 2 {
        return true;
    }

    for idx in 1..item.path.len() - 1 {
        let module_path = item.path[..=idx].join("::");
        if !public_module_paths.contains(&module_path) {
            debug!(
                item_path = %item.path_str(),
                missing_public_module = %module_path,
                "canonical path is not publicly reachable"
            );
            return false;
        }
    }

    true
}

fn item_path_preferred_over(candidate: &[String], incumbent: &[String]) -> bool {
    candidate.len() < incumbent.len()
        || (candidate.len() == incumbent.len() && candidate < incumbent)
}

#[instrument(
    skip(krate, existing_paths),
    fields(own_crate_key, prefix_match, existing_count = existing_paths.len())
)]
fn collect_public_signature_dependency_items(
    krate: &rustdoc_types::Crate,
    own_crate_key: &str,
    prefix_match: bool,
    existing_paths: &HashSet<String>,
) -> Vec<Item> {
    let mut discovered = Vec::new();
    let mut seen = existing_paths.clone();

    for (id, item) in &krate.index {
        match &item.inner {
            rustdoc_types::ItemEnum::Function(function)
                if item_is_public(item)
                    && krate.paths.get(id).is_some_and(|summary| {
                        path_matches_scope(&summary.path, own_crate_key, prefix_match)
                    }) =>
            {
                collect_items_from_function_signature(
                    krate,
                    function,
                    own_crate_key,
                    prefix_match,
                    &mut seen,
                    &mut discovered,
                );
            }
            rustdoc_types::ItemEnum::Trait(trait_item)
                if item_is_public(item)
                    && krate.paths.get(id).is_some_and(|summary| {
                        path_matches_scope(&summary.path, own_crate_key, prefix_match)
                    }) =>
            {
                for child_id in &trait_item.items {
                    let Some(child) = krate.index.get(child_id) else {
                        continue;
                    };
                    if !item_is_public(child) {
                        continue;
                    }
                    if let rustdoc_types::ItemEnum::Function(function) = &child.inner {
                        collect_items_from_function_signature(
                            krate,
                            function,
                            own_crate_key,
                            prefix_match,
                            &mut seen,
                            &mut discovered,
                        );
                    }
                }
            }
            rustdoc_types::ItemEnum::Impl(impl_item)
                if impl_item.trait_.is_none()
                    && inherent_impl_targets_scope(
                        krate,
                        impl_item,
                        own_crate_key,
                        prefix_match,
                    ) =>
            {
                for child_id in &impl_item.items {
                    let Some(child) = krate.index.get(child_id) else {
                        continue;
                    };
                    if !item_is_public(child) {
                        continue;
                    }
                    if let rustdoc_types::ItemEnum::Function(function) = &child.inner {
                        collect_items_from_function_signature(
                            krate,
                            function,
                            own_crate_key,
                            prefix_match,
                            &mut seen,
                            &mut discovered,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    debug!(
        discovered_count = discovered.len(),
        "collected public signature dependency items"
    );

    discovered
}

fn item_is_public(item: &rustdoc_types::Item) -> bool {
    matches!(item.visibility, rustdoc_types::Visibility::Public)
}

fn inherent_impl_targets_scope(
    krate: &rustdoc_types::Crate,
    impl_item: &rustdoc_types::Impl,
    own_crate_key: &str,
    prefix_match: bool,
) -> bool {
    let rustdoc_types::Type::ResolvedPath(resolved) = &impl_item.for_ else {
        return false;
    };
    krate
        .paths
        .get(&resolved.id)
        .is_some_and(|summary| path_matches_scope(&summary.path, own_crate_key, prefix_match))
}

#[instrument(
    skip(krate, function, seen, discovered),
    fields(input_count = function.sig.inputs.len(), has_output = function.sig.output.is_some())
)]
fn collect_items_from_function_signature(
    krate: &rustdoc_types::Crate,
    function: &rustdoc_types::Function,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    for (_, input) in &function.sig.inputs {
        collect_items_from_type(krate, input, own_crate_key, prefix_match, seen, discovered);
    }
    if let Some(output) = &function.sig.output {
        collect_items_from_type(krate, output, own_crate_key, prefix_match, seen, discovered);
    }
    collect_items_from_generics(
        krate,
        &function.generics,
        own_crate_key,
        prefix_match,
        seen,
        discovered,
    );

    debug!(
        discovered_count = discovered.len(),
        "processed function signature for dependency discovery"
    );
}

#[instrument(
    skip(krate, ty, seen, discovered),
    fields(type_kind = type_kind_name(ty))
)]
fn collect_items_from_type(
    krate: &rustdoc_types::Crate,
    ty: &rustdoc_types::Type,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    match ty {
        rustdoc_types::Type::ResolvedPath(resolved) => {
            if let Some(summary) = krate.paths.get(&resolved.id) {
                let path = summary.path.join("::");
                if !path_matches_scope(&summary.path, own_crate_key, prefix_match)
                    && !path.starts_with("std::")
                    && !path.starts_with("core::")
                    && !path.starts_with("alloc::")
                {
                    if seen.insert(path) {
                        debug!(
                            discovered_path = %summary.path.join("::"),
                            "discovered signature dependency from resolved type"
                        );
                        if let Some(item) = build_inventory_item(krate, &resolved.id, summary) {
                            discovered.push(item);
                        }
                    }
                } else {
                    debug!(
                        candidate_path = %summary.path.join("::"),
                        in_scope = path_matches_scope(&summary.path, own_crate_key, prefix_match),
                        std_like = path.starts_with("std::")
                            || path.starts_with("core::")
                            || path.starts_with("alloc::"),
                        "skipping resolved type during signature dependency discovery"
                    );
                }
            }
            if let Some(args) = &resolved.args {
                collect_items_from_generic_args(
                    krate,
                    args,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
        }
        rustdoc_types::Type::BorrowedRef { type_, .. }
        | rustdoc_types::Type::RawPointer { type_, .. }
        | rustdoc_types::Type::Slice(type_)
        | rustdoc_types::Type::Array { type_, .. } => {
            collect_items_from_type(krate, type_, own_crate_key, prefix_match, seen, discovered)
        }
        rustdoc_types::Type::Tuple(items) => {
            for item in items {
                collect_items_from_type(krate, item, own_crate_key, prefix_match, seen, discovered);
            }
        }
        rustdoc_types::Type::FunctionPointer(function_pointer) => {
            for (_, input) in &function_pointer.sig.inputs {
                collect_items_from_type(
                    krate,
                    input,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            if let Some(output) = &function_pointer.sig.output {
                collect_items_from_type(
                    krate,
                    output,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            collect_items_from_generic_param_defs(
                krate,
                &function_pointer.generic_params,
                own_crate_key,
                prefix_match,
                seen,
                discovered,
            );
        }
        rustdoc_types::Type::DynTrait(dyn_trait) => {
            for bound in &dyn_trait.traits {
                collect_items_from_poly_trait(
                    krate,
                    bound,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
        }
        rustdoc_types::Type::ImplTrait(bounds) => {
            for bound in bounds {
                collect_items_from_generic_bound(
                    krate,
                    bound,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
        }
        rustdoc_types::Type::QualifiedPath {
            self_type,
            trait_,
            args,
            ..
        } => {
            collect_items_from_type(
                krate,
                self_type,
                own_crate_key,
                prefix_match,
                seen,
                discovered,
            );
            if let Some(trait_) = trait_ {
                collect_items_from_path(
                    krate,
                    trait_,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            if let Some(args) = args {
                collect_items_from_generic_args(
                    krate,
                    args,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
        }
        rustdoc_types::Type::Primitive(_)
        | rustdoc_types::Type::Generic(_)
        | rustdoc_types::Type::Infer => {}
        _ => {}
    }
}

#[instrument(
    skip(krate, path, seen, discovered),
    fields(path = %path.path)
)]
fn collect_items_from_path(
    krate: &rustdoc_types::Crate,
    path: &rustdoc_types::Path,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    if let Some(summary) = krate.paths.get(&path.id) {
        let path_str = summary.path.join("::");
        if !path_matches_scope(&summary.path, own_crate_key, prefix_match)
            && !path_str.starts_with("std::")
            && !path_str.starts_with("core::")
            && !path_str.starts_with("alloc::")
        {
            if seen.insert(path_str) {
                debug!(
                    discovered_path = %summary.path.join("::"),
                    "discovered signature dependency from path"
                );
                if let Some(item) = build_inventory_item(krate, &path.id, summary) {
                    discovered.push(item);
                }
            }
        } else {
            debug!(
                candidate_path = %summary.path.join("::"),
                in_scope = path_matches_scope(&summary.path, own_crate_key, prefix_match),
                std_like = path_str.starts_with("std::")
                    || path_str.starts_with("core::")
                    || path_str.starts_with("alloc::"),
                "skipping path during signature dependency discovery"
            );
        }
    }
    if let Some(args) = &path.args {
        collect_items_from_generic_args(krate, args, own_crate_key, prefix_match, seen, discovered);
    }
}

#[instrument(skip(krate, args, seen, discovered))]
fn collect_items_from_generic_args(
    krate: &rustdoc_types::Crate,
    args: &rustdoc_types::GenericArgs,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    match args {
        rustdoc_types::GenericArgs::AngleBracketed { args, constraints } => {
            for arg in args {
                match arg {
                    rustdoc_types::GenericArg::Type(ty) => collect_items_from_type(
                        krate,
                        ty,
                        own_crate_key,
                        prefix_match,
                        seen,
                        discovered,
                    ),
                    _ => {}
                }
            }
            for constraint in constraints {
                if let Some(args) = &constraint.args {
                    collect_items_from_generic_args(
                        krate,
                        args,
                        own_crate_key,
                        prefix_match,
                        seen,
                        discovered,
                    );
                }
                match &constraint.binding {
                    rustdoc_types::AssocItemConstraintKind::Equality(term) => {
                        collect_items_from_term(
                            krate,
                            term,
                            own_crate_key,
                            prefix_match,
                            seen,
                            discovered,
                        );
                    }
                    rustdoc_types::AssocItemConstraintKind::Constraint(bounds) => {
                        for bound in bounds {
                            collect_items_from_generic_bound(
                                krate,
                                bound,
                                own_crate_key,
                                prefix_match,
                                seen,
                                discovered,
                            );
                        }
                    }
                }
            }
        }
        rustdoc_types::GenericArgs::Parenthesized { inputs, output } => {
            for input in inputs {
                collect_items_from_type(
                    krate,
                    input,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            if let Some(output) = output {
                collect_items_from_type(
                    krate,
                    output,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
        }
        rustdoc_types::GenericArgs::ReturnTypeNotation => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use rustdoc_types::{
        Abi, Crate, ExternalCrate, Function, FunctionHeader, FunctionSignature, GenericBound,
        Generics, Id, Item, ItemEnum, ItemKind, ItemSummary, Module, Path, Target, Type, Use,
        Visibility, WherePredicate,
    };

    use super::{collect_trenchcoat_pairs_from_crate, extract_items};

    #[test]
    fn extract_items_includes_public_foreign_reexports() {
        let root_id = Id(1);
        let use_id = Id(10);
        let foreign_id = Id(20);

        let mut index = HashMap::new();
        index.insert(
            root_id,
            Item {
                id: root_id,
                crate_id: 0,
                name: Some("reqwest".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: true,
                    items: vec![use_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            use_id,
            Item {
                id: use_id,
                crate_id: 0,
                name: None,
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Use(Use {
                    source: "http::header::HeaderName".to_string(),
                    name: "HeaderName".to_string(),
                    id: Some(foreign_id),
                    is_glob: false,
                }),
            },
        );

        let mut paths = HashMap::new();
        paths.insert(
            root_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["reqwest".to_string()],
                kind: ItemKind::Module,
            },
        );
        paths.insert(
            use_id,
            ItemSummary {
                crate_id: 0,
                path: vec![
                    "reqwest".to_string(),
                    "header".to_string(),
                    "HeaderName".to_string(),
                ],
                kind: ItemKind::Use,
            },
        );
        paths.insert(
            foreign_id,
            ItemSummary {
                crate_id: 1,
                path: vec![
                    "http".to_string(),
                    "header".to_string(),
                    "HeaderName".to_string(),
                ],
                kind: ItemKind::Struct,
            },
        );

        let mut external_crates = HashMap::new();
        external_crates.insert(
            1,
            ExternalCrate {
                name: "http".to_string(),
                html_root_url: None,
                path: PathBuf::from("/tmp/http.rmeta"),
            },
        );

        let krate = Crate {
            root: root_id,
            crate_version: Some("1.0.0".to_string()),
            includes_private: false,
            index,
            paths,
            external_crates,
            target: Target {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                target_features: Vec::new(),
            },
            format_version: rustdoc_types::FORMAT_VERSION,
        };

        let items = extract_items(&krate, "reqwest", false);
        assert!(
            items
                .iter()
                .any(|item| item.path_str() == "http::header::HeaderName"),
            "expected foreign reexport to be inventoried: {items:#?}"
        );
    }

    #[test]
    fn extract_items_includes_foreign_types_from_public_where_predicates() {
        let root_id = Id(1);
        let function_id = Id(10);
        let foreign_id = Id(20);

        let mut index = HashMap::new();
        index.insert(
            root_id,
            Item {
                id: root_id,
                crate_id: 0,
                name: Some("reqwest".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: true,
                    items: vec![function_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            function_id,
            Item {
                id: function_id,
                crate_id: 0,
                name: Some("header".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Function(Function {
                    sig: FunctionSignature {
                        inputs: vec![("self".to_string(), Type::Generic("Self".to_string()))],
                        output: None,
                        is_c_variadic: false,
                    },
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: vec![WherePredicate::BoundPredicate {
                            type_: Type::ResolvedPath(Path {
                                path: "HeaderName".to_string(),
                                id: foreign_id,
                                args: None,
                            }),
                            bounds: vec![GenericBound::TraitBound {
                                trait_: Path {
                                    path: "IntoHeaderName".to_string(),
                                    id: Id(21),
                                    args: None,
                                },
                                generic_params: Vec::new(),
                                modifier: rustdoc_types::TraitBoundModifier::None,
                            }],
                            generic_params: Vec::new(),
                        }],
                    },
                    header: FunctionHeader {
                        is_const: false,
                        is_unsafe: false,
                        is_async: false,
                        abi: Abi::Rust,
                    },
                    has_body: true,
                }),
            },
        );

        let mut paths = HashMap::new();
        paths.insert(
            root_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["reqwest".to_string()],
                kind: ItemKind::Module,
            },
        );
        paths.insert(
            function_id,
            ItemSummary {
                crate_id: 0,
                path: vec![
                    "reqwest".to_string(),
                    "RequestBuilder".to_string(),
                    "header".to_string(),
                ],
                kind: ItemKind::Function,
            },
        );
        paths.insert(
            foreign_id,
            ItemSummary {
                crate_id: 1,
                path: vec![
                    "http".to_string(),
                    "header".to_string(),
                    "name".to_string(),
                    "HeaderName".to_string(),
                ],
                kind: ItemKind::Struct,
            },
        );

        let mut external_crates = HashMap::new();
        external_crates.insert(
            1,
            ExternalCrate {
                name: "http".to_string(),
                html_root_url: None,
                path: PathBuf::from("/tmp/http.rmeta"),
            },
        );

        let krate = Crate {
            root: root_id,
            crate_version: Some("1.0.0".to_string()),
            includes_private: false,
            index,
            paths,
            external_crates,
            target: Target {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                target_features: Vec::new(),
            },
            format_version: rustdoc_types::FORMAT_VERSION,
        };

        let items = extract_items(&krate, "reqwest", false);
        assert!(
            items
                .iter()
                .any(|item| item.path_str() == "http::header::name::HeaderName"),
            "expected foreign where-predicate type to be inventoried: {items:#?}"
        );
    }

    #[test]
    fn extract_items_prefers_public_same_crate_reexport_alias() {
        let root_id = Id(1);
        let internal_module_id = Id(2);
        let internal_client_id = Id(3);
        let reexport_id = Id(4);

        let mut index = HashMap::new();
        index.insert(
            root_id,
            Item {
                id: root_id,
                crate_id: 0,
                name: Some("reqwest".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: true,
                    items: vec![reexport_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            internal_module_id,
            Item {
                id: internal_module_id,
                crate_id: 0,
                name: Some("async_impl".to_string()),
                span: None,
                visibility: Visibility::Default,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: false,
                    items: vec![internal_client_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            internal_client_id,
            Item {
                id: internal_client_id,
                crate_id: 0,
                name: Some("Client".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Struct(rustdoc_types::Struct {
                    kind: rustdoc_types::StructKind::Unit,
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: Vec::new(),
                    },
                    impls: Vec::new(),
                }),
            },
        );
        index.insert(
            reexport_id,
            Item {
                id: reexport_id,
                crate_id: 0,
                name: None,
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Use(Use {
                    source: "reqwest::async_impl::client::Client".to_string(),
                    name: "Client".to_string(),
                    id: Some(internal_client_id),
                    is_glob: false,
                }),
            },
        );

        let mut paths = HashMap::new();
        paths.insert(
            root_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["reqwest".to_string()],
                kind: ItemKind::Module,
            },
        );
        paths.insert(
            internal_client_id,
            ItemSummary {
                crate_id: 0,
                path: vec![
                    "reqwest".to_string(),
                    "async_impl".to_string(),
                    "client".to_string(),
                    "Client".to_string(),
                ],
                kind: ItemKind::Struct,
            },
        );
        paths.insert(
            reexport_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["reqwest".to_string(), "Client".to_string()],
                kind: ItemKind::Use,
            },
        );

        let krate = Crate {
            root: root_id,
            crate_version: Some("1.0.0".to_string()),
            includes_private: false,
            index,
            paths,
            external_crates: HashMap::new(),
            target: Target {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                target_features: Vec::new(),
            },
            format_version: rustdoc_types::FORMAT_VERSION,
        };

        let items = extract_items(&krate, "reqwest", false);
        assert!(
            items
                .iter()
                .any(|item| item.path_str() == "reqwest::Client"),
            "expected public alias to be inventoried: {items:#?}"
        );
        assert!(
            items
                .iter()
                .all(|item| item.path_str() != "reqwest::async_impl::client::Client"),
            "expected internal canonical path to be suppressed: {items:#?}"
        );
    }

    #[test]
    fn extract_items_skips_non_publicly_reachable_canonical_paths() {
        let root_id = Id(1);
        let sealed_type_id = Id(2);

        let mut index = HashMap::new();
        index.insert(
            root_id,
            Item {
                id: root_id,
                crate_id: 0,
                name: Some("reqwest".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: true,
                    items: vec![sealed_type_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            sealed_type_id,
            Item {
                id: sealed_type_id,
                crate_id: 0,
                name: Some("Conn".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Struct(rustdoc_types::Struct {
                    kind: rustdoc_types::StructKind::Unit,
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: Vec::new(),
                    },
                    impls: Vec::new(),
                }),
            },
        );

        let mut paths = HashMap::new();
        paths.insert(
            root_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["reqwest".to_string()],
                kind: ItemKind::Module,
            },
        );
        paths.insert(
            sealed_type_id,
            ItemSummary {
                crate_id: 0,
                path: vec![
                    "reqwest".to_string(),
                    "connect".to_string(),
                    "sealed".to_string(),
                    "Conn".to_string(),
                ],
                kind: ItemKind::Struct,
            },
        );

        let krate = Crate {
            root: root_id,
            crate_version: Some("1.0.0".to_string()),
            includes_private: false,
            index,
            paths,
            external_crates: HashMap::new(),
            target: Target {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                target_features: Vec::new(),
            },
            format_version: rustdoc_types::FORMAT_VERSION,
        };

        let items = extract_items(&krate, "reqwest", false);
        assert!(
            items
                .iter()
                .all(|item| item.path_str() != "reqwest::connect::sealed::Conn"),
            "expected non-publicly-reachable canonical path to be skipped: {items:#?}"
        );
    }

    #[test]
    fn collect_trenchcoat_pairs_detects_public_build_raw_wrapper_methods() {
        let root_id = Id(1);
        let wrapper_id = Id(2);
        let impl_id = Id(3);
        let build_raw_id = Id(4);
        let result_id = Id(5);
        let proxy_id = Id(6);

        let mut index = HashMap::new();
        index.insert(
            root_id,
            Item {
                id: root_id,
                crate_id: 0,
                name: Some("elicitation".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Module(Module {
                    is_crate: true,
                    items: vec![wrapper_id, impl_id],
                    is_stripped: false,
                }),
            },
        );
        index.insert(
            wrapper_id,
            Item {
                id: wrapper_id,
                crate_id: 0,
                name: Some("ReqwestProxy".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Struct(rustdoc_types::Struct {
                    kind: rustdoc_types::StructKind::Unit,
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: Vec::new(),
                    },
                    impls: vec![impl_id],
                }),
            },
        );
        index.insert(
            impl_id,
            Item {
                id: impl_id,
                crate_id: 0,
                name: None,
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Impl(rustdoc_types::Impl {
                    is_unsafe: false,
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: Vec::new(),
                    },
                    provided_trait_methods: Vec::new(),
                    trait_: None,
                    for_: Type::ResolvedPath(Path {
                        path: "crate::ReqwestProxy".to_string(),
                        id: wrapper_id,
                        args: None,
                    }),
                    items: vec![build_raw_id],
                    is_negative: false,
                    is_synthetic: false,
                    blanket_impl: None,
                }),
            },
        );
        index.insert(
            build_raw_id,
            Item {
                id: build_raw_id,
                crate_id: 0,
                name: Some("build_raw".to_string()),
                span: None,
                visibility: Visibility::Public,
                docs: None,
                links: HashMap::new(),
                attrs: Vec::new(),
                deprecation: None,
                inner: ItemEnum::Function(Function {
                    sig: FunctionSignature {
                        inputs: vec![(
                            "self".to_string(),
                            Type::BorrowedRef {
                                lifetime: None,
                                is_mutable: false,
                                type_: Box::new(Type::Generic("Self".to_string())),
                            },
                        )],
                        output: Some(Type::ResolvedPath(Path {
                            path: "crate::ElicitResult".to_string(),
                            id: result_id,
                            args: Some(Box::new(rustdoc_types::GenericArgs::AngleBracketed {
                                args: vec![rustdoc_types::GenericArg::Type(Type::ResolvedPath(
                                    Path {
                                        path: "reqwest::Proxy".to_string(),
                                        id: proxy_id,
                                        args: None,
                                    },
                                ))],
                                constraints: Vec::new(),
                            })),
                        })),
                        is_c_variadic: false,
                    },
                    generics: Generics {
                        params: Vec::new(),
                        where_predicates: Vec::new(),
                    },
                    header: FunctionHeader {
                        is_const: false,
                        is_unsafe: false,
                        is_async: false,
                        abi: Abi::Rust,
                    },
                    has_body: true,
                }),
            },
        );

        let mut paths = HashMap::new();
        paths.insert(
            root_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["elicitation".to_string()],
                kind: ItemKind::Module,
            },
        );
        paths.insert(
            wrapper_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["elicitation".to_string(), "ReqwestProxy".to_string()],
                kind: ItemKind::Struct,
            },
        );
        paths.insert(
            result_id,
            ItemSummary {
                crate_id: 0,
                path: vec!["elicitation".to_string(), "ElicitResult".to_string()],
                kind: ItemKind::TypeAlias,
            },
        );
        paths.insert(
            proxy_id,
            ItemSummary {
                crate_id: 1,
                path: vec!["reqwest".to_string(), "Proxy".to_string()],
                kind: ItemKind::Struct,
            },
        );

        let mut external_crates = HashMap::new();
        external_crates.insert(
            1,
            ExternalCrate {
                name: "reqwest".to_string(),
                html_root_url: None,
                path: PathBuf::from("/tmp/reqwest.rmeta"),
            },
        );

        let krate = Crate {
            root: root_id,
            crate_version: Some("1.0.0".to_string()),
            includes_private: false,
            index,
            paths,
            external_crates,
            target: Target {
                triple: "x86_64-unknown-linux-gnu".to_string(),
                target_features: Vec::new(),
            },
            format_version: rustdoc_types::FORMAT_VERSION,
        };

        let pairs = collect_trenchcoat_pairs_from_crate(&krate);
        assert_eq!(
            pairs,
            vec![(
                "reqwest::Proxy".to_string(),
                "elicitation::ReqwestProxy".to_string(),
            )]
        );
    }
}

#[instrument(skip(krate, term, seen, discovered))]
fn collect_items_from_term(
    krate: &rustdoc_types::Crate,
    term: &rustdoc_types::Term,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    if let rustdoc_types::Term::Type(ty) = term {
        collect_items_from_type(krate, ty, own_crate_key, prefix_match, seen, discovered);
    }
}

#[instrument(skip(krate, bound, seen, discovered))]
fn collect_items_from_generic_bound(
    krate: &rustdoc_types::Crate,
    bound: &rustdoc_types::GenericBound,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    if let rustdoc_types::GenericBound::TraitBound {
        trait_,
        generic_params,
        ..
    } = bound
    {
        collect_items_from_path(krate, trait_, own_crate_key, prefix_match, seen, discovered);
        collect_items_from_generic_param_defs(
            krate,
            generic_params,
            own_crate_key,
            prefix_match,
            seen,
            discovered,
        );
    }
}

#[instrument(skip(krate, poly_trait, seen, discovered), fields(path = %poly_trait.trait_.path))]
fn collect_items_from_poly_trait(
    krate: &rustdoc_types::Crate,
    poly_trait: &rustdoc_types::PolyTrait,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    collect_items_from_path(
        krate,
        &poly_trait.trait_,
        own_crate_key,
        prefix_match,
        seen,
        discovered,
    );
    collect_items_from_generic_param_defs(
        krate,
        &poly_trait.generic_params,
        own_crate_key,
        prefix_match,
        seen,
        discovered,
    );
}

#[instrument(skip(krate, generic_params, seen, discovered), fields(param_count = generic_params.len()))]
fn collect_items_from_generic_param_defs(
    krate: &rustdoc_types::Crate,
    generic_params: &[rustdoc_types::GenericParamDef],
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    for generic in generic_params {
        match &generic.kind {
            rustdoc_types::GenericParamDefKind::Type {
                bounds, default, ..
            } => {
                for bound in bounds {
                    collect_items_from_generic_bound(
                        krate,
                        bound,
                        own_crate_key,
                        prefix_match,
                        seen,
                        discovered,
                    );
                }
                if let Some(default) = default {
                    collect_items_from_type(
                        krate,
                        default,
                        own_crate_key,
                        prefix_match,
                        seen,
                        discovered,
                    );
                }
            }
            rustdoc_types::GenericParamDefKind::Const { type_, .. } => {
                collect_items_from_type(
                    krate,
                    type_,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            rustdoc_types::GenericParamDefKind::Lifetime { .. } => {}
        }
    }
}

#[instrument(skip(krate, generics, seen, discovered), fields(param_count = generics.params.len(), predicate_count = generics.where_predicates.len()))]
fn collect_items_from_generics(
    krate: &rustdoc_types::Crate,
    generics: &rustdoc_types::Generics,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    collect_items_from_generic_param_defs(
        krate,
        &generics.params,
        own_crate_key,
        prefix_match,
        seen,
        discovered,
    );
    for predicate in &generics.where_predicates {
        collect_items_from_where_predicate(
            krate,
            predicate,
            own_crate_key,
            prefix_match,
            seen,
            discovered,
        );
    }
}

#[instrument(skip(krate, predicate, seen, discovered))]
fn collect_items_from_where_predicate(
    krate: &rustdoc_types::Crate,
    predicate: &rustdoc_types::WherePredicate,
    own_crate_key: &str,
    prefix_match: bool,
    seen: &mut HashSet<String>,
    discovered: &mut Vec<Item>,
) {
    match predicate {
        rustdoc_types::WherePredicate::BoundPredicate {
            type_,
            bounds,
            generic_params,
        } => {
            collect_items_from_type(krate, type_, own_crate_key, prefix_match, seen, discovered);
            for bound in bounds {
                collect_items_from_generic_bound(
                    krate,
                    bound,
                    own_crate_key,
                    prefix_match,
                    seen,
                    discovered,
                );
            }
            collect_items_from_generic_param_defs(
                krate,
                generic_params,
                own_crate_key,
                prefix_match,
                seen,
                discovered,
            );
        }
        rustdoc_types::WherePredicate::EqPredicate { lhs, rhs } => {
            collect_items_from_type(krate, lhs, own_crate_key, prefix_match, seen, discovered);
            collect_items_from_term(krate, rhs, own_crate_key, prefix_match, seen, discovered);
        }
        rustdoc_types::WherePredicate::LifetimePredicate { .. } => {}
    }
}

fn type_kind_name(ty: &rustdoc_types::Type) -> &'static str {
    match ty {
        rustdoc_types::Type::ResolvedPath(_) => "ResolvedPath",
        rustdoc_types::Type::DynTrait(_) => "DynTrait",
        rustdoc_types::Type::Generic(_) => "Generic",
        rustdoc_types::Type::Primitive(_) => "Primitive",
        rustdoc_types::Type::FunctionPointer(_) => "FunctionPointer",
        rustdoc_types::Type::Tuple(_) => "Tuple",
        rustdoc_types::Type::Slice(_) => "Slice",
        rustdoc_types::Type::Array { .. } => "Array",
        rustdoc_types::Type::ImplTrait(_) => "ImplTrait",
        rustdoc_types::Type::Infer => "Infer",
        rustdoc_types::Type::RawPointer { .. } => "RawPointer",
        rustdoc_types::Type::BorrowedRef { .. } => "BorrowedRef",
        rustdoc_types::Type::QualifiedPath { .. } => "QualifiedPath",
        _ => "Other",
    }
}

/// Map a rustdoc item to our [`ItemKind`], and extract generics info.
fn classify_item(item: &rustdoc_types::Item) -> (ItemKind, bool, Vec<String>, Vec<String>) {
    match &item.inner {
        rustdoc_types::ItemEnum::Struct(s) => {
            let (lifetime_params, type_params) = extract_generic_params(&s.generics);
            let is_generic = !lifetime_params.is_empty() || !type_params.is_empty();
            (ItemKind::Struct, is_generic, lifetime_params, type_params)
        }
        rustdoc_types::ItemEnum::Enum(e) => {
            let (lifetime_params, type_params) = extract_generic_params(&e.generics);
            let is_generic = !lifetime_params.is_empty() || !type_params.is_empty();
            (ItemKind::Enum, is_generic, lifetime_params, type_params)
        }
        rustdoc_types::ItemEnum::Trait(t) => {
            let (lifetime_params, type_params) = extract_generic_params(&t.generics);
            let is_generic = !lifetime_params.is_empty() || !type_params.is_empty();
            (ItemKind::Trait, is_generic, lifetime_params, type_params)
        }
        rustdoc_types::ItemEnum::TypeAlias(t) => {
            let (lifetime_params, type_params) = extract_generic_params(&t.generics);
            let is_generic = !lifetime_params.is_empty() || !type_params.is_empty();
            (
                ItemKind::TypeAlias,
                is_generic,
                lifetime_params,
                type_params,
            )
        }
        rustdoc_types::ItemEnum::Function(_) => (ItemKind::Function, false, vec![], vec![]),
        rustdoc_types::ItemEnum::Macro(_) => (ItemKind::Macro, false, vec![], vec![]),
        rustdoc_types::ItemEnum::Constant { .. } => (ItemKind::Constant, false, vec![], vec![]),
        rustdoc_types::ItemEnum::Module(_) => (ItemKind::Module, false, vec![], vec![]),
        _ => (ItemKind::Other, false, vec![], vec![]),
    }
}

/// Extract lifetime and type parameter names from a [`Generics`] block.
fn extract_generic_params(generics: &rustdoc_types::Generics) -> (Vec<String>, Vec<String>) {
    let mut lifetime_params = Vec::new();
    let mut type_params = Vec::new();

    for param in &generics.params {
        match &param.kind {
            rustdoc_types::GenericParamDefKind::Lifetime { .. } => {
                lifetime_params.push(param.name.clone())
            }
            rustdoc_types::GenericParamDefKind::Type { .. } => type_params.push(param.name.clone()),
            _ => {}
        }
    }

    (lifetime_params, type_params)
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
