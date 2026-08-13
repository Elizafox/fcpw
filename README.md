# fcpw

[![CI](https://github.com/Elizafox/fcpw/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Elizafox/fcpw/actions/workflows/ci.yml)

`fcpw` is a variation-tolerant CBOR codec for Rust that aims to be the fastest
available CBOR parser in the general case. The name is an acronym for the
**F**astest **C**BOR **P**arser in the **W**est. The name and phrase are
an allusion to FFTW, the Fastest Fourier Transform in the West, because I'm
not particularly original. Performance is not pursued at the expense of
reliability; correctness and fault tolerance are equal design goals.

FCPW provides a zero-copy slice decoder, structural event parser, dynamic
borrowed and owned values, Serde integration, deterministic encoding and
validation, synchronous I/O, RFC 8742 sequence iteration, diagnostic notation,
and opt-in parallel batch decoding.

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Message<'a> {
    id: u64,
    text: &'a str,
}

let message = Message { id: 7, text: "hello" };
let bytes = fcpw::to_vec(&message)?;
let decoded: Message<'_> = fcpw::from_slice(&bytes)?;
assert_eq!(decoded, message);
# Ok::<(), fcpw::Error>(())
```

Normal encoding preserves the source float width: `f32` is emitted as CBOR
binary32 and `f64` as binary64. Deterministic encoding uses the shortest exact
float width and canonicalizes NaNs. The low-level `Encoder` exposes the same
choice through `f32`/`f64` and `f32_preferred`/`f64_preferred`.

Repeated encoding can retain output capacity with `to_vec_into` or
`to_vec_deterministic_into`:

```rust
let mut output = Vec::with_capacity(1024);
fcpw::to_vec_into(&message, &mut output)?;
// The next call clears the bytes but reuses the allocation.
fcpw::to_vec_into(&message, &mut output)?;
# Ok::<(), fcpw::Error>(())
```

Repeated reader decoding can likewise retain its input allocation with
`from_reader_with_buffer`. Homogeneous numeric and boolean arrays expose
`from_slice_*_array_into` variants that reuse the result vector's capacity.

Consecutive owned items can be decoded from an input stream with
`ReaderDeserializer::deserialize_next`; its byte offset is absolute within
the stream. Refillable input cannot safely produce borrowed values, so use the
slice `Deserializer` when output borrows strings or bytes.

Typed semantic tags use `Tagged<T>`:

```rust
let bytes = fcpw::to_vec(&fcpw::Tagged::with_tag(32, "https://example.com"))?;
let tagged: fcpw::Tagged<String> = fcpw::from_slice(&bytes)?;
assert_eq!(tagged.tag, Some(32));
# Ok::<(), fcpw::Error>(())
```

`into_writer(value, writer)` provides Ciborium's argument order, while
`de`, `ser`, `tag`, and `value` modules ease imports during migration.
`write_self_describe` emits tag 55799. For serde-cbor's packed representation,
use `to_vec_packed` or `EncodeConfig::packed()`; packed encoding makes Rust
field and variant declaration order part of the wire format.

The default feature set is `std`, `alloc`, and `serde`. The scalar parser
remains available with `--no-default-features`. `parallel` enables Rayon-backed
sequence and delimited-batch decoding, while `diagnostic` enables RFC 8949
diagnostic formatting and parsing.

## API and features

- `SliceDecoder`, `Parser`, and `Encoder` provide allocation-free core CBOR
  processing.
- `Value` and `BorrowedValue` preserve CBOR-specific values, including tags,
  simple values, undefined, ordered maps with duplicate keys, and bignums.
- `Deserializer` and `Serializer` expose stateful Serde integration; the
  one-shot `from_slice`, `to_vec`, `to_slice`, and writer helpers cover common
  cases.
- `ReaderDeserializer` handles consecutive owned values from a stream, while
  `SequenceDecoder` exposes RFC 8742 sequences over slices.
- Deterministic, packed, diagnostic, and parallel operations are explicit so
  their wire-format or dependency tradeoffs remain visible at call sites.

The `alloc` feature adds owned values and collection helpers. `serde` adds the
Serde APIs and implies `alloc`; `std` adds reader/writer APIs; `diagnostic`
adds diagnostic notation; and `parallel` adds Rayon-backed batch decoding and
implies the other public API features it needs. A complete parity-oriented
walkthrough is available in [`examples/api_parity.rs`](examples/api_parity.rs).

The crate contains no unsafe code. Architecture-specific acceleration is only
enabled when it can retain the scalar implementation as its semantic oracle.

## Benchmarks

Criterion benchmarks compare FCPW with
[`cbor4ii`](https://crates.io/crates/cbor4ii),
[`serde_cbor`](https://crates.io/crates/serde_cbor), and
[`ciborium`](https://crates.io/crates/ciborium), and
[`minicbor`](https://crates.io/crates/minicbor). The latest local run produced
the following Criterion time estimates; lower is better and the fastest result
in each row is bold.

| Operation            |   Input |        FCPW | cbor4ii | serde_cbor | Ciborium | minicbor |
| -------------------- | ------: | ----------: | ------: | ---------: | -------: | -------: |
| Decode small record  |   503 B |  **295 ns** |  515 ns |     444 ns |  1.40 µs |   501 ns |
| Decode medium record | 7,210 B | **5.92 µs** | 7.78 µs |    6.82 µs |  21.1 µs |  7.10 µs |
| Encode small record  |   503 B |  **182 ns** |  357 ns |     492 ns |   471 ns |   523 ns |
| Encode medium record | 7,210 B | **1.73 µs** | 2.03 µs |    4.13 µs |  3.91 µs |  4.82 µs |

These results were measured on August 13, 2026, with Rust 1.97.1 on an Intel
Core Ultra 7 155H running Linux, using FCPW 0.1.3, cbor4ii 1.2.2,
serde_cbor 0.11.2, Ciborium 0.2.2, minicbor 2.3.0 through minicbor-serde 0.7.1,
and Criterion 0.8.2. All five implementations encode these workloads to the
same size. Results vary by hardware and toolchain; run
`cargo bench --bench codec` to measure the
[`benches/codec.rs`](benches/codec.rs) suite locally.

## MSRV and licensing

FCPW uses Rust 2024 and requires Rust 1.88 or newer. It is available under your
choice of the MIT or Apache-2.0 license.
