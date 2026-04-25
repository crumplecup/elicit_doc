//! Collection layer: invoke `cargo rustdoc` and parse the JSON output into
//! an [`Inventory`], and scan proof harness test files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::instrument;

use crate::error::{ElicitDocError, ElicitDocResult};
use crate::impl_coverage::ProofHarness;
use crate::inventory::{Inventory, Item, ItemKind};

/// Invoke `cargo rustdoc --output-format json` for `crate_name` inside the
/// workspace at `workspace_root`, then parse the resulting JSON into an
/// [`Inventory`].
///
/// Pass `features` as extra feature flags, e.g. `&["uuid", "chrono"]`.
#[instrument(skip(workspace_root), fields(crate_name, workspace_root = %workspace_root.display()))]
pub fn collect_inventory(
    workspace_root: &Path,
    crate_name: &str,
    features: &[&str],
) -> ElicitDocResult<Inventory> {
    let json_path = run_cargo_rustdoc(workspace_root, crate_name, features)?;
    parse_rustdoc_json(&json_path, crate_name)
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
        .arg("rustdoc")
        .arg("-p")
        .arg(crate_name)
        .arg("--")
        .arg("--output-format")
        .arg("json")
        .arg("-Z")
        .arg("unstable-options");

    if !features.is_empty() {
        cmd.arg("--features").arg(features.join(","));
    }

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
fn parse_rustdoc_json(json_path: &Path, crate_name: &str) -> ElicitDocResult<Inventory> {
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

    let items = extract_items(&krate);

    Ok(Inventory {
        crate_name: crate_name.to_string(),
        crate_version: version,
        items,
    })
}

/// Extract all public items from a rustdoc [`Crate`] into our flat [`Item`] list.
fn extract_items(krate: &rustdoc_types::Crate) -> Vec<Item> {
    let mut items = Vec::new();

    for (id, item) in &krate.index {
        // Only include items that appear in the path map (i.e. are reachable
        // from the crate root) and skip items that are private/hidden.
        let Some(summary) = krate.paths.get(id) else {
            continue;
        };

        if summary.kind == rustdoc_types::ItemKind::Primitive {
            // Primitives like bool/i32 appear in rustdoc JSON for std but under
            // a synthetic path — include them with their simple name.
        }

        let path = summary.path.clone();
        let name = path
            .last()
            .cloned()
            .unwrap_or_else(|| item.name.clone().unwrap_or_default());

        let (kind, is_generic, type_params) = classify_item(item);

        if kind == ItemKind::Other {
            continue;
        }

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
