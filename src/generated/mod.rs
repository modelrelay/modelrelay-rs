//! Auto-generated types from OpenAPI spec.
//!
//! Types are generated at compile time from `api/openapi/api.json` via build.rs.
//! No generated files are checked into git - types are always fresh.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use modelrelay::generated::{Customer, Tier, Project};
//! ```

// Suppress clippy warnings for generated code
#![allow(clippy::derivable_impls)]
#![allow(clippy::redundant_closure_call)]
#![allow(clippy::needless_lifetimes)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::clone_on_copy)]

include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));

#[cfg(test)]
mod tests;
