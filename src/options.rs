#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// The level of validation applied while decoding.
pub enum Validation {
    /// Checks structural validity needed to decode the requested value.
    #[default]
    Basic,
    /// Additionally validates skipped text strings and nested item structure.
    Strict,
    /// Requires deterministic encoding as defined by RFC 8949.
    Deterministic,
}

#[derive(Clone, Copy, Debug)]
/// Validation and resource-limit settings for decoding.
pub struct DecodeOptions {
    /// The validation level to apply.
    pub validation: Validation,
    /// The maximum permitted nesting depth.
    pub max_depth: usize,
    /// The maximum number of elements permitted in a collection.
    pub max_collection_len: usize,
    /// Whether [`crate::SliceDecoder::finish`] accepts unconsumed input.
    pub allow_trailing: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            validation: Validation::Basic,
            max_depth: 128,
            max_collection_len: 16 * 1024 * 1024,
            allow_trailing: false,
        }
    }
}
