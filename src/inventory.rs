//! Core inventory data model.
//!
//! An [`Inventory`] is a flat list of public [`Item`]s extracted from a crate's
//! rustdoc JSON. It is the input to all coverage and drift analyses.

use serde::{Deserialize, Serialize};

/// All public items from one crate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub crate_name: String,
    pub crate_version: String,
    pub items: Vec<Item>,
}

impl Inventory {
    /// All items of a specific kind.
    pub fn items_of_kind(&self, kind: ItemKind) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.kind == kind)
    }

    /// All type-like items (Struct, Enum, TypeAlias).
    pub fn type_items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(|i| i.kind.is_type())
    }
}

/// A single public item in a crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Item {
    /// Fully-qualified path segments, e.g. `["std", "collections", "HashMap"]`.
    pub path: Vec<String>,
    pub kind: ItemKind,
    pub name: String,
    /// Whether this item is generic (has type parameters).
    pub is_generic: bool,
    /// Type parameter names if generic, e.g. `["T", "K", "V"]`.
    pub type_params: Vec<String>,
}

impl Item {
    /// Dot-joined path string, e.g. `"std::collections::HashMap"`.
    pub fn path_str(&self) -> String {
        self.path.join("::")
    }
}

/// The kind of a public item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Struct,
    Enum,
    Trait,
    TypeAlias,
    Method,
    Function,
    Macro,
    Constant,
    Module,
    Other,
}

impl ItemKind {
    /// True for Struct, Enum, TypeAlias — the "type" items.
    pub fn is_type(self) -> bool {
        matches!(self, Self::Struct | Self::Enum | Self::TypeAlias)
    }
}

impl std::fmt::Display for ItemKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Struct => write!(f, "Struct"),
            Self::Enum => write!(f, "Enum"),
            Self::Trait => write!(f, "Trait"),
            Self::TypeAlias => write!(f, "TypeAlias"),
            Self::Method => write!(f, "Method"),
            Self::Function => write!(f, "Function"),
            Self::Macro => write!(f, "Macro"),
            Self::Constant => write!(f, "Constant"),
            Self::Module => write!(f, "Module"),
            Self::Other => write!(f, "Other"),
        }
    }
}
