use alloc::{string::String, vec, vec::Vec};
use serde::{
    Deserialize, Serialize,
    de::{
        self, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
        Visitor,
    },
    ser,
};

use crate::{
    BorrowedValue, DecodeOptions, Encoder, Error, ErrorKind, Event, Parser, Result, Value,
};

impl de::Error for Error {
    fn custom<T: core::fmt::Display>(_msg: T) -> Self {
        Error::new(ErrorKind::Message, 0)
    }
}
impl ser::Error for Error {
    fn custom<T: core::fmt::Display>(_msg: T) -> Self {
        Error::new(ErrorKind::Message, 0)
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub enum EncodeError {
    Message,
    CollectionLimit,
    DuplicateKey,
    OutputTooSmall,
    Error(Error),
}

impl core::fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl core::error::Error for EncodeError {}

impl ser::Error for EncodeError {
    fn custom<T: core::fmt::Display>(_message: T) -> Self {
        Self::Message
    }
}

impl EncodeError {
    fn into_public(self) -> Error {
        self.into()
    }
}

impl From<EncodeError> for Error {
    fn from(error: EncodeError) -> Self {
        Error::new(
            match error {
                EncodeError::Message => ErrorKind::Message,
                EncodeError::CollectionLimit => ErrorKind::CollectionLimit,
                EncodeError::DuplicateKey => ErrorKind::DuplicateKey,
                EncodeError::OutputTooSmall => ErrorKind::OutputTooSmall,
                EncodeError::Error(error) => return error,
            },
            0,
        )
    }
}

type EncodeResult<T> = core::result::Result<T, EncodeError>;
// Covers common sub-kilobyte records without making one-shot buffers too large.
const INITIAL_OUTPUT_CAPACITY: usize = 512;

/// Deserializes exactly one CBOR item from `input`.
pub fn from_slice<'de, T: Deserialize<'de>>(input: &'de [u8]) -> Result<T> {
    from_slice_with_options(input, DecodeOptions::default())
}
/// Deserializes exactly one CBOR item using explicit decoding options.
pub fn from_slice_with_options<'de, T: Deserialize<'de>>(
    input: &'de [u8],
    options: DecodeOptions,
) -> Result<T> {
    let mut deserializer = Deserializer::from_slice_with_options(input, options);
    let value = T::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}
/// Serializes `value` as CBOR into a newly allocated byte vector.
pub fn to_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(INITIAL_OUTPUT_CAPACITY);
    to_vec_into(value, &mut output)?;
    Ok(output)
}
/// Serializes in packed form, replacing struct field and enum variant names
/// with their zero-based declaration indices.
pub fn to_vec_packed<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(INITIAL_OUTPUT_CAPACITY);
    value
        .serialize(PackedSerializer {
            output: &mut output,
        })
        .map_err(EncodeError::into_public)?;
    Ok(output)
}
/// Encodes into `output`, clearing its contents while retaining its allocation.
///
/// On failure, `output` is empty and can be reused for another encoding.
pub fn to_vec_into<T: Serialize + ?Sized>(value: &T, output: &mut Vec<u8>) -> Result<()> {
    output.clear();
    if let Err(error) = value.serialize(StreamingSerializer { output }) {
        output.clear();
        return Err(error.into_public());
    }
    Ok(())
}
/// Deterministically serializes `value` into a new byte vector.
pub fn to_vec_deterministic<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    to_vec_deterministic_into(value, &mut output)?;
    Ok(output)
}
/// Deterministically encodes into `output`, retaining its allocation for reuse.
///
/// The previous contents are cleared first. On failure, `output` is empty.
pub fn to_vec_deterministic_into<T: Serialize + ?Sized>(
    value: &T,
    output: &mut Vec<u8>,
) -> Result<()> {
    to_vec_deterministic_impl(value, output, None)
}

/// Reusable temporary storage for deterministic map encoding.
///
/// This complements an output buffer passed to
/// [`to_vec_deterministic_into_with_scratch`]. Reusing both buffers avoids
/// reallocating the encoded-key arena and sort metadata for a top-level map.
#[derive(Default)]
pub struct DeterministicScratch {
    arena: Vec<u8>,
    entries: Vec<EncodedEntry>,
}

impl DeterministicScratch {
    /// Creates empty reusable deterministic-encoding scratch storage.
    pub const fn new() -> Self {
        Self {
            arena: Vec::new(),
            entries: Vec::new(),
        }
    }
}

/// Deterministically encodes into `output`, reusing map sorting storage.
///
/// The output and scratch allocations are retained after a successful call.
/// On failure, `output` is empty.
pub fn to_vec_deterministic_into_with_scratch<T: Serialize + ?Sized>(
    value: &T,
    output: &mut Vec<u8>,
    scratch: &mut DeterministicScratch,
) -> Result<()> {
    to_vec_deterministic_impl(value, output, Some(scratch))
}

fn to_vec_deterministic_impl<T: Serialize + ?Sized>(
    value: &T,
    output: &mut Vec<u8>,
    scratch: Option<&mut DeterministicScratch>,
) -> Result<()> {
    output.clear();
    if let Err(error) = value.serialize(DeterministicSerializer { output, scratch }) {
        output.clear();
        return Err(error.into_public());
    }
    Ok(())
}
/// Serializes `value` into `output`, returning the initialized prefix.
///
/// Returns [`ErrorKind::OutputTooSmall`] if the slice lacks capacity.
pub fn to_slice<'a, T: Serialize + ?Sized>(
    value: &T,
    output: &'a mut [u8],
) -> Result<&'a mut [u8]> {
    let written = {
        let mut sink = SliceSink {
            output,
            position: 0,
        };
        value
            .serialize(StreamingSerializer { output: &mut sink })
            .map_err(EncodeError::into_public)?;
        sink.position
    };
    Ok(&mut output[..written])
}
/// Computes the number of bytes produced by ordinary CBOR serialization.
pub fn serialized_size<T: Serialize + ?Sized>(value: &T) -> Result<usize> {
    let mut sink = CountSink { position: 0 };
    value
        .serialize(StreamingSerializer { output: &mut sink })
        .map_err(EncodeError::into_public)?;
    Ok(sink.position)
}

#[cfg(feature = "std")]
pub(crate) fn to_writer_impl<T: Serialize + ?Sized, W: std::io::Write>(
    writer: W,
    value: &T,
) -> Result<()> {
    let mut serializer = Serializer::new(crate::IoWrite::new(writer));
    value
        .serialize(&mut serializer)
        .map_err(EncodeError::into_public)
}

/// A reusable choice between ordinary and deterministic vector encoding.
pub struct EncodeConfig {
    deterministic: bool,
    packed: bool,
}
impl EncodeConfig {
    /// Creates a serializer using ordinary CBOR encoding.
    pub const fn new() -> Self {
        Self {
            deterministic: false,
            packed: false,
        }
    }
    /// Creates a serializer using deterministic CBOR encoding.
    pub const fn deterministic() -> Self {
        Self {
            deterministic: true,
            packed: false,
        }
    }
    /// Creates a serializer using packed field and variant indices.
    pub const fn packed() -> Self {
        Self {
            deterministic: false,
            packed: true,
        }
    }
    /// Serializes `value` into a newly allocated byte vector.
    pub fn serialize<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<u8>> {
        if self.packed {
            to_vec_packed(value)
        } else if self.deterministic {
            to_vec_deterministic(value)
        } else {
            to_vec(value)
        }
    }
    /// Encodes into `output`, retaining its allocation for subsequent calls.
    pub fn serialize_into<T: Serialize + ?Sized>(
        &self,
        value: &T,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        if self.packed {
            output.clear();
            if let Err(error) = value.serialize(PackedSerializer { output }) {
                output.clear();
                return Err(error.into_public());
            }
            Ok(())
        } else if self.deterministic {
            to_vec_deterministic_into(value, output)
        } else {
            to_vec_into(value, output)
        }
    }
}
impl Default for EncodeConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[doc(hidden)]
pub struct OutputSink<W>(W);

impl<W: crate::Output> EncodeSink for OutputSink<W> {
    fn push(&mut self, byte: u8) -> EncodeResult<()> {
        self.0
            .write_all(core::slice::from_ref(&byte))
            .map_err(|error| match error.kind() {
                ErrorKind::OutputTooSmall => EncodeError::OutputTooSmall,
                _ => EncodeError::Error(error),
            })
    }

    fn write(&mut self, bytes: &[u8]) -> EncodeResult<()> {
        self.0.write_all(bytes).map_err(|error| match error.kind() {
            ErrorKind::OutputTooSmall => EncodeError::OutputTooSmall,
            _ => EncodeError::Error(error),
        })
    }

    fn reserve(&mut self, _: usize) -> EncodeResult<()> {
        Ok(())
    }
}

/// A stateful Serde serializer that writes CBOR to a caller-owned output.
///
/// A serializer can encode consecutive CBOR items. Its output is returned by
/// [`Serializer::into_inner`]. Deterministic encoding remains available through
/// [`to_vec_deterministic`] because canonical map ordering requires buffering.
pub struct Serializer<W> {
    output: OutputSink<W>,
}

impl<W: crate::Output> Serializer<W> {
    /// Creates a serializer writing ordinary CBOR encoding to `output`.
    pub fn new(output: W) -> Self {
        Self {
            output: OutputSink(output),
        }
    }

    /// Returns the output, preserving any state it accumulated.
    pub fn into_inner(self) -> W {
        self.output.0
    }
}

#[doc(hidden)]
pub trait EncodeSink {
    fn push(&mut self, byte: u8) -> EncodeResult<()>;
    fn write(&mut self, bytes: &[u8]) -> EncodeResult<()>;
    fn reserve(&mut self, additional: usize) -> EncodeResult<()>;

    fn write_signed(&mut self, value: i64) -> EncodeResult<()> {
        let (major, argument) = if value >= 0 {
            (0, value as u64)
        } else {
            (1, !value as u64)
        };
        write_head(self, major, argument)
    }

    fn write_unsigned(&mut self, value: u64) -> EncodeResult<()> {
        write_head(self, 0, value)
    }
}

impl EncodeSink for Vec<u8> {
    #[inline(always)]
    fn push(&mut self, byte: u8) -> EncodeResult<()> {
        Vec::push(self, byte);
        Ok(())
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) -> EncodeResult<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }

    fn reserve(&mut self, additional: usize) -> EncodeResult<()> {
        self.try_reserve(additional)
            .map_err(|_| EncodeError::CollectionLimit)
    }

    #[inline(always)]
    fn write_signed(&mut self, value: i64) -> EncodeResult<()> {
        vec_write_signed(self, value);
        Ok(())
    }

    #[inline(always)]
    fn write_unsigned(&mut self, argument: u64) -> EncodeResult<()> {
        match argument {
            0..=23 => self.push(argument as u8),
            24..=255 => self.extend_from_slice(&[0x18, argument as u8]),
            256..=65_535 => {
                let bytes = (argument as u16).to_be_bytes();
                self.extend_from_slice(&[0x19, bytes[0], bytes[1]]);
            }
            65_536..=4_294_967_295 => {
                let bytes = (argument as u32).to_be_bytes();
                self.extend_from_slice(&[0x1a, bytes[0], bytes[1], bytes[2], bytes[3]]);
            }
            _ => {
                let bytes = argument.to_be_bytes();
                self.extend_from_slice(&[
                    0x1b, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                    bytes[7],
                ]);
            }
        }
        Ok(())
    }
}

#[inline(always)]
fn vec_write_signed(output: &mut Vec<u8>, value: i64) {
    let (major, argument) = if value >= 0 {
        (0, value as u64)
    } else {
        (1, !value as u64)
    };
    let major = major << 5;
    match argument {
        0..=23 => output.push(major | argument as u8),
        24..=255 => output.extend_from_slice(&[major | 24, argument as u8]),
        _ => vec_write_signed_wide(output, major, argument),
    }
}

#[inline(always)]
fn vec_write_signed_wide(output: &mut Vec<u8>, major: u8, argument: u64) {
    match argument {
        0..=255 => unreachable!(),
        256..=65_535 => {
            let bytes = (argument as u16).to_be_bytes();
            output.extend_from_slice(&[major | 25, bytes[0], bytes[1]]);
        }
        65_536..=4_294_967_295 => {
            let bytes = (argument as u32).to_be_bytes();
            output.extend_from_slice(&[major | 26, bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        _ => {
            let bytes = argument.to_be_bytes();
            output.extend_from_slice(&[
                major | 27,
                bytes[0],
                bytes[1],
                bytes[2],
                bytes[3],
                bytes[4],
                bytes[5],
                bytes[6],
                bytes[7],
            ]);
        }
    }
}

struct SliceSink<'a> {
    output: &'a mut [u8],
    position: usize,
}

impl EncodeSink for SliceSink<'_> {
    #[inline]
    fn push(&mut self, byte: u8) -> EncodeResult<()> {
        let target = self
            .output
            .get_mut(self.position)
            .ok_or(EncodeError::OutputTooSmall)?;
        *target = byte;
        self.position += 1;
        Ok(())
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) -> EncodeResult<()> {
        let end = self
            .position
            .checked_add(bytes.len())
            .ok_or(EncodeError::OutputTooSmall)?;
        let target = self
            .output
            .get_mut(self.position..end)
            .ok_or(EncodeError::OutputTooSmall)?;
        target.copy_from_slice(bytes);
        self.position = end;
        Ok(())
    }

    fn reserve(&mut self, _: usize) -> EncodeResult<()> {
        Ok(())
    }
}

struct CountSink {
    position: usize,
}

impl EncodeSink for CountSink {
    #[inline(always)]
    fn push(&mut self, _: u8) -> EncodeResult<()> {
        self.position = self
            .position
            .checked_add(1)
            .ok_or(EncodeError::CollectionLimit)?;
        Ok(())
    }

    #[inline(always)]
    fn write(&mut self, bytes: &[u8]) -> EncodeResult<()> {
        self.position = self
            .position
            .checked_add(bytes.len())
            .ok_or(EncodeError::CollectionLimit)?;
        Ok(())
    }

    fn reserve(&mut self, _: usize) -> EncodeResult<()> {
        Ok(())
    }
}

struct StreamingSerializer<'a, O: ?Sized> {
    output: &'a mut O,
}

struct RawCborSerializer<'a, O: ?Sized> {
    output: &'a mut O,
}

impl<'a, O: EncodeSink + ?Sized> ser::Serializer for RawCborSerializer<'a, O> {
    type Ok = ();
    type Error = EncodeError;
    type SerializeSeq = ser::Impossible<(), EncodeError>;
    type SerializeTuple = ser::Impossible<(), EncodeError>;
    type SerializeTupleStruct = ser::Impossible<(), EncodeError>;
    type SerializeTupleVariant = ser::Impossible<(), EncodeError>;
    type SerializeMap = ser::Impossible<(), EncodeError>;
    type SerializeStruct = ser::Impossible<(), EncodeError>;
    type SerializeStructVariant = ser::Impossible<(), EncodeError>;
    fn serialize_bytes(self, value: &[u8]) -> EncodeResult<()> {
        self.output.write(value)
    }
    fn serialize_bool(self, _: bool) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_i8(self, _: i8) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_i16(self, _: i16) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_i32(self, _: i32) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_i64(self, _: i64) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_i128(self, _: i128) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_u8(self, _: u8) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_u16(self, _: u16) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_u32(self, _: u32) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_u64(self, _: u64) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_u128(self, _: u128) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_f32(self, _: f32) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_f64(self, _: f64) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_char(self, _: char) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_str(self, _: &str) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_none(self) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_unit(self) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_unit_struct(self, _: &'static str) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> EncodeResult<()> {
        Err(EncodeError::Message)
    }
    fn serialize_seq(self, _: Option<usize>) -> EncodeResult<Self::SerializeSeq> {
        Err(EncodeError::Message)
    }
    fn serialize_tuple(self, _: usize) -> EncodeResult<Self::SerializeTuple> {
        Err(EncodeError::Message)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> EncodeResult<Self::SerializeTupleStruct> {
        Err(EncodeError::Message)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> EncodeResult<Self::SerializeTupleVariant> {
        Err(EncodeError::Message)
    }
    fn serialize_map(self, _: Option<usize>) -> EncodeResult<Self::SerializeMap> {
        Err(EncodeError::Message)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> EncodeResult<Self::SerializeStruct> {
        Err(EncodeError::Message)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> EncodeResult<Self::SerializeStructVariant> {
        Err(EncodeError::Message)
    }
}

fn push_head(output: &mut Vec<u8>, major: u8, value: u64) {
    let major = major << 5;
    match value {
        0..=23 => output.push(major | value as u8),
        24..=255 => output.extend_from_slice(&[major | 24, value as u8]),
        256..=65_535 => {
            output.push(major | 25);
            output.extend_from_slice(&(value as u16).to_be_bytes());
        }
        65_536..=4_294_967_295 => {
            output.push(major | 26);
            output.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            output.push(major | 27);
            output.extend_from_slice(&value.to_be_bytes());
        }
    }
}

fn push_text(output: &mut Vec<u8>, value: &str) {
    push_head(output, 3, value.len() as u64);
    output.extend_from_slice(value.as_bytes());
}

fn write_head<O: EncodeSink + ?Sized>(output: &mut O, major: u8, value: u64) -> EncodeResult<()> {
    let major = major << 5;
    match value {
        0..=23 => output.push(major | value as u8),
        24..=255 => output.write(&[major | 24, value as u8]),
        256..=65_535 => {
            output.push(major | 25)?;
            output.write(&(value as u16).to_be_bytes())
        }
        65_536..=4_294_967_295 => {
            output.push(major | 26)?;
            output.write(&(value as u32).to_be_bytes())
        }
        _ => {
            output.push(major | 27)?;
            output.write(&value.to_be_bytes())
        }
    }
}

#[inline(always)]
fn write_signed<O: EncodeSink + ?Sized>(output: &mut O, value: i64) -> EncodeResult<()> {
    output.write_signed(value)
}

#[inline(always)]
fn write_unsigned<O: EncodeSink + ?Sized>(output: &mut O, value: u64) -> EncodeResult<()> {
    output.write_unsigned(value)
}

fn write_bytes<O: EncodeSink + ?Sized>(output: &mut O, value: &[u8]) -> EncodeResult<()> {
    write_head(output, 2, value.len() as u64)?;
    output.write(value)
}

fn write_bignum_bytes<O: EncodeSink + ?Sized>(output: &mut O, value: u128) -> EncodeResult<()> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|byte| *byte != 0).unwrap_or(15);
    write_bytes(output, &bytes[first..])
}

fn write_text<O: EncodeSink + ?Sized>(output: &mut O, value: &str) -> EncodeResult<()> {
    write_head(output, 3, value.len() as u64)?;
    output.write(value.as_bytes())
}

#[inline(always)]
fn write_f32<O: EncodeSink + ?Sized>(output: &mut O, value: f32) -> EncodeResult<()> {
    let bytes = value.to_bits().to_be_bytes();
    output.write(&[0xfa, bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[inline(always)]
fn write_f64<O: EncodeSink + ?Sized>(output: &mut O, value: f64) -> EncodeResult<()> {
    let bytes = value.to_bits().to_be_bytes();
    output.write(&[
        0xfb, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

#[inline(always)]
fn write_float_preferred<O: EncodeSink + ?Sized>(output: &mut O, value: f64) -> EncodeResult<()> {
    let bits = value.to_bits();
    const EXPONENT: u64 = 0x7ff0_0000_0000_0000;
    const F32_DISCARDED: u64 = (1u64 << 29) - 1;
    if bits & EXPONENT != EXPONENT && bits & F32_DISCARDED != 0 {
        let bytes = bits.to_be_bytes();
        return output.write(&[
            0xfb, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
    }
    if value.is_nan() {
        return output.write(&[0xf9, 0x7e, 0x00]);
    }
    const HALF_DISCARDED: u64 = (1u64 << 42) - 1;
    if bits & HALF_DISCARDED == 0
        && let Some(half) = exact_half_f64(bits)
    {
        let bytes = half.to_be_bytes();
        return output.write(&[0xf9, bytes[0], bytes[1]]);
    }
    let narrowed = value as f32;
    if narrowed as f64 == value {
        let bytes = narrowed.to_bits().to_be_bytes();
        output.write(&[0xfa, bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        let bytes = bits.to_be_bytes();
        output.write(&[
            0xfb, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }
}

#[inline(always)]
fn exact_half_f64(bits: u64) -> Option<u16> {
    const FRACTION: u64 = (1u64 << 52) - 1;
    let sign = ((bits >> 48) & 0x8000) as u16;
    let exponent = (bits >> 52) & 0x7ff;
    let fraction = bits & FRACTION;
    match exponent {
        2047 if fraction == 0 => Some(sign | 0x7c00),
        1009..=1038 if fraction & ((1u64 << 42) - 1) == 0 => {
            Some(sign | (((exponent - 1008) as u16) << 10) | (fraction >> 42) as u16)
        }
        999..=1008 => {
            let mantissa = fraction | (1u64 << 52);
            let shift = 1051 - exponent;
            if mantissa & ((1u64 << shift) - 1) == 0 {
                Some(sign | (mantissa >> shift) as u16)
            } else {
                None
            }
        }
        0 if fraction == 0 => Some(sign),
        _ => None,
    }
}

impl<'a, O: EncodeSink + ?Sized> ser::Serializer for StreamingSerializer<'a, O> {
    type Ok = ();
    type Error = EncodeError;
    type SerializeSeq = StreamingCompound<'a, O>;
    type SerializeTuple = StreamingCompound<'a, O>;
    type SerializeTupleStruct = StreamingCompound<'a, O>;
    type SerializeTupleVariant = StreamingCompound<'a, O>;
    type SerializeMap = StreamingCompound<'a, O>;
    type SerializeStruct = StreamingCompound<'a, O>;
    type SerializeStructVariant = StreamingCompound<'a, O>;

    fn serialize_bool(self, value: bool) -> EncodeResult<()> {
        self.output.push(if value { 0xf5 } else { 0xf4 })
    }
    fn serialize_i8(self, value: i8) -> EncodeResult<()> {
        write_signed(self.output, value as i64)
    }
    fn serialize_i16(self, value: i16) -> EncodeResult<()> {
        write_signed(self.output, value as i64)
    }
    #[inline(always)]
    fn serialize_i32(self, value: i32) -> EncodeResult<()> {
        write_signed(self.output, value as i64)
    }
    fn serialize_i64(self, value: i64) -> EncodeResult<()> {
        write_signed(self.output, value)
    }
    fn serialize_i128(self, value: i128) -> EncodeResult<()> {
        if value >= 0 {
            if let Ok(value) = u64::try_from(value) {
                return write_head(self.output, 0, value);
            }
            write_head(self.output, 6, 2)?;
            write_bignum_bytes(self.output, value as u128)?;
        } else {
            let argument = (-1i128).checked_sub(value).unwrap() as u128;
            if argument <= u64::MAX as u128 {
                return write_head(self.output, 1, argument as u64);
            }
            write_head(self.output, 6, 3)?;
            write_bignum_bytes(self.output, argument)?;
        }
        Ok(())
    }
    #[inline(always)]
    fn serialize_u8(self, value: u8) -> EncodeResult<()> {
        self.serialize_u64(value as u64)
    }
    #[inline(always)]
    fn serialize_u16(self, value: u16) -> EncodeResult<()> {
        self.serialize_u64(value as u64)
    }
    #[inline(always)]
    fn serialize_u32(self, value: u32) -> EncodeResult<()> {
        self.serialize_u64(value as u64)
    }
    #[inline(always)]
    fn serialize_u64(self, value: u64) -> EncodeResult<()> {
        write_unsigned(self.output, value)
    }
    fn serialize_u128(self, value: u128) -> EncodeResult<()> {
        if let Ok(value) = u64::try_from(value) {
            write_head(self.output, 0, value)?;
        } else {
            write_head(self.output, 6, 2)?;
            write_bignum_bytes(self.output, value)?;
        }
        Ok(())
    }
    #[inline(always)]
    fn serialize_f32(self, value: f32) -> EncodeResult<()> {
        write_f32(self.output, value)
    }
    #[inline(always)]
    fn serialize_f64(self, value: f64) -> EncodeResult<()> {
        write_f64(self.output, value)
    }
    fn serialize_char(self, value: char) -> EncodeResult<()> {
        write_text(self.output, value.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(self, value: &str) -> EncodeResult<()> {
        write_text(self.output, value)
    }
    fn serialize_bytes(self, value: &[u8]) -> EncodeResult<()> {
        write_bytes(self.output, value)
    }
    fn serialize_none(self) -> EncodeResult<()> {
        self.output.push(0xf6)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> EncodeResult<()> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> EncodeResult<()> {
        self.output.push(0xf6)
    }
    fn serialize_unit_struct(self, _: &'static str) -> EncodeResult<()> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> EncodeResult<()> {
        write_text(self.output, variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        if name == crate::value::VALUE_MARKER {
            return value.serialize(RawCborSerializer {
                output: self.output,
            });
        }
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        write_head(self.output, 5, 1)?;
        write_text(self.output, variant)?;
        value.serialize(self)
    }
    fn serialize_seq(self, len: Option<usize>) -> EncodeResult<Self::SerializeSeq> {
        StreamingCompound::new(self.output, CompoundKind::Sequence, len)
    }
    fn serialize_tuple(self, len: usize) -> EncodeResult<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleVariant> {
        write_head(self.output, 5, 1)?;
        write_text(self.output, variant)?;
        StreamingCompound::new(self.output, CompoundKind::Sequence, Some(len))
    }
    fn serialize_map(self, len: Option<usize>) -> EncodeResult<Self::SerializeMap> {
        StreamingCompound::new(self.output, CompoundKind::Map, len)
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> EncodeResult<Self::SerializeStruct> {
        self.serialize_map(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeStructVariant> {
        write_head(self.output, 5, 1)?;
        write_text(self.output, variant)?;
        StreamingCompound::new(self.output, CompoundKind::Map, Some(len))
    }
    fn collect_str<T: core::fmt::Display + ?Sized>(self, value: &T) -> EncodeResult<()> {
        write_text(self.output, &alloc::format!("{value}"))
    }
}

#[derive(Clone, Copy)]
enum CompoundKind {
    Sequence,
    Map,
}

struct PackedSerializer<'a, O: ?Sized> {
    output: &'a mut O,
}

#[doc(hidden)]
pub struct PackedCompound<'a, O: ?Sized> {
    inner: StreamingCompound<'a, O>,
    field: u64,
}

macro_rules! packed_delegate {
    ($($name:ident($ty:ty)),* $(,)?) => {$(
        fn $name(self, value: $ty) -> EncodeResult<()> {
            ser::Serializer::$name(StreamingSerializer { output: self.output }, value)
        }
    )*};
}

impl<'a, O: EncodeSink + ?Sized> ser::Serializer for PackedSerializer<'a, O> {
    type Ok = ();
    type Error = EncodeError;
    type SerializeSeq = StreamingCompound<'a, O>;
    type SerializeTuple = StreamingCompound<'a, O>;
    type SerializeTupleStruct = StreamingCompound<'a, O>;
    type SerializeTupleVariant = StreamingCompound<'a, O>;
    type SerializeMap = StreamingCompound<'a, O>;
    type SerializeStruct = PackedCompound<'a, O>;
    type SerializeStructVariant = PackedCompound<'a, O>;

    packed_delegate! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16), serialize_i32(i32),
        serialize_i64(i64), serialize_i128(i128), serialize_u8(u8), serialize_u16(u16),
        serialize_u32(u32), serialize_u64(u64), serialize_u128(u128), serialize_f32(f32),
        serialize_f64(f64), serialize_char(char)
    }
    fn serialize_str(self, value: &str) -> EncodeResult<()> {
        write_text(self.output, value)
    }
    fn serialize_bytes(self, value: &[u8]) -> EncodeResult<()> {
        write_bytes(self.output, value)
    }
    fn serialize_none(self) -> EncodeResult<()> {
        self.output.push(0xf6)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> EncodeResult<()> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> EncodeResult<()> {
        self.output.push(0xf6)
    }
    fn serialize_unit_struct(self, _: &'static str) -> EncodeResult<()> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _: &'static str,
        index: u32,
        _: &'static str,
    ) -> EncodeResult<()> {
        self.serialize_u32(index)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        if name == crate::value::VALUE_MARKER {
            value.serialize(RawCborSerializer {
                output: self.output,
            })
        } else {
            value.serialize(self)
        }
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        write_head(self.output, 5, 1)?;
        write_text(self.output, variant)?;
        value.serialize(self)
    }
    fn serialize_seq(self, len: Option<usize>) -> EncodeResult<Self::SerializeSeq> {
        StreamingCompound::new_packed(self.output, CompoundKind::Sequence, len)
    }
    fn serialize_tuple(self, len: usize) -> EncodeResult<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleVariant> {
        write_head(self.output, 5, 1)?;
        write_text(self.output, variant)?;
        StreamingCompound::new_packed(self.output, CompoundKind::Sequence, Some(len))
    }
    fn serialize_map(self, len: Option<usize>) -> EncodeResult<Self::SerializeMap> {
        StreamingCompound::new_packed(self.output, CompoundKind::Map, len)
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> EncodeResult<Self::SerializeStruct> {
        Ok(PackedCompound {
            inner: StreamingCompound::new_packed(self.output, CompoundKind::Map, Some(len))?,
            field: 0,
        })
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        index: u32,
        _: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeStructVariant> {
        write_head(self.output, 5, 1)?;
        write_unsigned(self.output, index as u64)?;
        Ok(PackedCompound {
            inner: StreamingCompound::new_packed(self.output, CompoundKind::Map, Some(len))?,
            field: 0,
        })
    }
    fn collect_str<T: core::fmt::Display + ?Sized>(self, value: &T) -> EncodeResult<()> {
        write_text(self.output, &alloc::format!("{value}"))
    }
}

impl<O: EncodeSink + ?Sized> ser::SerializeStruct for PackedCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        let field = self.field;
        self.field += 1;
        self.inner.serialize(&field)?;
        value.serialize(PackedSerializer {
            output: self.inner.target(),
        })
    }
    fn skip_field(&mut self, _: &'static str) -> EncodeResult<()> {
        self.field += 1;
        Ok(())
    }
    fn end(self) -> EncodeResult<()> {
        self.inner.finish()
    }
}

impl<O: EncodeSink + ?Sized> ser::SerializeStructVariant for PackedCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        ser::SerializeStruct::serialize_field(self, key, value)
    }
    fn skip_field(&mut self, key: &'static str) -> EncodeResult<()> {
        ser::SerializeStruct::skip_field(self, key)
    }
    fn end(self) -> EncodeResult<()> {
        self.inner.finish()
    }
}

#[doc(hidden)]
pub struct StreamingCompound<'a, O: ?Sized> {
    output: &'a mut O,
    indefinite: bool,
    packed: bool,
}

impl<'a, W: crate::Output> ser::Serializer for &'a mut Serializer<W> {
    type Ok = ();
    type Error = EncodeError;
    type SerializeSeq = StreamingCompound<'a, OutputSink<W>>;
    type SerializeTuple = StreamingCompound<'a, OutputSink<W>>;
    type SerializeTupleStruct = StreamingCompound<'a, OutputSink<W>>;
    type SerializeTupleVariant = StreamingCompound<'a, OutputSink<W>>;
    type SerializeMap = StreamingCompound<'a, OutputSink<W>>;
    type SerializeStruct = StreamingCompound<'a, OutputSink<W>>;
    type SerializeStructVariant = StreamingCompound<'a, OutputSink<W>>;

    fn serialize_bool(self, v: bool) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_bool(v)
    }
    fn serialize_i8(self, v: i8) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_i8(v)
    }
    fn serialize_i16(self, v: i16) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_i16(v)
    }
    fn serialize_i32(self, v: i32) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_i32(v)
    }
    fn serialize_i64(self, v: i64) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_i64(v)
    }
    fn serialize_i128(self, v: i128) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_i128(v)
    }
    fn serialize_u8(self, v: u8) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_u8(v)
    }
    fn serialize_u16(self, v: u16) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_u16(v)
    }
    fn serialize_u32(self, v: u32) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_u32(v)
    }
    fn serialize_u64(self, v: u64) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_u64(v)
    }
    fn serialize_u128(self, v: u128) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_u128(v)
    }
    fn serialize_f32(self, v: f32) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_f32(v)
    }
    fn serialize_f64(self, v: f64) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_f64(v)
    }
    fn serialize_char(self, v: char) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_char(v)
    }
    fn serialize_str(self, v: &str) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_str(v)
    }
    fn serialize_bytes(self, v: &[u8]) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_bytes(v)
    }
    fn serialize_none(self) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_none()
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_some(v)
    }
    fn serialize_unit(self) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_unit()
    }
    fn serialize_unit_struct(self, n: &'static str) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_unit_struct(n)
    }
    fn serialize_unit_variant(self, n: &'static str, i: u32, v: &'static str) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_unit_variant(n, i, v)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        n: &'static str,
        v: &T,
    ) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_newtype_struct(n, v)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        n: &'static str,
        i: u32,
        variant: &'static str,
        v: &T,
    ) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_newtype_variant(n, i, variant, v)
    }
    fn serialize_seq(self, len: Option<usize>) -> EncodeResult<Self::SerializeSeq> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_seq(len)
    }
    fn serialize_tuple(self, len: usize) -> EncodeResult<Self::SerializeTuple> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_tuple(len)
    }
    fn serialize_tuple_struct(
        self,
        n: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleStruct> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_tuple_struct(n, len)
    }
    fn serialize_tuple_variant(
        self,
        n: &'static str,
        i: u32,
        v: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleVariant> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_tuple_variant(n, i, v, len)
    }
    fn serialize_map(self, len: Option<usize>) -> EncodeResult<Self::SerializeMap> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_map(len)
    }
    fn serialize_struct(self, n: &'static str, len: usize) -> EncodeResult<Self::SerializeStruct> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_struct(n, len)
    }
    fn serialize_struct_variant(
        self,
        n: &'static str,
        i: u32,
        v: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeStructVariant> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .serialize_struct_variant(n, i, v, len)
    }
    fn collect_str<T: core::fmt::Display + ?Sized>(self, v: &T) -> EncodeResult<()> {
        StreamingSerializer {
            output: &mut self.output,
        }
        .collect_str(v)
    }
}

impl<'a, O: EncodeSink + ?Sized> StreamingCompound<'a, O> {
    fn new(output: &'a mut O, kind: CompoundKind, len: Option<usize>) -> EncodeResult<Self> {
        if let Some(len) = len {
            match kind {
                CompoundKind::Sequence => write_head(output, 4, len as u64)?,
                CompoundKind::Map => write_head(output, 5, len as u64)?,
            }
            let minimum_body = match kind {
                CompoundKind::Sequence => len,
                CompoundKind::Map => len.checked_mul(2).ok_or(EncodeError::CollectionLimit)?,
            };
            let reserve = minimum_body
                .checked_mul(2)
                .ok_or(EncodeError::CollectionLimit)?;
            output.reserve(reserve)?;
        } else {
            output.push(match kind {
                CompoundKind::Sequence => 0x9f,
                CompoundKind::Map => 0xbf,
            })?;
        }
        Ok(Self {
            output,
            indefinite: len.is_none(),
            packed: false,
        })
    }

    fn new_packed(output: &'a mut O, kind: CompoundKind, len: Option<usize>) -> EncodeResult<Self> {
        let mut compound = Self::new(output, kind, len)?;
        compound.packed = true;
        Ok(compound)
    }

    fn target(&mut self) -> &mut O {
        self.output
    }

    #[inline(always)]
    fn serialize<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        if self.packed {
            value.serialize(PackedSerializer {
                output: self.target(),
            })
        } else {
            value.serialize(StreamingSerializer {
                output: self.target(),
            })
        }
    }

    fn finish(self) -> EncodeResult<()> {
        if self.indefinite {
            self.output.push(0xff)?;
        }
        Ok(())
    }
}

impl<O: EncodeSink + ?Sized> ser::SerializeSeq for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    #[inline(always)]
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeTuple for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeTupleStruct for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeTupleVariant for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeMap for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> EncodeResult<()> {
        self.serialize(key)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeStruct for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        self.serialize(key)?;
        self.serialize(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl<O: EncodeSink + ?Sized> ser::SerializeStructVariant for StreamingCompound<'_, O> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        ser::SerializeStruct::serialize_field(self, key, value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}

struct DeterministicSerializer<'a> {
    output: &'a mut Vec<u8>,
    scratch: Option<&'a mut DeterministicScratch>,
}

impl<'a> DeterministicSerializer<'a> {
    fn streaming(self) -> StreamingSerializer<'a, Vec<u8>> {
        StreamingSerializer {
            output: self.output,
        }
    }
}

impl<'a> ser::Serializer for DeterministicSerializer<'a> {
    type Ok = ();
    type Error = EncodeError;
    type SerializeSeq = DeterministicCompound<'a>;
    type SerializeTuple = DeterministicCompound<'a>;
    type SerializeTupleStruct = DeterministicCompound<'a>;
    type SerializeTupleVariant = DeterministicCompound<'a>;
    type SerializeMap = DeterministicCompound<'a>;
    type SerializeStruct = DeterministicCompound<'a>;
    type SerializeStructVariant = DeterministicCompound<'a>;

    fn serialize_bool(self, value: bool) -> EncodeResult<()> {
        ser::Serializer::serialize_bool(self.streaming(), value)
    }
    fn serialize_i8(self, value: i8) -> EncodeResult<()> {
        ser::Serializer::serialize_i8(self.streaming(), value)
    }
    fn serialize_i16(self, value: i16) -> EncodeResult<()> {
        ser::Serializer::serialize_i16(self.streaming(), value)
    }
    fn serialize_i32(self, value: i32) -> EncodeResult<()> {
        ser::Serializer::serialize_i32(self.streaming(), value)
    }
    fn serialize_i64(self, value: i64) -> EncodeResult<()> {
        ser::Serializer::serialize_i64(self.streaming(), value)
    }
    fn serialize_i128(self, value: i128) -> EncodeResult<()> {
        ser::Serializer::serialize_i128(self.streaming(), value)
    }
    fn serialize_u8(self, value: u8) -> EncodeResult<()> {
        ser::Serializer::serialize_u8(self.streaming(), value)
    }
    fn serialize_u16(self, value: u16) -> EncodeResult<()> {
        ser::Serializer::serialize_u16(self.streaming(), value)
    }
    fn serialize_u32(self, value: u32) -> EncodeResult<()> {
        ser::Serializer::serialize_u32(self.streaming(), value)
    }
    fn serialize_u64(self, value: u64) -> EncodeResult<()> {
        ser::Serializer::serialize_u64(self.streaming(), value)
    }
    fn serialize_u128(self, value: u128) -> EncodeResult<()> {
        ser::Serializer::serialize_u128(self.streaming(), value)
    }
    fn serialize_f32(self, value: f32) -> EncodeResult<()> {
        write_float_preferred(self.output, value as f64)
    }
    fn serialize_f64(self, value: f64) -> EncodeResult<()> {
        write_float_preferred(self.output, value)
    }
    fn serialize_char(self, value: char) -> EncodeResult<()> {
        ser::Serializer::serialize_char(self.streaming(), value)
    }
    fn serialize_str(self, value: &str) -> EncodeResult<()> {
        ser::Serializer::serialize_str(self.streaming(), value)
    }
    fn serialize_bytes(self, value: &[u8]) -> EncodeResult<()> {
        ser::Serializer::serialize_bytes(self.streaming(), value)
    }
    fn serialize_none(self) -> EncodeResult<()> {
        ser::Serializer::serialize_none(self.streaming())
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> EncodeResult<()> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> EncodeResult<()> {
        ser::Serializer::serialize_unit(self.streaming())
    }
    fn serialize_unit_struct(self, name: &'static str) -> EncodeResult<()> {
        ser::Serializer::serialize_unit_struct(self.streaming(), name)
    }
    fn serialize_unit_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> EncodeResult<()> {
        ser::Serializer::serialize_unit_variant(self.streaming(), name, index, variant)
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        if name == crate::value::VALUE_MARKER {
            return value.serialize(RawCborSerializer {
                output: self.output,
            });
        }
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        push_head(self.output, 5, 1);
        push_text(self.output, variant);
        value.serialize(self)
    }
    fn serialize_seq(self, len: Option<usize>) -> EncodeResult<Self::SerializeSeq> {
        DeterministicCompound::sequence(self.output, len)
    }
    fn serialize_tuple(self, len: usize) -> EncodeResult<Self::SerializeTuple> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleStruct> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeTupleVariant> {
        push_head(self.output, 5, 1);
        push_text(self.output, variant);
        DeterministicCompound::sequence(self.output, Some(len))
    }
    fn serialize_map(self, len: Option<usize>) -> EncodeResult<Self::SerializeMap> {
        DeterministicCompound::map(self.output, len, self.scratch)
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> EncodeResult<Self::SerializeStruct> {
        DeterministicCompound::map(self.output, Some(len), self.scratch)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        len: usize,
    ) -> EncodeResult<Self::SerializeStructVariant> {
        push_head(self.output, 5, 1);
        push_text(self.output, variant);
        DeterministicCompound::map(self.output, Some(len), self.scratch)
    }
    fn collect_str<T: core::fmt::Display + ?Sized>(self, value: &T) -> EncodeResult<()> {
        ser::Serializer::collect_str(self.streaming(), value)
    }
}

enum DeterministicState<'a> {
    DirectSequence,
    BufferedSequence {
        body: Vec<u8>,
        count: usize,
    },
    Map {
        arena: Vec<u8>,
        entries: Vec<EncodedEntry>,
        pending_key: Option<(usize, usize)>,
        scratch: Option<&'a mut DeterministicScratch>,
    },
}

#[derive(Clone, Copy)]
struct EncodedEntry {
    key_start: usize,
    key_end: usize,
    value_start: usize,
    value_end: usize,
}

struct DeterministicCompound<'a> {
    output: &'a mut Vec<u8>,
    state: DeterministicState<'a>,
}

impl<'a> DeterministicCompound<'a> {
    fn sequence(output: &'a mut Vec<u8>, len: Option<usize>) -> EncodeResult<Self> {
        let state = if let Some(len) = len {
            push_head(output, 4, len as u64);
            output
                .try_reserve(len.checked_mul(2).ok_or(EncodeError::CollectionLimit)?)
                .map_err(|_| EncodeError::CollectionLimit)?;
            DeterministicState::DirectSequence
        } else {
            DeterministicState::BufferedSequence {
                body: Vec::new(),
                count: 0,
            }
        };
        Ok(Self { output, state })
    }

    fn map(
        output: &'a mut Vec<u8>,
        len: Option<usize>,
        mut scratch: Option<&'a mut DeterministicScratch>,
    ) -> EncodeResult<Self> {
        let (mut arena, mut entries) = if let Some(scratch) = scratch.as_deref_mut() {
            (
                core::mem::take(&mut scratch.arena),
                core::mem::take(&mut scratch.entries),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        arena.clear();
        entries.clear();
        if let Some(len) = len {
            entries
                .try_reserve(len)
                .map_err(|_| EncodeError::CollectionLimit)?;
            arena
                .try_reserve(len.checked_mul(16).ok_or(EncodeError::CollectionLimit)?)
                .map_err(|_| EncodeError::CollectionLimit)?;
        }
        Ok(Self {
            output,
            state: DeterministicState::Map {
                arena,
                entries,
                pending_key: None,
                scratch,
            },
        })
    }

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        match &mut self.state {
            DeterministicState::DirectSequence => value.serialize(DeterministicSerializer {
                output: self.output,
                scratch: None,
            }),
            DeterministicState::BufferedSequence { body, count } => {
                value.serialize(DeterministicSerializer {
                    output: body,
                    scratch: None,
                })?;
                *count = count.checked_add(1).ok_or(EncodeError::CollectionLimit)?;
                Ok(())
            }
            DeterministicState::Map { .. } => Err(EncodeError::Message),
        }
    }

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> EncodeResult<()> {
        let DeterministicState::Map {
            arena, pending_key, ..
        } = &mut self.state
        else {
            return Err(EncodeError::Message);
        };
        if pending_key.is_some() {
            return Err(EncodeError::Message);
        }
        let start = arena.len();
        key.serialize(DeterministicSerializer {
            output: arena,
            scratch: None,
        })?;
        *pending_key = Some((start, arena.len()));
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        let DeterministicState::Map {
            arena,
            entries,
            pending_key,
            ..
        } = &mut self.state
        else {
            return Err(EncodeError::Message);
        };
        let (key_start, key_end) = pending_key.take().ok_or(EncodeError::Message)?;
        let value_start = arena.len();
        value.serialize(DeterministicSerializer {
            output: arena,
            scratch: None,
        })?;
        entries.push(EncodedEntry {
            key_start,
            key_end,
            value_start,
            value_end: arena.len(),
        });
        Ok(())
    }

    fn finish(self) -> EncodeResult<()> {
        match self.state {
            DeterministicState::DirectSequence => Ok(()),
            DeterministicState::BufferedSequence { mut body, count } => {
                push_head(self.output, 4, count as u64);
                self.output.append(&mut body);
                Ok(())
            }
            DeterministicState::Map {
                mut arena,
                mut entries,
                pending_key,
                scratch,
            } => {
                let result = if pending_key.is_some() {
                    Err(EncodeError::Message)
                } else {
                    entries.sort_unstable_by(|a, b| {
                        arena[a.key_start..a.key_end].cmp(&arena[b.key_start..b.key_end])
                    });
                    if entries.windows(2).any(|pair| {
                        arena[pair[0].key_start..pair[0].key_end]
                            == arena[pair[1].key_start..pair[1].key_end]
                    }) {
                        Err(EncodeError::DuplicateKey)
                    } else {
                        push_head(self.output, 5, entries.len() as u64);
                        self.output
                            .try_reserve(arena.len())
                            .map_err(|_| EncodeError::CollectionLimit)?;
                        for entry in &entries {
                            self.output
                                .extend_from_slice(&arena[entry.key_start..entry.key_end]);
                            self.output
                                .extend_from_slice(&arena[entry.value_start..entry.value_end]);
                        }
                        Ok(())
                    }
                };
                if let Some(scratch) = scratch {
                    arena.clear();
                    entries.clear();
                    scratch.arena = arena;
                    scratch.entries = entries;
                }
                result
            }
        }
    }
}

impl ser::SerializeSeq for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize_element(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeTuple for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize_element(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeTupleStruct for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize_element(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeTupleVariant for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize_element(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeMap for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> EncodeResult<()> {
        self.serialize_key(key)
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> EncodeResult<()> {
        self.serialize_value(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeStruct for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        self.serialize_key(key)?;
        self.serialize_value(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}
impl ser::SerializeStructVariant for DeterministicCompound<'_> {
    type Ok = ();
    type Error = EncodeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> EncodeResult<()> {
        self.serialize_key(key)?;
        self.serialize_value(value)
    }
    fn end(self) -> EncodeResult<()> {
        self.finish()
    }
}

#[allow(dead_code)]
fn encode_deterministic(value: &Value) -> Result<Vec<u8>> {
    fn go(value: &Value, out: &mut Vec<u8>) -> Result<()> {
        match value {
            Value::Array(xs) => {
                Encoder::new(&mut *out).array(xs.len())?;
                for x in xs {
                    go(x, out)?;
                }
            }
            Value::Map(xs) => {
                let mut entries = Vec::with_capacity(xs.len());
                for (k, v) in xs {
                    entries.push((encode_deterministic(k)?, encode_deterministic(v)?));
                }
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                for pair in entries.windows(2) {
                    if pair[0].0 == pair[1].0 {
                        return Err(Error::new(ErrorKind::DuplicateKey, 0));
                    }
                }
                Encoder::new(&mut *out).map(entries.len())?;
                for (k, v) in entries {
                    out.extend(k);
                    out.extend(v);
                }
            }
            Value::Float(value) => Encoder::new(&mut *out).f64_preferred(*value)?,
            _ => value.encode(&mut Encoder::new(&mut *out))?,
        }
        Ok(())
    }
    let mut out = Vec::new();
    go(value, &mut out)?;
    Ok(out)
}

#[allow(dead_code)]
pub(crate) struct ValueSerializer;
impl ser::Serializer for ValueSerializer {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = Compound;
    type SerializeTuple = Compound;
    type SerializeTupleStruct = Compound;
    type SerializeTupleVariant = Compound;
    type SerializeMap = Compound;
    type SerializeStruct = Compound;
    type SerializeStructVariant = Compound;
    fn serialize_bool(self, v: bool) -> Result<Value> {
        Ok(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Value> {
        self.serialize_i128(v as i128)
    }
    fn serialize_i16(self, v: i16) -> Result<Value> {
        self.serialize_i128(v as i128)
    }
    fn serialize_i32(self, v: i32) -> Result<Value> {
        self.serialize_i128(v as i128)
    }
    fn serialize_i64(self, v: i64) -> Result<Value> {
        self.serialize_i128(v as i128)
    }
    fn serialize_i128(self, v: i128) -> Result<Value> {
        if v >= 0 {
            match u64::try_from(v) {
                Ok(value) => Ok(Value::Unsigned(value)),
                Err(_) => Ok(Value::Tag(
                    2,
                    alloc::boxed::Box::new(Value::Bytes(unsigned_bytes(v as u128))),
                )),
            }
        } else {
            let argument = (-1i128).checked_sub(v).unwrap() as u128;
            match u64::try_from(argument) {
                Ok(_) => Ok(Value::Negative(v)),
                Err(_) => Ok(Value::Tag(
                    3,
                    alloc::boxed::Box::new(Value::Bytes(unsigned_bytes(argument))),
                )),
            }
        }
    }
    fn serialize_u8(self, v: u8) -> Result<Value> {
        Ok(Value::Unsigned(v as u64))
    }
    fn serialize_u16(self, v: u16) -> Result<Value> {
        Ok(Value::Unsigned(v as u64))
    }
    fn serialize_u32(self, v: u32) -> Result<Value> {
        Ok(Value::Unsigned(v as u64))
    }
    fn serialize_u64(self, v: u64) -> Result<Value> {
        Ok(Value::Unsigned(v))
    }
    fn serialize_u128(self, v: u128) -> Result<Value> {
        match u64::try_from(v) {
            Ok(value) => Ok(Value::Unsigned(value)),
            Err(_) => Ok(Value::Tag(
                2,
                alloc::boxed::Box::new(Value::Bytes(unsigned_bytes(v))),
            )),
        }
    }
    fn serialize_f32(self, v: f32) -> Result<Value> {
        Ok(Value::Float(v as f64))
    }
    fn serialize_f64(self, v: f64) -> Result<Value> {
        Ok(Value::Float(v))
    }
    fn serialize_char(self, v: char) -> Result<Value> {
        self.serialize_str(v.encode_utf8(&mut [0; 4]))
    }
    fn serialize_str(self, v: &str) -> Result<Value> {
        Ok(Value::Text(v.into()))
    }
    fn serialize_bytes(self, v: &[u8]) -> Result<Value> {
        Ok(Value::Bytes(v.into()))
    }
    fn serialize_none(self) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, v: &T) -> Result<Value> {
        v.serialize(self)
    }
    fn serialize_unit(self) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value> {
        Ok(Value::Null)
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, v: &'static str) -> Result<Value> {
        Ok(Value::Text(v.into()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        v: &T,
    ) -> Result<Value> {
        if name == crate::value::VALUE_MARKER {
            return v.serialize(RawValueSerializer);
        }
        v.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        n: &'static str,
        v: &T,
    ) -> Result<Value> {
        Ok(Value::Map(vec![(
            Value::Text(n.into()),
            v.serialize(self)?,
        )]))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Compound> {
        Ok(Compound::seq(len))
    }
    fn serialize_tuple(self, len: usize) -> Result<Compound> {
        Ok(Compound::seq(Some(len)))
    }
    fn serialize_tuple_struct(self, _: &'static str, len: usize) -> Result<Compound> {
        Ok(Compound::seq(Some(len)))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        n: &'static str,
        len: usize,
    ) -> Result<Compound> {
        Ok(Compound::variant(n, len))
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Compound> {
        Ok(Compound::map(len))
    }
    fn serialize_struct(self, _: &'static str, len: usize) -> Result<Compound> {
        Ok(Compound::map(Some(len)))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        n: &'static str,
        len: usize,
    ) -> Result<Compound> {
        Ok(Compound::variant_map(n, len))
    }
    fn collect_str<T: core::fmt::Display + ?Sized>(self, value: &T) -> Result<Value> {
        Ok(Value::Text(alloc::format!("{value}")))
    }
}

#[allow(dead_code)]
fn unsigned_bytes(value: u128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let first = bytes.iter().position(|byte| *byte != 0).unwrap_or(15);
    bytes[first..].to_vec()
}
#[allow(dead_code)]
pub(crate) struct Compound {
    values: Vec<Value>,
    entries: Vec<(Value, Value)>,
    key: Option<Value>,
    variant: Option<String>,
    map: bool,
}

struct RawValueSerializer;
impl ser::Serializer for RawValueSerializer {
    type Ok = Value;
    type Error = Error;
    type SerializeSeq = ser::Impossible<Value, Error>;
    type SerializeTuple = ser::Impossible<Value, Error>;
    type SerializeTupleStruct = ser::Impossible<Value, Error>;
    type SerializeTupleVariant = ser::Impossible<Value, Error>;
    type SerializeMap = ser::Impossible<Value, Error>;
    type SerializeStruct = ser::Impossible<Value, Error>;
    type SerializeStructVariant = ser::Impossible<Value, Error>;
    fn serialize_bytes(self, bytes: &[u8]) -> Result<Value> {
        crate::decode::decode_owned_value(bytes)
    }
    fn serialize_bool(self, _: bool) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_i8(self, _: i8) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_i16(self, _: i16) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_i32(self, _: i32) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_i64(self, _: i64) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_i128(self, _: i128) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_u8(self, _: u8) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_u16(self, _: u16) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_u32(self, _: u32) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_u64(self, _: u64) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_u128(self, _: u128) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_f32(self, _: f32) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_f64(self, _: f64) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_char(self, _: char) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_str(self, _: &str) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_none(self) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_unit(self) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: &T,
    ) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<Value> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct> {
        Err(Error::new(ErrorKind::Message, 0))
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant> {
        Err(Error::new(ErrorKind::Message, 0))
    }
}
#[allow(dead_code)]
impl Compound {
    fn seq(n: Option<usize>) -> Self {
        Self {
            values: Vec::with_capacity(n.unwrap_or(0)),
            entries: Vec::new(),
            key: None,
            variant: None,
            map: false,
        }
    }
    fn map(n: Option<usize>) -> Self {
        Self {
            values: Vec::new(),
            entries: Vec::with_capacity(n.unwrap_or(0)),
            key: None,
            variant: None,
            map: true,
        }
    }
    fn variant(n: &str, len: usize) -> Self {
        let mut x = Self::seq(Some(len));
        x.variant = Some(n.into());
        x
    }
    fn variant_map(n: &str, len: usize) -> Self {
        let mut x = Self::map(Some(len));
        x.variant = Some(n.into());
        x
    }
    fn end(self) -> Result<Value> {
        let v = if self.map {
            Value::Map(self.entries)
        } else {
            Value::Array(self.values)
        };
        Ok(if let Some(n) = self.variant {
            Value::Map(vec![(Value::Text(n), v)])
        } else {
            v
        })
    }
}
impl ser::SerializeSeq for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        self.values.push(v.serialize(ValueSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeTuple for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeTupleStruct for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeTupleVariant for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        ser::SerializeSeq::serialize_element(self, v)
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeMap for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, k: &T) -> Result<()> {
        self.key = Some(k.serialize(ValueSerializer)?);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, v: &T) -> Result<()> {
        let k = self.key.take().ok_or(Error::new(ErrorKind::Message, 0))?;
        self.entries.push((k, v.serialize(ValueSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeStruct for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<()> {
        self.entries
            .push((Value::Text(k.into()), v.serialize(ValueSerializer)?));
        Ok(())
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}
impl ser::SerializeStructVariant for Compound {
    type Ok = Value;
    type Error = Error;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, k: &'static str, v: &T) -> Result<()> {
        ser::SerializeStruct::serialize_field(self, k, v)
    }
    fn end(self) -> Result<Value> {
        self.end()
    }
}

/// A stateful Serde deserializer over a borrowed CBOR byte slice.
///
/// Unlike [`from_slice`], this type permits inspecting or decoding the input
/// that follows the first data item. Call [`Deserializer::end`] when exactly
/// one item is expected.
pub struct Deserializer<'de> {
    parser: Parser<'de>,
    depth: usize,
    max_depth: usize,
    max_collection_len: usize,
}

impl<'de> Deserializer<'de> {
    /// Creates a deserializer using the default decoding options.
    pub fn from_slice(input: &'de [u8]) -> Self {
        Self::from_slice_with_options(input, DecodeOptions::default())
    }

    /// Creates a deserializer using explicit decoding options.
    pub fn from_slice_with_options(input: &'de [u8], options: DecodeOptions) -> Self {
        Self {
            parser: Parser::with_options(input, options),
            depth: 0,
            max_depth: options.max_depth,
            max_collection_len: options.max_collection_len,
        }
    }

    /// Returns the byte offset immediately following the consumed input.
    pub fn byte_offset(&self) -> usize {
        self.parser.position()
    }

    /// Returns the exact suffix that has not yet been consumed.
    pub fn remaining(&self) -> &'de [u8] {
        self.parser.remaining()
    }

    /// Succeeds if all input has been consumed.
    pub fn end(&self) -> Result<()> {
        if self.remaining().is_empty() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::TrailingData, self.byte_offset()))
        }
    }

    #[inline]
    fn next(&mut self) -> Result<Event<'de>> {
        self.parser
            .next()
            .ok_or(Error::new(ErrorKind::Eof, self.parser.position()))?
    }

    fn joined_bytes(&mut self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        loop {
            match self.next()? {
                Event::Bytes(chunk) => {
                    if bytes.len().saturating_add(chunk.len()) > self.max_collection_len {
                        return Err(Error::new(ErrorKind::CollectionLimit, self.position()));
                    }
                    bytes.extend_from_slice(chunk);
                }
                Event::Break => return Ok(bytes),
                _ => return Err(Error::new(ErrorKind::UnexpectedType, self.position())),
            }
        }
    }

    fn joined_text(&mut self) -> Result<String> {
        let mut text = String::new();
        loop {
            match self.next()? {
                Event::Text(chunk) => {
                    if text.len().saturating_add(chunk.len()) > self.max_collection_len {
                        return Err(Error::new(ErrorKind::CollectionLimit, self.position()));
                    }
                    text.push_str(chunk);
                }
                Event::Break => return Ok(text),
                _ => return Err(Error::new(ErrorKind::UnexpectedType, self.position())),
            }
        }
    }

    fn position(&self) -> usize {
        self.parser.position()
    }

    #[inline(always)]
    fn next_float(&mut self) -> Result<f64> {
        let initial = self.parser.peek_initial()?;
        if matches!(initial, 0xf9..=0xfb) {
            return self.parser.read_float(initial);
        }
        match self.next()? {
            Event::Float(value) => Ok(value),
            Event::Unsigned(value) => Ok(value as f64),
            Event::Negative(value) => Ok(value as f64),
            _ => Err(Error::new(ErrorKind::UnexpectedType, self.position())),
        }
    }

    #[inline(always)]
    fn next_f32(&mut self) -> Result<f32> {
        if self.parser.peek_initial()? == 0xfa {
            self.parser.read_f32()
        } else {
            Ok(self.next_float()? as f32)
        }
    }

    fn enter_collection(&mut self, len: Option<u64>) -> Result<()> {
        if len.is_some_and(|len| len > self.max_collection_len as u64) {
            return Err(Error::new(ErrorKind::CollectionLimit, self.position()));
        }
        if self.depth >= self.max_depth {
            return Err(Error::new(ErrorKind::DepthLimit, self.position()));
        }
        self.depth += 1;
        Ok(())
    }

    #[inline]
    fn deserialize_sequence<V: Visitor<'de>>(&mut self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 != 4 {
            return de::Deserializer::deserialize_any(&mut *self, visitor);
        }
        let at = self.position();
        let remaining = self.parser.read_collection(initial, 4)?;
        if remaining.is_none() && self.parser.is_deterministic() {
            return Err(Error::new(ErrorKind::NonDeterministic, at));
        }
        self.enter_collection(remaining)?;
        let result = if let Some(remaining) = remaining {
            visitor.visit_seq(DirectSeq {
                deserializer: self,
                remaining,
            })
        } else {
            visitor.visit_seq(IndefiniteSeq {
                deserializer: self,
                seen: 0,
            })
        };
        self.depth -= 1;
        result
    }

    #[inline]
    fn deserialize_mapping<V: Visitor<'de>>(&mut self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 != 5 {
            return de::Deserializer::deserialize_any(&mut *self, visitor);
        }
        let at = self.position();
        let remaining = self.parser.read_collection(initial, 5)?;
        if remaining.is_none() && self.parser.is_deterministic() {
            return Err(Error::new(ErrorKind::NonDeterministic, at));
        }
        self.enter_collection(remaining)?;
        let result = if let Some(remaining) = remaining {
            visitor.visit_map(DirectMap {
                deserializer: self,
                remaining,
                previous_key: None,
            })
        } else {
            visitor.visit_map(IndefiniteMap {
                deserializer: self,
                expecting_value: false,
                seen: 0,
            })
        };
        self.depth -= 1;
        result
    }
}

struct DirectSeq<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    remaining: u64,
}

struct IndefiniteSeq<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    seen: usize,
}

impl<'de> SeqAccess<'de> for DirectSeq<'_, 'de> {
    type Error = Error;

    #[inline(always)]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(&mut *self.deserializer).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        usize::try_from(self.remaining).ok()
    }
}

impl<'de> SeqAccess<'de> for IndefiniteSeq<'_, 'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        if self.deserializer.parser.peek_initial()? == 0xff {
            self.deserializer.parser.consume_one();
            return Ok(None);
        }
        self.seen += 1;
        if self.seen > self.deserializer.max_collection_len {
            return Err(Error::new(
                ErrorKind::CollectionLimit,
                self.deserializer.position(),
            ));
        }
        seed.deserialize(&mut *self.deserializer).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

struct DirectMap<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    remaining: u64,
    previous_key: Option<&'de [u8]>,
}

struct IndefiniteMap<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    expecting_value: bool,
    seen: usize,
}

impl<'de> MapAccess<'de> for DirectMap<'_, 'de> {
    type Error = Error;

    #[inline(always)]
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let start = self.deserializer.position();
        let key = seed.deserialize(&mut *self.deserializer)?;
        if self.deserializer.parser.is_deterministic() {
            let encoded = self
                .deserializer
                .parser
                .raw_range(start, self.deserializer.position());
            if self
                .previous_key
                .is_some_and(|previous| previous >= encoded)
            {
                return Err(Error::new(ErrorKind::NonDeterministic, start));
            }
            self.previous_key = Some(encoded);
        }
        Ok(Some(key))
    }

    #[inline(always)]
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        seed.deserialize(&mut *self.deserializer)
    }

    fn size_hint(&self) -> Option<usize> {
        usize::try_from(self.remaining).ok()
    }
}

impl<'de> MapAccess<'de> for IndefiniteMap<'_, 'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        if self.expecting_value {
            return Err(Error::new(ErrorKind::Message, self.deserializer.position()));
        }
        if self.deserializer.parser.peek_initial()? == 0xff {
            self.deserializer.parser.consume_one();
            return Ok(None);
        }
        self.seen += 1;
        if self.seen > self.deserializer.max_collection_len {
            return Err(Error::new(
                ErrorKind::CollectionLimit,
                self.deserializer.position(),
            ));
        }
        self.expecting_value = true;
        seed.deserialize(&mut *self.deserializer).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        if !self.expecting_value {
            return Err(Error::new(ErrorKind::Message, self.deserializer.position()));
        }
        self.expecting_value = false;
        seed.deserialize(&mut *self.deserializer)
    }

    fn size_hint(&self) -> Option<usize> {
        None
    }
}

impl<'de> de::Deserializer<'de> for &mut Deserializer<'de> {
    type Error = Error;

    #[inline]
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let at = self.position();
        match self.next()? {
            Event::Unsigned(value) => visitor.visit_u64(value),
            Event::Negative(value) => visitor.visit_i128(value),
            Event::Bytes(value) => visitor.visit_borrowed_bytes(value),
            Event::Text(value) => visitor.visit_borrowed_str(value),
            Event::IndefiniteBytes => {
                if self.parser.is_deterministic() {
                    return Err(Error::new(ErrorKind::NonDeterministic, at));
                }
                visitor.visit_byte_buf(self.joined_bytes()?)
            }
            Event::IndefiniteText => {
                if self.parser.is_deterministic() {
                    return Err(Error::new(ErrorKind::NonDeterministic, at));
                }
                visitor.visit_string(self.joined_text()?)
            }
            Event::Array(Some(remaining)) => {
                self.enter_collection(Some(remaining))?;
                let result = visitor.visit_seq(DirectSeq {
                    deserializer: self,
                    remaining,
                });
                self.depth -= 1;
                result
            }
            Event::Array(None) => {
                if self.parser.is_deterministic() {
                    return Err(Error::new(ErrorKind::NonDeterministic, at));
                }
                self.enter_collection(None)?;
                let result = visitor.visit_seq(IndefiniteSeq {
                    deserializer: self,
                    seen: 0,
                });
                self.depth -= 1;
                result
            }
            Event::Map(Some(remaining)) => {
                self.enter_collection(Some(remaining))?;
                let result = visitor.visit_map(DirectMap {
                    deserializer: self,
                    remaining,
                    previous_key: None,
                });
                self.depth -= 1;
                result
            }
            Event::Map(None) => {
                if self.parser.is_deterministic() {
                    return Err(Error::new(ErrorKind::NonDeterministic, at));
                }
                self.enter_collection(None)?;
                let result = visitor.visit_map(IndefiniteMap {
                    deserializer: self,
                    expecting_value: false,
                    seen: 0,
                });
                self.depth -= 1;
                result
            }
            Event::Tag(_) => self.deserialize_any(visitor),
            Event::Simple(value) => visitor.visit_u8(value),
            Event::Bool(value) => visitor.visit_bool(value),
            Event::Null | Event::Undefined => visitor.visit_unit(),
            Event::Float(value) => visitor.visit_f64(value),
            Event::Break => Err(Error::new(ErrorKind::UnexpectedBreak, self.position())),
        }
    }

    #[inline(always)]
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if matches!(initial, 0xf6 | 0xf7) {
            self.parser.consume_one();
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    #[inline(always)]
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if matches!(initial, 0xf4 | 0xf5) {
            return visitor.visit_bool(self.parser.read_bool(initial)?);
        }
        self.deserialize_any(visitor)
    }

    #[inline(always)]
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 <= 1 && !self.parser.is_deterministic() {
            return visitor.visit_i8(self.parser.read_i8(initial)?);
        }
        visitor.visit_i8(
            i8::try_from(direct_integer(self)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?,
        )
    }

    #[inline(always)]
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial <= 0x17 {
            self.parser.consume_one();
            return visitor.visit_u8(initial);
        }
        if initial == 0x18 {
            return visitor.visit_u8(self.parser.read_u8_one_byte()?);
        }
        if initial >> 5 == 0 {
            let value = if self.parser.is_deterministic() {
                u8::try_from(self.parser.read_unsigned(initial)?)
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?
            } else {
                self.parser.read_u8(initial)?
            };
            return visitor.visit_u8(value);
        }
        self.deserialize_any(visitor)
    }

    #[inline(always)]
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        let value = if initial >> 5 <= 1 && !self.parser.is_deterministic() {
            self.parser.read_i16(initial)?
        } else {
            i16::try_from(direct_integer(self)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?
        };
        visitor.visit_i16(value)
    }

    #[inline(always)]
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        let value = if initial >> 5 <= 1 && !self.parser.is_deterministic() {
            self.parser.read_i32(initial)?
        } else {
            i32::try_from(direct_integer(self)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?
        };
        visitor.visit_i32(value)
    }

    #[inline(always)]
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        let value = if initial >> 5 <= 1 && !self.parser.is_deterministic() {
            self.parser.read_i64(initial)?
        } else {
            i64::try_from(direct_integer(self)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?
        };
        visitor.visit_i64(value)
    }

    #[inline(always)]
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 0 {
            let value = u16::try_from(self.parser.read_unsigned(initial)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?;
            visitor.visit_u16(value)
        } else {
            self.deserialize_any(visitor)
        }
    }

    #[inline(always)]
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 0 {
            let value = u32::try_from(self.parser.read_unsigned(initial)?)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?;
            visitor.visit_u32(value)
        } else {
            self.deserialize_any(visitor)
        }
    }

    #[inline(always)]
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 0 {
            visitor.visit_u64(self.parser.read_unsigned(initial)?)
        } else {
            self.deserialize_any(visitor)
        }
    }

    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i128(direct_integer(self)?)
    }

    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u128(direct_unsigned(self)?)
    }

    #[inline]
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 3 && initial & 31 != 31 && !self.parser.is_deterministic() {
            return visitor.visit_borrowed_str(self.parser.read_text(initial)?);
        }
        self.deserialize_any(visitor)
    }

    #[inline(always)]
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 3 && initial & 31 != 31 && !self.parser.is_deterministic() {
            return visitor.visit_borrowed_str(self.parser.read_text(initial)?);
        }
        self.deserialize_any(visitor)
    }

    #[inline]
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    #[inline]
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let initial = self.parser.peek_initial()?;
        if initial >> 5 == 2 && initial & 31 != 31 && !self.parser.is_deterministic() {
            return visitor.visit_borrowed_bytes(self.parser.read_bytes(initial)?);
        }
        self.deserialize_any(visitor)
    }

    #[inline]
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_bytes(visitor)
    }

    #[inline]
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if matches!(self.parser.peek_initial()?, 0xf6 | 0xf7) {
            self.parser.consume_one();
            visitor.visit_unit()
        } else {
            self.deserialize_any(visitor)
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_unit(visitor)
    }

    #[inline(always)]
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f32(self.next_f32()?)
    }

    #[inline(always)]
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f64(self.next_float()?)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &str,
        _: &[&str],
        visitor: V,
    ) -> Result<V::Value> {
        match self.next()? {
            Event::Text(variant) => visitor.visit_enum(variant.into_deserializer()),
            Event::Unsigned(variant) => {
                let variant = u32::try_from(variant)
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?;
                visitor.visit_enum(variant.into_deserializer())
            }
            Event::Map(Some(1)) => match self.next()? {
                Event::Text(variant) => visitor.visit_enum(DirectEnum {
                    deserializer: self,
                    variant,
                    close_indefinite: false,
                }),
                Event::Unsigned(variant) => {
                    let variant = u32::try_from(variant)
                        .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?;
                    visitor.visit_enum(DirectEnumIndex {
                        deserializer: self,
                        variant,
                        close_indefinite: false,
                    })
                }
                _ => Err(Error::new(ErrorKind::UnexpectedType, self.position())),
            },
            Event::Map(None) => {
                if self.parser.is_deterministic() {
                    return Err(Error::new(
                        ErrorKind::NonDeterministic,
                        self.position().saturating_sub(1),
                    ));
                }
                match self.next()? {
                    Event::Text(variant) => visitor.visit_enum(DirectEnum {
                        deserializer: self,
                        variant,
                        close_indefinite: true,
                    }),
                    Event::Unsigned(variant) => {
                        let variant = u32::try_from(variant)
                            .map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.position()))?;
                        visitor.visit_enum(DirectEnumIndex {
                            deserializer: self,
                            variant,
                            close_indefinite: true,
                        })
                    }
                    _ => Err(Error::new(ErrorKind::UnexpectedType, self.position())),
                }
            }
            _ => Err(Error::new(ErrorKind::UnexpectedType, self.position())),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &str,
        visitor: V,
    ) -> Result<V::Value> {
        if name == crate::value::VALUE_MARKER {
            let input = self.parser.remaining();
            self.parser.skip_item()?;
            let consumed = input.len() - self.parser.remaining().len();
            let raw = serde::de::value::BorrowedBytesDeserializer::<Error>::new(&input[..consumed]);
            return visitor.visit_newtype_struct(raw);
        }
        visitor.visit_newtype_struct(self)
    }

    #[inline]
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.parser.skip_item()?;
        visitor.visit_unit()
    }

    #[inline]
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_sequence(visitor)
    }

    #[inline]
    fn deserialize_tuple<V: Visitor<'de>>(self, _length: usize, visitor: V) -> Result<V::Value> {
        self.deserialize_sequence(visitor)
    }

    #[inline]
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _length: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_sequence(visitor)
    }

    #[inline]
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_mapping(visitor)
    }

    #[inline]
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        if self.parser.peek_initial()? >> 5 == 4 {
            self.deserialize_sequence(visitor)
        } else {
            self.deserialize_mapping(visitor)
        }
    }
}

#[inline]
fn direct_integer(deserializer: &mut Deserializer<'_>) -> Result<i128> {
    match deserializer.next()? {
        Event::Unsigned(value) => Ok(value as i128),
        Event::Negative(value) => Ok(value),
        Event::Tag(tag @ (2 | 3)) => {
            let unsigned = direct_biguint(deserializer)?;
            if tag == 2 {
                i128::try_from(unsigned)
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, deserializer.position()))
            } else {
                let value = i128::try_from(unsigned)
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, deserializer.position()))?;
                Ok(-1 - value)
            }
        }
        _ => Err(Error::new(
            ErrorKind::UnexpectedType,
            deserializer.position(),
        )),
    }
}

fn direct_unsigned(deserializer: &mut Deserializer<'_>) -> Result<u128> {
    match deserializer.next()? {
        Event::Unsigned(value) => Ok(value as u128),
        Event::Tag(2) => direct_biguint(deserializer),
        _ => Err(Error::new(
            ErrorKind::UnexpectedType,
            deserializer.position(),
        )),
    }
}

fn direct_biguint(deserializer: &mut Deserializer<'_>) -> Result<u128> {
    let bytes = match deserializer.next()? {
        Event::Bytes(bytes) => bytes,
        _ => {
            return Err(Error::new(
                ErrorKind::UnexpectedType,
                deserializer.position(),
            ));
        }
    };
    if bytes.is_empty() || bytes.len() > 16 {
        return Err(Error::new(
            ErrorKind::IntegerOverflow,
            deserializer.position(),
        ));
    }
    Ok(bytes
        .iter()
        .fold(0u128, |value, byte| (value << 8) | *byte as u128))
}

struct DirectEnum<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    variant: &'de str,
    close_indefinite: bool,
}

struct DirectEnumIndex<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    variant: u32,
    close_indefinite: bool,
}

impl<'a, 'de> EnumAccess<'de> for DirectEnumIndex<'a, 'de> {
    type Error = Error;
    type Variant = DirectVariant<'a, 'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        let variant = seed.deserialize(
            <u32 as IntoDeserializer<'de, Error>>::into_deserializer(self.variant),
        )?;
        Ok((
            variant,
            DirectVariant {
                deserializer: self.deserializer,
                close_indefinite: self.close_indefinite,
            },
        ))
    }
}

impl<'a, 'de> EnumAccess<'de> for DirectEnum<'a, 'de> {
    type Error = Error;
    type Variant = DirectVariant<'a, 'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        let variant = seed.deserialize(
            <&'de str as IntoDeserializer<'de, Error>>::into_deserializer(self.variant),
        )?;
        Ok((
            variant,
            DirectVariant {
                deserializer: self.deserializer,
                close_indefinite: self.close_indefinite,
            },
        ))
    }
}

struct DirectVariant<'a, 'de> {
    deserializer: &'a mut Deserializer<'de>,
    close_indefinite: bool,
}

impl DirectVariant<'_, '_> {
    fn finish(&mut self) -> Result<()> {
        if self.close_indefinite {
            match self.deserializer.next()? {
                Event::Break => {}
                _ => {
                    return Err(Error::new(
                        ErrorKind::UnexpectedType,
                        self.deserializer.position(),
                    ));
                }
            }
        }
        Ok(())
    }
}

impl<'de> VariantAccess<'de> for DirectVariant<'_, 'de> {
    type Error = Error;

    fn unit_variant(mut self) -> Result<()> {
        <()>::deserialize(&mut *self.deserializer)?;
        self.finish()
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(mut self, seed: T) -> Result<T::Value> {
        let value = seed.deserialize(&mut *self.deserializer)?;
        self.finish()?;
        Ok(value)
    }

    fn tuple_variant<V: Visitor<'de>>(mut self, len: usize, visitor: V) -> Result<V::Value> {
        let value = de::Deserializer::deserialize_tuple(&mut *self.deserializer, len, visitor)?;
        self.finish()?;
        Ok(value)
    }

    fn struct_variant<V: Visitor<'de>>(
        mut self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        let value =
            de::Deserializer::deserialize_struct(&mut *self.deserializer, "", fields, visitor)?;
        self.finish()?;
        Ok(value)
    }
}

struct Seq<I>(I);
impl<'de, I: Iterator<Item = BorrowedValue<'de>>> SeqAccess<'de> for Seq<I> {
    type Error = Error;
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, s: T) -> Result<Option<T::Value>> {
        self.0.next().map(|v| s.deserialize(v)).transpose()
    }
}

struct OwnedBytesDeserializer(Vec<u8>);

impl<'de> de::Deserializer<'de> for OwnedBytesDeserializer {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_byte_buf(self.0)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum identifier ignored_any
    }
}

impl<'de> de::Deserializer<'de> for BorrowedValue<'de> {
    type Error = Error;
    fn deserialize_any<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        match self {
            Self::Unsigned(x) => v.visit_u64(x),
            Self::Negative(x) => v.visit_i128(x),
            Self::Bytes(x) => match x {
                alloc::borrow::Cow::Borrowed(x) => v.visit_borrowed_bytes(x),
                alloc::borrow::Cow::Owned(x) => v.visit_byte_buf(x),
            },
            Self::Text(x) => match x {
                alloc::borrow::Cow::Borrowed(x) => v.visit_borrowed_str(x),
                alloc::borrow::Cow::Owned(x) => v.visit_string(x),
            },
            Self::Array(x) => v.visit_seq(Seq(x.into_iter())),
            Self::Map(x) => v.visit_map(Pairs {
                x: x.into_iter(),
                value: None,
            }),
            Self::Bool(x) => v.visit_bool(x),
            Self::Null | Self::Undefined => v.visit_unit(),
            Self::Float(x) => v.visit_f64(x),
            Self::Tag(_, x) => x.deserialize_any(v),
            Self::Simple(x) => v.visit_u8(x),
        }
    }
    fn deserialize_option<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        if matches!(self, Self::Null | Self::Undefined) {
            v.visit_none()
        } else {
            v.visit_some(self)
        }
    }
    fn deserialize_i8<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_i8(
            i8::try_from(as_i128(self)?).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?,
        )
    }
    fn deserialize_i16<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_i16(
            i16::try_from(as_i128(self)?).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?,
        )
    }
    fn deserialize_i32<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_i32(
            i32::try_from(as_i128(self)?).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?,
        )
    }
    fn deserialize_i64<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_i64(
            i64::try_from(as_i128(self)?).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?,
        )
    }
    fn deserialize_i128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_i128(as_i128(self)?)
    }
    fn deserialize_u128<V: Visitor<'de>>(self, v: V) -> Result<V::Value> {
        v.visit_u128(as_u128(self)?)
    }
    fn deserialize_enum<V: Visitor<'de>>(self, _: &str, _: &[&str], v: V) -> Result<V::Value> {
        match self {
            Self::Text(s) => v.visit_enum(s.into_owned().into_deserializer()),
            Self::Map(mut entries) if entries.len() == 1 => {
                let (key, value) = entries.pop().unwrap();
                let Self::Text(variant) = key else {
                    return Err(Error::new(ErrorKind::UnexpectedType, 0));
                };
                v.visit_enum(ValueEnum {
                    variant: variant.into_owned(),
                    value,
                })
            }
            _ => Err(Error::new(ErrorKind::UnexpectedType, 0)),
        }
    }
    fn deserialize_newtype_struct<V: Visitor<'de>>(self, name: &str, v: V) -> Result<V::Value> {
        if name == crate::value::VALUE_MARKER {
            let mut bytes = Vec::new();
            Value::from(self).encode(&mut Encoder::new(&mut bytes))?;
            return v.visit_newtype_struct(OwnedBytesDeserializer(bytes));
        }
        v.visit_newtype_struct(self)
    }
    serde::forward_to_deserialize_any! {bool u8 u16 u32 u64 f32 f64 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any}
}

struct ValueEnum<'de> {
    variant: String,
    value: BorrowedValue<'de>,
}

impl<'de> EnumAccess<'de> for ValueEnum<'de> {
    type Error = Error;
    type Variant = ValueVariant<'de>;
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        let variant = seed.deserialize(
            <String as IntoDeserializer<'de, Error>>::into_deserializer(self.variant),
        )?;
        Ok((variant, ValueVariant(self.value)))
    }
}

struct ValueVariant<'de>(BorrowedValue<'de>);

impl<'de> VariantAccess<'de> for ValueVariant<'de> {
    type Error = Error;
    fn unit_variant(self) -> Result<()> {
        <()>::deserialize(self.0)
    }
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        seed.deserialize(self.0)
    }
    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value> {
        de::Deserializer::deserialize_tuple(self.0, len, visitor)
    }
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        de::Deserializer::deserialize_struct(self.0, "", fields, visitor)
    }
}

fn as_i128(value: BorrowedValue<'_>) -> Result<i128> {
    match value {
        BorrowedValue::Unsigned(value) => Ok(value as i128),
        BorrowedValue::Negative(value) => Ok(value),
        BorrowedValue::Tag(2, value) => i128::try_from(as_biguint(*value)?)
            .map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0)),
        BorrowedValue::Tag(3, value) => {
            let argument = as_biguint(*value)?;
            let argument =
                i128::try_from(argument).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?;
            Ok(-1 - argument)
        }
        _ => Err(Error::new(ErrorKind::UnexpectedType, 0)),
    }
}

fn as_u128(value: BorrowedValue<'_>) -> Result<u128> {
    match value {
        BorrowedValue::Unsigned(value) => Ok(value as u128),
        BorrowedValue::Tag(2, value) => as_biguint(*value),
        _ => Err(Error::new(ErrorKind::UnexpectedType, 0)),
    }
}

fn as_biguint(value: BorrowedValue<'_>) -> Result<u128> {
    let BorrowedValue::Bytes(bytes) = value else {
        return Err(Error::new(ErrorKind::UnexpectedType, 0));
    };
    if bytes.is_empty() || bytes.len() > 16 {
        return Err(Error::new(ErrorKind::IntegerOverflow, 0));
    }
    let mut result = 0u128;
    for byte in bytes.iter() {
        result = (result << 8) | *byte as u128;
    }
    Ok(result)
}
struct Pairs<'de> {
    x: alloc::vec::IntoIter<(BorrowedValue<'de>, BorrowedValue<'de>)>,
    value: Option<BorrowedValue<'de>>,
}
impl<'de> MapAccess<'de> for Pairs<'de> {
    type Error = Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, s: K) -> Result<Option<K::Value>> {
        match self.x.next() {
            Some((k, v)) => {
                self.value = Some(v);
                s.deserialize(k).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, s: V) -> Result<V::Value> {
        s.deserialize(self.value.take().ok_or(Error::new(ErrorKind::Message, 0))?)
    }
}
