//! OpenAPI spec generation and Scalar documentation UI.
//!
//! This module has two forms:
//! - With the `openapi` feature: full spec generation, a Scalar-based `/docs`
//!   endpoint, and route-level [`OperationMeta`] annotations.
//! - Without the feature: only a zero-sized [`OperationMeta`] stub whose
//!   builders are no-ops, so user code compiles identically either way.

#[cfg(feature = "openapi")]
pub mod meta;
#[cfg(feature = "openapi")]
pub(crate) mod spec;
#[cfg(feature = "openapi")]
pub(crate) mod ui;

#[cfg(feature = "openapi")]
pub use meta::OperationMeta;

/// Zero-size stub for `OperationMeta` when the `openapi` feature is disabled.
///
/// All builder methods are no-ops that return `Self`, so user code compiles
/// identically regardless of whether `openapi` is enabled.
#[cfg(not(feature = "openapi"))]
#[derive(Clone, Debug, Default)]
pub struct OperationMeta;

#[cfg(not(feature = "openapi"))]
impl OperationMeta {
    /// Create a new (no-op) OperationMeta.
    pub fn new() -> Self {
        Self
    }
    /// No-op without the `openapi` feature.
    pub fn summary(self, _: impl Into<String>) -> Self {
        self
    }
    /// No-op without the `openapi` feature.
    pub fn description(self, _: impl Into<String>) -> Self {
        self
    }
    /// No-op without the `openapi` feature.
    pub fn operation_id(self, _: impl Into<String>) -> Self {
        self
    }
    /// No-op without the `openapi` feature.
    pub fn tag(self, _: impl Into<String>) -> Self {
        self
    }
    /// No-op without the `openapi` feature.
    pub fn deprecated(self) -> Self {
        self
    }
    /// No-op without the `openapi` feature.
    pub fn response(self, _status: u16, _description: impl Into<String>) -> Self {
        self
    }
}
