#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! A fast, variation-tolerant CBOR codec implementing [RFC 8949].
//!
//! FCPW provides an allocation-free event parser and encoder, zero-copy
//! deserialization from byte slices, Serde integration, dynamic CBOR values,
//! deterministic encoding and validation, RFC 8742 sequence decoding, and
//! optional diagnostic-notation and parallel APIs.
//!
//! # Quick start
//!
//! ```
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, PartialEq, Serialize, Deserialize)]
//! struct Message<'a> {
//!     id: u64,
//!     text: &'a str,
//! }
//!
//! let message = Message { id: 7, text: "hello" };
//! let encoded = fcpw::to_vec(&message)?;
//! let decoded: Message<'_> = fcpw::from_slice(&encoded)?;
//! assert_eq!(decoded, message);
//! # Ok::<(), fcpw::Error>(())
//! ```
//!
//! For allocation-free processing, start with [`SliceDecoder`], [`Parser`], and
//! [`Encoder`]. With the `alloc` feature, [`Value`] and [`BorrowedValue`] retain
//! CBOR-specific constructs such as tags, simple values, undefined values,
//! duplicate map keys, and bignums.
//!
//! # Cargo features
//!
//! - **`std`** (default): reader and writer integration; implies `alloc`.
//! - **`alloc`** (default): owned values and collection helpers.
//! - **`serde`** (default): serialization and deserialization through Serde;
//!   implies `alloc`.
//! - **`diagnostic`**: RFC 8949 diagnostic notation; implies `alloc`.
//! - **`parallel`**: Rayon-backed batch decoding; implies the default public API
//!   features.
//!
//! The scalar parser and encoder remain available with `default-features = false`.
//! The crate contains no unsafe code and requires Rust 1.97 or newer.
//!
//! [RFC 8949]: https://www.rfc-editor.org/rfc/rfc8949

#[cfg(feature = "alloc")]
extern crate alloc;

mod decode;
#[cfg(feature = "diagnostic")]
pub mod diagnostic;
mod encode;
mod error;
mod options;
#[cfg(feature = "parallel")]
pub mod parallel;
#[cfg(feature = "serde")]
mod serde_codec;
#[cfg(feature = "serde")]
pub mod tag;
#[cfg(feature = "alloc")]
pub mod value;

pub use decode::{Event, Parser, RawValue, SequenceDecoder, SliceDecoder};
#[cfg(feature = "alloc")]
pub use decode::{
    from_slice_bool_array, from_slice_bool_array_into, from_slice_f32_array,
    from_slice_f32_array_into, from_slice_f64_array, from_slice_f64_array_into,
    from_slice_i8_array, from_slice_i8_array_into, from_slice_i16_array, from_slice_i16_array_into,
    from_slice_i32_array, from_slice_i32_array_into, from_slice_i64_array,
    from_slice_i64_array_into, from_slice_u8_array, from_slice_u8_array_into, from_slice_u16_array,
    from_slice_u16_array_into, from_slice_u32_array, from_slice_u32_array_into,
    from_slice_u64_array, from_slice_u64_array_into,
};
pub use encode::{Encoder, Output, SliceOutput};
pub use error::{Error, ErrorKind, Result};
pub use options::{DecodeOptions, Validation};
#[cfg(feature = "serde")]
pub use serde_codec::{
    Deserializer, DeterministicScratch, EncodeConfig, Serializer, from_slice,
    from_slice_with_options, serialized_size, to_slice, to_vec, to_vec_deterministic,
    to_vec_deterministic_into, to_vec_deterministic_into_with_scratch, to_vec_into, to_vec_packed,
};
#[cfg(feature = "serde")]
pub use tag::Tagged;

/// Serde deserialization APIs.
#[cfg(feature = "serde")]
pub mod de {
    pub use crate::{Deserializer, from_slice, from_slice_with_options};
    #[cfg(feature = "std")]
    pub use crate::{ReaderDeserializer, from_reader, from_reader_with_buffer};
}

/// Serde serialization APIs.
#[cfg(feature = "serde")]
pub mod ser {
    pub use crate::{EncodeConfig, Serializer, serialized_size, to_slice, to_vec, to_vec_packed};
    #[cfg(feature = "std")]
    pub use crate::{into_writer, to_writer};
}

/// Adapts a [`std::io::Write`] destination to [`Output`].
#[cfg(feature = "std")]
pub struct IoWrite<W> {
    inner: W,
    offset: usize,
}

#[cfg(feature = "std")]
impl<W> IoWrite<W> {
    /// Creates an output adapter around `writer`.
    pub const fn new(writer: W) -> Self {
        Self {
            inner: writer,
            offset: 0,
        }
    }

    /// Returns the wrapped writer.
    pub fn into_inner(self) -> W {
        self.inner
    }
}

#[cfg(feature = "std")]
impl<W: std::io::Write> Output for IoWrite<W> {
    fn write_all(&mut self, mut bytes: &[u8]) -> Result<()> {
        use std::io::ErrorKind as IoErrorKind;

        while !bytes.is_empty() {
            match self.inner.write(bytes) {
                Ok(0) => {
                    return Err(Error::from_io(
                        std::io::Error::from(IoErrorKind::WriteZero),
                        self.offset,
                    ));
                }
                Ok(written) => {
                    self.offset += written;
                    bytes = &bytes[written..];
                }
                Err(error) if error.kind() == IoErrorKind::Interrupted => {}
                Err(error) => return Err(Error::from_io(error, self.offset)),
            }
        }

        Ok(())
    }
}

#[cfg(feature = "alloc")]
pub use value::{BorrowedValue, Value};

#[cfg(all(feature = "serde", feature = "std"))]
/// Deserializes one CBOR item read to the end of `reader`.
pub fn from_reader<T: serde::de::DeserializeOwned, R: std::io::Read>(reader: R) -> Result<T> {
    const INITIAL_CHUNK_SIZE: usize = 8 * 1024;
    let mut buffer = alloc::vec::Vec::with_capacity(INITIAL_CHUNK_SIZE);
    from_reader_with_buffer(reader, &mut buffer)
}

/// Decodes from `reader` using caller-owned reusable input storage.
///
/// `buffer` is cleared before and after the call while retaining its capacity.
#[cfg(all(feature = "serde", feature = "std"))]
pub fn from_reader_with_buffer<T: serde::de::DeserializeOwned, R: std::io::Read>(
    mut reader: R,
    buffer: &mut alloc::vec::Vec<u8>,
) -> Result<T> {
    const INITIAL_CHUNK_SIZE: usize = 8 * 1024;
    use std::io::Read as _;

    buffer.clear();

    let result = (|| loop {
        let chunk_size = buffer.len().max(INITIAL_CHUNK_SIZE);
        let before = buffer.len();
        (&mut reader)
            .take(chunk_size as u64)
            .read_to_end(buffer)
            .map_err(|error| Error::from_io(error, buffer.len()))?;

        if buffer.len() == before {
            return from_slice(buffer);
        }

        match from_slice(buffer) {
            Ok(value) => {
                let mut trailing = [0];
                let trailing_len = loop {
                    match reader.read(&mut trailing) {
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => return Err(Error::from_io(error, buffer.len())),
                        Ok(length) => break length,
                    }
                };
                return if trailing_len == 0 {
                    Ok(value)
                } else {
                    Err(Error::new(ErrorKind::TrailingData, buffer.len()))
                };
            }
            Err(error) if error.kind() == ErrorKind::Eof => {}
            Err(error) => return Err(error),
        }
    })();

    buffer.clear();

    result
}

/// A stateful decoder for consecutive owned CBOR items from a reader.
///
/// The decoder retains only unread bytes between items. A single item is
/// buffered until complete because refillable storage cannot safely back
/// borrowed Serde output.
#[cfg(all(feature = "serde", feature = "std"))]
pub struct ReaderDeserializer<R> {
    reader: R,
    buffer: alloc::vec::Vec<u8>,
    start: usize,
    offset: usize,
    eof: bool,
    options: DecodeOptions,
}

#[cfg(all(feature = "serde", feature = "std"))]
impl<R: std::io::Read> ReaderDeserializer<R> {
    /// Creates a reader decoder with default decoding options.
    pub fn new(reader: R) -> Self {
        Self::with_options(reader, DecodeOptions::default())
    }

    /// Creates a reader decoder with explicit decoding options.
    pub fn with_options(reader: R, options: DecodeOptions) -> Self {
        Self {
            reader,
            buffer: alloc::vec::Vec::new(),
            start: 0,
            offset: 0,
            eof: false,
            options,
        }
    }

    /// Returns the absolute byte offset of the next unread item.
    pub const fn byte_offset(&self) -> usize {
        self.offset
    }

    /// Returns the wrapped reader. Already-buffered unread bytes are discarded.
    pub fn into_inner(self) -> R {
        self.reader
    }

    /// Decodes the next item, or returns an empty option at a clean end of stream.
    pub fn deserialize_next<T: serde::de::DeserializeOwned>(&mut self) -> Result<Option<T>> {
        loop {
            if self.start < self.buffer.len() {
                let mut deserializer =
                    Deserializer::from_slice_with_options(&self.buffer[self.start..], self.options);
                match T::deserialize(&mut deserializer) {
                    Ok(value) => {
                        let consumed = deserializer.byte_offset();
                        self.start += consumed;
                        self.offset += consumed;
                        if self.start == self.buffer.len() {
                            self.buffer.clear();
                            self.start = 0;
                        }

                        return Ok(Some(value));
                    }
                    Err(error) if error.kind() == ErrorKind::Eof && !self.eof => {}
                    Err(error) => {
                        return Err(Error::new(error.kind(), self.offset + error.offset()));
                    }
                }
            } else if self.eof {
                return Ok(None);
            }

            if self.start != 0 {
                self.buffer.copy_within(self.start.., 0);
                self.buffer.truncate(self.buffer.len() - self.start);
                self.start = 0;
            }

            let before = self.buffer.len();
            self.buffer.resize(before + 8 * 1024, 0);

            let read = loop {
                match self.reader.read(&mut self.buffer[before..]) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        self.buffer.truncate(before);
                        return Err(Error::from_io(error, self.offset + before));
                    }
                    Ok(read) => break read,
                }
            };

            self.buffer.truncate(before + read);
            self.eof = read == 0;
        }
    }
}

#[cfg(all(feature = "serde", feature = "std"))]
/// Serializes `value` as CBOR and writes it to `writer`.
pub fn to_writer<T: serde::Serialize + ?Sized, W: std::io::Write>(
    mut writer: W,
    value: &T,
) -> Result<()> {
    serde_codec::to_writer_impl(&mut writer, value)
}

/// Serializes `value` to `writer`, using Ciborium's argument order.
#[cfg(all(feature = "serde", feature = "std"))]
pub fn into_writer<T: serde::Serialize + ?Sized, W: std::io::Write>(
    value: &T,
    writer: W,
) -> Result<()> {
    to_writer(writer, value)
}

/// Writes CBOR's self-described-CBOR tag (55799) to an output.
pub fn write_self_describe<O: Output>(output: &mut O) -> Result<()> {
    let mut encoder = Encoder::new(output);
    encoder.tag(55_799)
}

/// Validates exactly one CBOR data item.
pub fn validate(input: &[u8]) -> Result<()> {
    let mut decoder = SliceDecoder::new(input);
    decoder.skip()?;
    decoder.finish()
}

/// Validates exactly one deterministically encoded CBOR data item.
pub fn validate_deterministic(input: &[u8]) -> Result<()> {
    let options = DecodeOptions {
        validation: Validation::Deterministic,
        ..DecodeOptions::default()
    };

    let mut decoder = SliceDecoder::with_options(input, options);

    decoder.skip()?;
    decoder.finish()
}

#[cfg(feature = "alloc")]
/// Decodes an owned dynamic CBOR value.
pub fn from_slice_value(input: &[u8]) -> Result<Value> {
    decode::decode_owned_value(input)
}

#[cfg(feature = "alloc")]
/// Encodes a dynamic value into a new buffer.
pub fn to_vec_value(value: &Value) -> Result<alloc::vec::Vec<u8>> {
    let mut out = alloc::vec::Vec::new();

    value.encode(&mut Encoder::new(&mut out))?;

    Ok(out)
}
