# Changelog

All notable changes to this project will be documented in this file.

## 0.1.0 - 2026-08-05

- Added allocation-free CBOR parsing, encoding, validation, and raw sequence
  APIs.
- Added owned and borrowed dynamic values with Serde conversion, checked
  integer conversions, and ergonomic accessors.
- Added one-shot and stateful Serde decoding and encoding, including reusable
  buffers, deterministic encoding, and optional serde-cbor-compatible packed
  encoding.
- Added typed semantic tags, self-described-CBOR output, and compatibility
  module/function aliases.
- Added bounded reader decoding, consecutive reader items, structured I/O
  errors, diagnostic notation, and parallel batch decoding.
