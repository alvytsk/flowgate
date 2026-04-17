/// Zero-size stub for `OperationMeta` when the `openapi` feature is disabled.
///
/// All builder methods are no-ops that return `Self`. This allows user code
/// to compile identically regardless of whether `openapi` is enabled.
#[derive(Clone, Debug, Default)]
pub struct OperationMeta;

impl OperationMeta {
    pub fn new() -> Self {
        Self
    }
    pub fn summary(self, _: impl Into<String>) -> Self {
        self
    }
    pub fn description(self, _: impl Into<String>) -> Self {
        self
    }
    pub fn operation_id(self, _: impl Into<String>) -> Self {
        self
    }
    pub fn tag(self, _: impl Into<String>) -> Self {
        self
    }
    pub fn deprecated(self) -> Self {
        self
    }
    pub fn response(self, _status: u16, _description: impl Into<String>) -> Self {
        self
    }
}
