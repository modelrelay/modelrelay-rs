//! Auto-generated types from OpenAPI spec.
//!
//! Types are generated at compile time from `api/openapi/api.json` via build.rs.
//! No generated files are checked into git - types are always fresh.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use modelrelay::generated::{Customer, Tier, Project, MessageRole, Citation};
//! use modelrelay::generated::MessageRoleExt; // Extension trait for as_str()
//! ```

// Suppress clippy warnings for generated code from typify.
// These are style preferences, not bugs. Fixing them would require complex
// AST manipulation (e.g., adding #[derive(Default)] attributes to structs).
// The `suspicious_else_formatting` warning IS fixed at the source in build.rs.
#![allow(clippy::derivable_impls)] // typify generates manual Default impls
#![allow(clippy::redundant_closure_call)]
#![allow(clippy::needless_lifetimes)]
#![allow(clippy::match_single_binding)]
#![allow(clippy::clone_on_copy)]

include!(concat!(env!("OUT_DIR"), "/generated_types.rs"));

mod extensions;
pub use extensions::MessageRoleExt;

#[cfg(test)]
mod tests;
