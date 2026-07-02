// SPDX-License-Identifier: GPL-3.0-only
pub mod l4d2center;

#[cfg(test)]
mod tests;

pub use l4d2center::{
    enrich_with_registry, fetch_index, CatalogMapStatus, L4d2CenterCatalogEntry,
    L4d2CenterIndexEntry,
};
