use core::str;

use crate::{DecodeOptions, Error, ErrorKind, Result, Validation};

#[cfg(feature = "alloc")]
use crate::value::{BorrowedValue, Value};
#[cfg(feature = "alloc")]
use alloc::{borrow::Cow, boxed::Box, string::String, vec::Vec};

#[inline]
fn validated_str(bytes: &[u8], at: usize) -> Result<&str> {
    str::from_utf8(bytes).map_err(|_| Error::new(ErrorKind::InvalidUtf8, at))
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
/// A token produced while parsing a CBOR item stream.
pub enum Event<'de> {
    /// An unsigned integer.
    Unsigned(u64),
    /// A negative integer represented by its mathematical value.
    Negative(i128),
    /// A definite-length byte string.
    Bytes(&'de [u8]),
    /// A definite-length UTF-8 text string.
    Text(&'de str),
    /// The start of an indefinite-length byte string.
    IndefiniteBytes,
    /// The start of an indefinite-length text string.
    IndefiniteText,
    /// The start of an array; `None` denotes indefinite length.
    Array(Option<u64>),
    /// The start of a map; `None` denotes indefinite length.
    Map(Option<u64>),
    /// A semantic tag followed by its tagged item.
    Tag(u64),
    /// An unassigned simple value.
    Simple(u8),
    /// A Boolean value.
    Bool(bool),
    /// The null value.
    Null,
    /// The undefined value.
    Undefined,
    /// A floating-point value, widened to binary64 when necessary.
    Float(f64),
    /// The break stop code ending an indefinite-length item.
    Break,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// The complete encoded bytes of one CBOR data item.
pub struct RawValue<'de>(&'de [u8]);
impl<'de> RawValue<'de> {
    /// Wraps bytes containing one encoded CBOR item without validating them.
    pub const fn new(bytes: &'de [u8]) -> Self {
        Self(bytes)
    }
    /// Returns the wrapped encoded bytes.
    pub const fn as_bytes(&self) -> &'de [u8] {
        self.0
    }
}

/// A cursor for decoding typed values from a byte slice.
pub struct SliceDecoder<'de> {
    input: &'de [u8],
    pos: usize,
    options: DecodeOptions,
}

impl<'de> SliceDecoder<'de> {
    /// Creates a decoder using [`DecodeOptions::default`].
    pub fn new(input: &'de [u8]) -> Self {
        Self::with_options(input, DecodeOptions::default())
    }
    /// Creates a decoder with explicit validation and resource limits.
    pub fn with_options(input: &'de [u8], options: DecodeOptions) -> Self {
        Self {
            input,
            pos: 0,
            options,
        }
    }
    /// Returns the byte offset of the next item.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Returns the input bytes not yet consumed.
    pub fn remaining(&self) -> &'de [u8] {
        &self.input[self.pos..]
    }
    /// Returns the next initial byte without consuming it.
    pub fn peek(&self) -> Result<u8> {
        self.input
            .get(self.pos)
            .copied()
            .ok_or(Error::new(ErrorKind::Eof, self.pos))
    }
    /// Verifies that no disallowed trailing bytes remain.
    pub fn finish(&self) -> Result<()> {
        if self.options.allow_trailing || self.pos == self.input.len() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::TrailingData, self.pos))
        }
    }
    #[inline]
    fn byte(&mut self) -> Result<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Ok(b)
    }
    #[inline]
    fn take(&mut self, n: usize) -> Result<&'de [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(Error::new(ErrorKind::Eof, self.pos))?;
        let result = self
            .input
            .get(self.pos..end)
            .ok_or(Error::new(ErrorKind::Eof, self.pos))?;
        self.pos = end;
        Ok(result)
    }
    #[inline]
    fn argument(&mut self, ai: u8, head: usize) -> Result<Option<u64>> {
        let (value, width) = match ai {
            0..=23 => (ai as u64, 0),
            24 => (self.byte()? as u64, 1),
            25 => (
                u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64,
                2,
            ),
            26 => (
                u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64,
                4,
            ),
            27 => (u64::from_be_bytes(self.take(8)?.try_into().unwrap()), 8),
            31 => return Ok(None),
            _ => return Err(Error::new(ErrorKind::InvalidAdditionalInfo, head)),
        };
        // For major type 7, additional information 25..=27 selects a
        // floating-point width; the following bytes are payload bits, not an
        // integer argument whose encoded width can be minimized.
        if self.options.validation == Validation::Deterministic && self.input[head] >> 5 != 7 {
            let preferred = match value {
                0..=23 => 0,
                24..=255 => 1,
                256..=65535 => 2,
                65536..=4_294_967_295 => 4,
                _ => 8,
            };
            if width != preferred {
                return Err(Error::new(ErrorKind::NonDeterministic, head));
            }
        }
        Ok(Some(value))
    }
    #[inline]
    fn header(&mut self) -> Result<(u8, Option<u64>, usize)> {
        let offset = self.pos;
        let initial = self.byte()?;
        let major = initial >> 5;
        let arg = self.argument(initial & 31, offset)?;
        Ok((major, arg, offset))
    }
    #[inline(always)]
    /// Decodes an unsigned integer.
    pub fn unsigned(&mut self) -> Result<u64> {
        let (m, n, at) = self.header()?;
        if m == 0 {
            n.ok_or(Error::new(ErrorKind::UnexpectedType, at))
        } else {
            Err(Error::new(ErrorKind::UnexpectedType, at))
        }
    }
    /// Decodes a positive or negative integer.
    pub fn integer(&mut self) -> Result<i128> {
        let (m, n, at) = self.header()?;
        let n = n.ok_or(Error::new(ErrorKind::UnexpectedType, at))?;
        match m {
            0 => Ok(n as i128),
            1 => Ok(-1 - n as i128),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(any(feature = "alloc", feature = "serde"))]
    #[inline(always)]
    fn basic_integer_argument(&mut self, additional: u8, at: usize) -> Result<u64> {
        match additional {
            0..=23 => Ok(additional as u64),
            24 => Ok(self.byte()? as u64),
            25 => Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64),
            26 => Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64),
            27 => Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            28..=30 => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            31 => Err(Error::new(ErrorKind::UnexpectedType, at)),
            _ => unreachable!(),
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn unsigned_basic(&mut self, initial: u8) -> Result<u64> {
        let at = self.pos;
        self.pos += 1;
        if initial >> 5 != 0 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        self.basic_integer_argument(initial & 31, at)
    }
    #[cfg(any(feature = "alloc", feature = "serde"))]
    #[inline(always)]
    fn integer_i64_basic(&mut self, initial: u8) -> Result<i64> {
        let at = self.pos;
        self.pos += 1;
        let major = initial >> 5;
        let argument = self.basic_integer_argument(initial & 31, at)?;
        if argument > i64::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, self.pos));
        }
        match major {
            0 => Ok(argument as i64),
            1 => Ok(!(argument as i64)),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(any(feature = "alloc", feature = "serde"))]
    #[inline(always)]
    fn integer_i32_basic(&mut self, initial: u8) -> Result<i32> {
        let at = self.pos;
        self.pos += 1;
        let major = initial >> 5;
        let argument = self.basic_integer_argument(initial & 31, at)?;
        if argument > i32::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, self.pos));
        }
        match major {
            0 => Ok(argument as i32),
            1 => Ok(!(argument as i32)),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(any(feature = "alloc", feature = "serde"))]
    #[inline(always)]
    fn integer_i16_basic(&mut self, initial: u8) -> Result<i16> {
        let at = self.pos;
        self.pos += 1;
        let major = initial >> 5;
        let argument = self.basic_integer_argument(initial & 31, at)?;
        if argument > i16::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, self.pos));
        }
        match major {
            0 => Ok(argument as i16),
            1 => Ok(!(argument as i16)),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(any(feature = "alloc", feature = "serde"))]
    #[inline(always)]
    fn integer_i8_basic(&mut self, initial: u8) -> Result<i8> {
        let at = self.pos;
        self.pos += 1;
        let major = initial >> 5;
        let argument = self.basic_integer_argument(initial & 31, at)?;
        if argument > i8::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, self.pos));
        }
        match major {
            0 => Ok(argument as i8),
            1 => Ok(!(argument as i8)),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn bool_basic(&mut self, initial: u8) -> Result<bool> {
        let at = self.pos;
        self.pos += 1;
        match initial {
            0xf4 => Ok(false),
            0xf5 => Ok(true),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn unsigned_u8_one_byte(&mut self) -> Result<u8> {
        let at = self.pos;
        self.pos += 1;
        let value = self.byte()?;
        if value < 24 && self.options.validation == Validation::Deterministic {
            return Err(Error::new(ErrorKind::NonDeterministic, at));
        }
        Ok(value)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn unsigned_u8_basic(&mut self, initial: u8) -> Result<u8> {
        let at = self.pos;
        self.pos += 1;
        let argument = self.basic_integer_argument(initial & 31, at)?;
        if initial >> 5 != 0 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        u8::try_from(argument).map_err(|_| Error::new(ErrorKind::IntegerOverflow, self.pos))
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn collection_basic(&mut self, initial: u8, expected_major: u8) -> Result<Option<u64>> {
        let at = self.pos;
        self.pos += 1;
        if initial >> 5 != expected_major {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        if initial & 31 == 31 {
            Ok(None)
        } else {
            self.basic_integer_argument(initial & 31, at).map(Some)
        }
    }
    /// Decodes a definite-length byte string.
    pub fn bytes(&mut self) -> Result<&'de [u8]> {
        let (m, n, at) = self.header()?;
        if m != 2 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let n = n.ok_or(Error::new(ErrorKind::UnexpectedType, at))?;
        self.take(usize::try_from(n).map_err(|_| Error::new(ErrorKind::CollectionLimit, at))?)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn bytes_basic(&mut self, initial: u8, expected_major: u8) -> Result<&'de [u8]> {
        let at = self.pos;
        self.pos += 1;
        if initial >> 5 != expected_major {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let length = self.basic_integer_argument(initial & 31, at)?;
        self.take(usize::try_from(length).map_err(|_| Error::new(ErrorKind::CollectionLimit, at))?)
    }
    /// Decodes a definite-length UTF-8 text string.
    pub fn text(&mut self) -> Result<&'de str> {
        let at = self.pos;
        let bytes = self.bytes_major(3)?;
        validated_str(bytes, at)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn text_basic(&mut self, initial: u8) -> Result<&'de str> {
        let at = self.pos;
        let bytes = self.bytes_basic(initial, 3)?;
        validated_str(bytes, at)
    }

    /// Decodes a floating-point value and returns it as `f64`.
    pub fn float(&mut self) -> Result<f64> {
        let initial = self.peek()?;
        if self.options.validation == Validation::Basic && matches!(initial, 0xf9..=0xfb) {
            return self.float_basic(initial);
        }
        let (major, argument, at) = self.header()?;
        if major != 7 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let argument = argument.ok_or(Error::new(ErrorKind::UnexpectedType, at))?;
        self.validate_deterministic_float(initial & 31, argument, at)?;
        match initial & 31 {
            25 => Ok(half_to_f64(argument as u16)),
            26 => Ok(f32::from_bits(argument as u32) as f64),
            27 => Ok(f64::from_bits(argument)),
            _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
        }
    }
    #[inline]
    fn validate_deterministic_float(&self, additional: u8, value: u64, at: usize) -> Result<()> {
        if self.options.validation != Validation::Deterministic {
            return Ok(());
        }
        let non_preferred = match additional {
            25 => {
                let bits = value as u16;
                half_to_f64(bits).is_nan() && bits != 0x7e00
            }
            26 => crate::encode::exact_half(f32::from_bits(value as u32)).is_some(),
            27 => {
                let float = f64::from_bits(value);
                float.is_nan() || (float as f32) as f64 == float
            }
            _ => false,
        };
        if non_preferred {
            Err(Error::new(ErrorKind::NonDeterministic, at))
        } else {
            Ok(())
        }
    }
    #[inline(always)]
    fn float_basic(&mut self, initial: u8) -> Result<f64> {
        self.pos += 1;
        match initial {
            0xf9 => {
                let bits = u16::from_be_bytes(self.take(2)?.try_into().unwrap());
                Ok(half_to_f64(bits))
            }
            0xfa => {
                let bits = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
                Ok(f32::from_bits(bits) as f64)
            }
            0xfb => {
                let bits = u64::from_be_bytes(self.take(8)?.try_into().unwrap());
                Ok(f64::from_bits(bits))
            }
            _ => unreachable!(),
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    fn f32_basic(&mut self) -> Result<f32> {
        self.pos += 1;
        let bits = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
        Ok(f32::from_bits(bits))
    }
    fn bytes_major(&mut self, expected: u8) -> Result<&'de [u8]> {
        let (m, n, at) = self.header()?;
        if m != expected {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let n = n.ok_or(Error::new(ErrorKind::UnexpectedType, at))?;
        self.take(usize::try_from(n).map_err(|_| Error::new(ErrorKind::CollectionLimit, at))?)
    }
    /// Consumes and returns the complete encoding of the next item.
    pub fn raw(&mut self) -> Result<RawValue<'de>> {
        let start = self.pos;
        self.skip()?;
        Ok(RawValue(&self.input[start..self.pos]))
    }
    #[cfg(feature = "parallel")]
    pub(crate) fn raw_structural(&mut self) -> Result<RawValue<'de>> {
        let start = self.pos;
        self.skip_structural_at(0)?;
        Ok(RawValue(&self.input[start..self.pos]))
    }
    /// Consumes the next complete item without constructing a value.
    pub fn skip(&mut self) -> Result<()> {
        if self.options.validation == Validation::Deterministic {
            self.skip_at(0, true)
        } else {
            self.skip_basic_at(0)
        }
    }
    #[inline]
    fn basic_header(&mut self) -> Result<(u8, Option<u64>, usize)> {
        let at = self.pos;
        let initial = self.byte()?;
        let argument = match initial & 31 {
            additional @ 0..=23 => Some(additional as u64),
            24 => Some(self.byte()? as u64),
            25 => Some(u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64),
            26 => Some(u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64),
            27 => Some(u64::from_be_bytes(self.take(8)?.try_into().unwrap())),
            31 => None,
            _ => return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
        };
        Ok((initial >> 5, argument, at))
    }
    fn skip_basic_at(&mut self, depth: usize) -> Result<()> {
        if depth > self.options.max_depth {
            return Err(Error::new(ErrorKind::DepthLimit, self.pos));
        }
        let (major, argument, at) = self.basic_header()?;
        match major {
            0 | 1 => {
                if argument.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
            }
            2 | 3 => match argument {
                Some(length) => {
                    if length > self.options.max_collection_len as u64 {
                        return Err(Error::new(ErrorKind::CollectionLimit, at));
                    }
                    let length = usize::try_from(length)
                        .map_err(|_| Error::new(ErrorKind::CollectionLimit, at))?;
                    let data = self.take(length)?;
                    if major == 3 && validated_str(data, at).is_err() {
                        return Err(Error::new(ErrorKind::InvalidUtf8, at));
                    }
                }
                None => loop {
                    if self.peek()? == 0xff {
                        self.pos += 1;
                        break;
                    }
                    let chunk_at = self.pos;
                    let (chunk_major, chunk_length, _) = self.basic_header()?;
                    if chunk_major != major {
                        return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
                    }
                    let chunk_length =
                        chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
                    let chunk_length = usize::try_from(chunk_length)
                        .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
                    let data = self.take(chunk_length)?;
                    if major == 3 && validated_str(data, chunk_at).is_err() {
                        return Err(Error::new(ErrorKind::InvalidUtf8, chunk_at));
                    }
                },
            },
            4 => self.skip_basic_container(argument, false, depth, at)?,
            5 => self.skip_basic_container(argument, true, depth, at)?,
            6 => {
                if argument.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
                self.skip_basic_at(depth + 1)?;
            }
            7 => {
                let value = argument.ok_or(Error::new(ErrorKind::UnexpectedBreak, at))?;
                if self.input[at] & 31 == 24 && value < 32 {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
    fn skip_basic_container(
        &mut self,
        length: Option<u64>,
        map: bool,
        depth: usize,
        at: usize,
    ) -> Result<()> {
        if let Some(length) = length {
            if length > self.options.max_collection_len as u64 {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            let items = length
                .checked_mul(if map { 2 } else { 1 })
                .ok_or(Error::new(ErrorKind::CollectionLimit, at))?;
            for _ in 0..items {
                self.skip_basic_at(depth + 1)?;
            }
        } else {
            let mut items = 0usize;
            loop {
                if self.peek()? == 0xff {
                    self.pos += 1;
                    if map && !items.is_multiple_of(2) {
                        return Err(Error::new(ErrorKind::UnexpectedBreak, self.pos - 1));
                    }
                    break;
                }
                self.skip_basic_at(depth + 1)?;
                items += 1;
                if items
                    > self
                        .options
                        .max_collection_len
                        .saturating_mul(if map { 2 } else { 1 })
                {
                    return Err(Error::new(ErrorKind::CollectionLimit, at));
                }
            }
        }
        Ok(())
    }
    fn skip_at(&mut self, depth: usize, validate_utf8: bool) -> Result<()> {
        if depth > self.options.max_depth {
            return Err(Error::new(ErrorKind::DepthLimit, self.pos));
        }
        let (major, arg, at) = self.header()?;
        match major {
            0 | 1 => {
                if arg.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
            }
            2 | 3 => match arg {
                Some(n) => {
                    if n > self.options.max_collection_len as u64 {
                        return Err(Error::new(ErrorKind::CollectionLimit, at));
                    }
                    let data = self.take(n as usize)?;
                    if validate_utf8 && major == 3 && validated_str(data, at).is_err() {
                        return Err(Error::new(ErrorKind::InvalidUtf8, at));
                    }
                }
                None => {
                    if self.options.validation == Validation::Deterministic {
                        return Err(Error::new(ErrorKind::NonDeterministic, at));
                    }
                    loop {
                        if self.peek()? == 0xff {
                            self.pos += 1;
                            break;
                        }
                        let chunk_at = self.pos;
                        let (cm, cn, _) = self.header()?;
                        if cm != major {
                            return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
                        }
                        let n = cn.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))? as usize;
                        let data = self.take(n)?;
                        if validate_utf8 && major == 3 && validated_str(data, chunk_at).is_err() {
                            return Err(Error::new(ErrorKind::InvalidUtf8, chunk_at));
                        }
                    }
                }
            },
            4 => self.skip_container(arg, false, depth, at, validate_utf8)?,
            5 => self.skip_container(arg, true, depth, at, validate_utf8)?,
            6 => {
                if arg.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
                self.skip_at(depth + 1, validate_utf8)?;
            }
            7 => match arg {
                None => return Err(Error::new(ErrorKind::UnexpectedBreak, at)),
                Some(value) => {
                    let ai = self.input[at] & 31;
                    if ai == 24 && self.input[at + 1] < 32 {
                        return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                    }
                    self.validate_deterministic_float(ai, value, at)?;
                }
            },
            _ => unreachable!(),
        }
        Ok(())
    }
    fn skip_container(
        &mut self,
        len: Option<u64>,
        map: bool,
        depth: usize,
        at: usize,
        validate_utf8: bool,
    ) -> Result<()> {
        if len.is_none() && self.options.validation == Validation::Deterministic {
            return Err(Error::new(ErrorKind::NonDeterministic, at));
        }
        if let Some(n) = len {
            if n > self.options.max_collection_len as u64 {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            let count = n
                .checked_mul(if map { 2 } else { 1 })
                .ok_or(Error::new(ErrorKind::CollectionLimit, at))?;
            let mut previous: Option<&[u8]> = None;
            for i in 0..count {
                let start = self.pos;
                self.skip_at(depth + 1, validate_utf8)?;
                if map && i % 2 == 0 && self.options.validation == Validation::Deterministic {
                    let key = &self.input[start..self.pos];
                    if previous.is_some_and(|p| p >= key) {
                        return Err(Error::new(ErrorKind::NonDeterministic, start));
                    }
                    previous = Some(key);
                }
            }
        } else {
            let mut count = 0usize;
            loop {
                if self.peek()? == 0xff {
                    self.pos += 1;
                    if map && !count.is_multiple_of(2) {
                        return Err(Error::new(ErrorKind::UnexpectedBreak, self.pos - 1));
                    }
                    break;
                }
                self.skip_at(depth + 1, validate_utf8)?;
                count += 1;
                if count
                    > self
                        .options
                        .max_collection_len
                        .saturating_mul(if map { 2 } else { 1 })
                {
                    return Err(Error::new(ErrorKind::CollectionLimit, at));
                }
            }
        }
        Ok(())
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn structural_argument(&mut self, additional: u8, head: usize) -> Result<Option<u64>> {
        match additional {
            0..=23 => Ok(Some(additional as u64)),
            24 => Ok(Some(self.byte()? as u64)),
            25 => Ok(Some(
                u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64
            )),
            26 => Ok(Some(
                u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64
            )),
            27 => Ok(Some(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))),
            31 => Ok(None),
            _ => Err(Error::new(ErrorKind::InvalidAdditionalInfo, head)),
        }
    }

    #[cfg(feature = "parallel")]
    #[inline]
    fn structural_header(&mut self) -> Result<(u8, Option<u64>, usize)> {
        let at = self.pos;
        let initial = self.byte()?;
        Ok((
            initial >> 5,
            self.structural_argument(initial & 31, at)?,
            at,
        ))
    }

    #[cfg(feature = "parallel")]
    fn skip_structural_at(&mut self, depth: usize) -> Result<()> {
        if depth > self.options.max_depth {
            return Err(Error::new(ErrorKind::DepthLimit, self.pos));
        }
        let (major, argument, at) = self.structural_header()?;
        match major {
            0 | 1 => {
                if argument.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
            }
            2 | 3 => match argument {
                Some(length) => {
                    if length > self.options.max_collection_len as u64 {
                        return Err(Error::new(ErrorKind::CollectionLimit, at));
                    }
                    let length = usize::try_from(length)
                        .map_err(|_| Error::new(ErrorKind::CollectionLimit, at))?;
                    self.take(length)?;
                }
                None => loop {
                    if self.peek()? == 0xff {
                        self.pos += 1;
                        break;
                    }
                    let chunk_at = self.pos;
                    let (chunk_major, chunk_length, _) = self.structural_header()?;
                    if chunk_major != major {
                        return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
                    }
                    let chunk_length =
                        chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
                    let chunk_length = usize::try_from(chunk_length)
                        .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
                    self.take(chunk_length)?;
                },
            },
            4 => self.skip_structural_container(argument, false, depth, at)?,
            5 => self.skip_structural_container(argument, true, depth, at)?,
            6 => {
                if argument.is_none() {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
                self.skip_structural_at(depth + 1)?;
            }
            7 => {
                let value = argument.ok_or(Error::new(ErrorKind::UnexpectedBreak, at))?;
                if self.input[at] & 31 == 24 && value < 32 {
                    return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at));
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    #[cfg(feature = "parallel")]
    fn skip_structural_container(
        &mut self,
        length: Option<u64>,
        map: bool,
        depth: usize,
        at: usize,
    ) -> Result<()> {
        if let Some(length) = length {
            if length > self.options.max_collection_len as u64 {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            let items = length
                .checked_mul(if map { 2 } else { 1 })
                .ok_or(Error::new(ErrorKind::CollectionLimit, at))?;
            for _ in 0..items {
                self.skip_structural_at(depth + 1)?;
            }
        } else {
            let mut items = 0usize;
            loop {
                if self.peek()? == 0xff {
                    self.pos += 1;
                    if map && !items.is_multiple_of(2) {
                        return Err(Error::new(ErrorKind::UnexpectedBreak, self.pos - 1));
                    }
                    break;
                }
                self.skip_structural_at(depth + 1)?;
                items += 1;
                if items
                    > self
                        .options
                        .max_collection_len
                        .saturating_mul(if map { 2 } else { 1 })
                {
                    return Err(Error::new(ErrorKind::CollectionLimit, at));
                }
            }
        }
        Ok(())
    }
}

#[inline(always)]
fn is_complete_one_byte_item(initial: u8) -> bool {
    matches!(
        initial,
        0x00..=0x17 | 0x20..=0x37 | 0x40 | 0x60 | 0x80 | 0xa0 | 0xe0..=0xf7
    )
}

#[cfg(feature = "alloc")]
#[inline(never)]
pub(crate) fn decode_borrowed_value(input: &[u8]) -> Result<BorrowedValue<'_>> {
    let mut decoder = SliceDecoder::new(input);
    let value = decode_borrowed_value_at(&mut decoder, 0)?;
    decoder.finish()?;
    Ok(value)
}

#[cfg(feature = "alloc")]
fn decode_borrowed_value_at<'de>(
    decoder: &mut SliceDecoder<'de>,
    depth: usize,
) -> Result<BorrowedValue<'de>> {
    if depth > decoder.options.max_depth {
        return Err(Error::new(ErrorKind::DepthLimit, decoder.pos));
    }

    let initial = decoder.peek()?;
    let (major, argument, at) = decoder.header()?;
    match major {
        0 => Ok(BorrowedValue::Unsigned(
            argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))?,
        )),
        1 => Ok(BorrowedValue::Negative(
            -1 - argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))? as i128,
        )),
        2 | 3 => decode_borrowed_string(decoder, major, argument, at),
        4 => decode_borrowed_array(decoder, argument, depth, at),
        5 => decode_borrowed_map(decoder, argument, depth, at),
        6 => {
            let tag = argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))?;
            Ok(BorrowedValue::Tag(
                tag,
                Box::new(decode_borrowed_value_at(decoder, depth + 1)?),
            ))
        }
        7 => match initial & 31 {
            20 => Ok(BorrowedValue::Bool(false)),
            21 => Ok(BorrowedValue::Bool(true)),
            22 => Ok(BorrowedValue::Null),
            23 => Ok(BorrowedValue::Undefined),
            24 if argument.unwrap() >= 32 => Ok(BorrowedValue::Simple(argument.unwrap() as u8)),
            24 => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            25 => Ok(BorrowedValue::Float(half_to_f64(argument.unwrap() as u16))),
            26 => Ok(BorrowedValue::Float(
                f32::from_bits(argument.unwrap() as u32) as f64,
            )),
            27 => Ok(BorrowedValue::Float(f64::from_bits(argument.unwrap()))),
            n @ 0..=19 => Ok(BorrowedValue::Simple(n)),
            31 => Err(Error::new(ErrorKind::UnexpectedBreak, at)),
            _ => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
        },
        _ => unreachable!(),
    }
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_borrowed_string<'de>(
    decoder: &mut SliceDecoder<'de>,
    major: u8,
    length: Option<u64>,
    at: usize,
) -> Result<BorrowedValue<'de>> {
    if let Some(length) = length {
        let length = checked_collection_len(decoder, length, at)?;
        let bytes = decoder.take(length)?;
        return if major == 2 {
            Ok(BorrowedValue::Bytes(Cow::Borrowed(bytes)))
        } else {
            Ok(BorrowedValue::Text(Cow::Borrowed(validated_str(
                bytes, at,
            )?)))
        };
    }

    if decoder.options.validation == Validation::Deterministic {
        return Err(Error::new(ErrorKind::NonDeterministic, at));
    }
    if major == 2 {
        let mut joined = Vec::new();
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            let chunk_at = decoder.pos;
            let (chunk_major, chunk_length, _) = decoder.header()?;
            if chunk_major != major {
                return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
            }
            let chunk_length =
                chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
            let total = (joined.len() as u64)
                .checked_add(chunk_length)
                .ok_or(Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            checked_collection_len(decoder, total, chunk_at)?;
            let chunk_length = usize::try_from(chunk_length)
                .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            joined.extend_from_slice(decoder.take(chunk_length)?);
        }
        Ok(BorrowedValue::Bytes(Cow::Owned(joined)))
    } else {
        let mut joined = String::new();
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            let chunk_at = decoder.pos;
            let (chunk_major, chunk_length, _) = decoder.header()?;
            if chunk_major != major {
                return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
            }
            let chunk_length =
                chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
            let total = (joined.len() as u64)
                .checked_add(chunk_length)
                .ok_or(Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            checked_collection_len(decoder, total, chunk_at)?;
            let chunk_length = usize::try_from(chunk_length)
                .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            joined.push_str(validated_str(decoder.take(chunk_length)?, chunk_at)?);
        }
        Ok(BorrowedValue::Text(Cow::Owned(joined)))
    }
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_borrowed_array<'de>(
    decoder: &mut SliceDecoder<'de>,
    length: Option<u64>,
    depth: usize,
    at: usize,
) -> Result<BorrowedValue<'de>> {
    let mut values = match length {
        Some(length) => Vec::with_capacity(checked_collection_len(decoder, length, at)?),
        None => {
            if decoder.options.validation == Validation::Deterministic {
                return Err(Error::new(ErrorKind::NonDeterministic, at));
            }
            Vec::new()
        }
    };

    if let Some(length) = length {
        for _ in 0..length {
            values.push(decode_borrowed_value_at(decoder, depth + 1)?);
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if values.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            values.push(decode_borrowed_value_at(decoder, depth + 1)?);
        }
    }
    Ok(BorrowedValue::Array(values))
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_borrowed_map<'de>(
    decoder: &mut SliceDecoder<'de>,
    length: Option<u64>,
    depth: usize,
    at: usize,
) -> Result<BorrowedValue<'de>> {
    let mut entries = match length {
        Some(length) => Vec::with_capacity(checked_collection_len(decoder, length, at)?),
        None => {
            if decoder.options.validation == Validation::Deterministic {
                return Err(Error::new(ErrorKind::NonDeterministic, at));
            }
            Vec::new()
        }
    };

    if let Some(length) = length {
        for _ in 0..length {
            let key = decode_borrowed_value_at(decoder, depth + 1)?;
            let value = decode_borrowed_value_at(decoder, depth + 1)?;
            entries.push((key, value));
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if entries.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            let key = decode_borrowed_value_at(decoder, depth + 1)?;
            if decoder.peek()? == 0xff {
                return Err(Error::new(ErrorKind::UnexpectedBreak, decoder.pos));
            }
            let value = decode_borrowed_value_at(decoder, depth + 1)?;
            entries.push((key, value));
        }
    }
    Ok(BorrowedValue::Map(entries))
}

/// Decodes one CBOR array of native integers directly into `i32` values.
///
/// Definite and indefinite arrays are accepted. Positive and negative CBOR
/// integers may use any argument width; values outside the `i32` range are
/// rejected. This bulk path avoids per-element Serde dispatch.
#[cfg(feature = "alloc")]
pub fn from_slice_i32_array(input: &[u8]) -> Result<Vec<i32>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_i32_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        values.push(decode_i32_array_element(&mut decoder)?);
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of native integers directly into `i64` values.
///
/// Definite and indefinite arrays are accepted. Positive and negative CBOR
/// integers may use any argument width; values outside the `i64` range are
/// rejected. This bulk path avoids per-element Serde dispatch.
#[cfg(feature = "alloc")]
pub fn from_slice_i64_array(input: &[u8]) -> Result<Vec<i64>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_i64_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        values.push(decode_i64_array_element(&mut decoder)?);
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of native unsigned integers directly into `u64`
/// values.
///
/// Definite and indefinite arrays are accepted, and integers may use any CBOR
/// argument width. Negative integers and other value types are rejected. This
/// bulk path avoids per-element Serde dispatch.
#[cfg(feature = "alloc")]
pub fn from_slice_u64_array(input: &[u8]) -> Result<Vec<u64>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_u64_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        values.push(decoder.unsigned()?);
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of unsigned integers directly into `u32` values.
///
/// Definite and indefinite arrays are accepted, and elements may use any CBOR
/// unsigned argument width. Values above `u32::MAX`, negative integers, and
/// other value types are rejected.
#[cfg(feature = "alloc")]
pub fn from_slice_u32_array(input: &[u8]) -> Result<Vec<u32>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_u32_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        let value = decoder.unsigned()?;
        values.push(
            u32::try_from(value)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, decoder.position()))?,
        );
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of unsigned integers directly into `u16` values.
///
/// Definite and indefinite arrays are accepted, and elements may use any CBOR
/// unsigned argument width. Values above `u16::MAX`, negative integers, and
/// other value types are rejected.
#[cfg(feature = "alloc")]
pub fn from_slice_u16_array(input: &[u8]) -> Result<Vec<u16>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_u16_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        let value = decoder.unsigned()?;
        values.push(
            u16::try_from(value)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, decoder.position()))?,
        );
    }
    decoder.finish()?;
    Ok(values)
}

#[cfg(feature = "alloc")]
fn decode_u16_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<u16>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 12)
            && encoded[0] == 0x19
            && encoded[3] == 0x19
            && encoded[6] == 0x19
            && encoded[9] == 0x19
        {
            values.extend_from_slice(&[
                u16::from_be_bytes([encoded[1], encoded[2]]),
                u16::from_be_bytes([encoded[4], encoded[5]]),
                u16::from_be_bytes([encoded[7], encoded[8]]),
                u16::from_be_bytes([encoded[10], encoded[11]]),
            ]);
            pos += 12;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        let value = match initial {
            0x00..=0x17 => initial as u16,
            0x18 => {
                let value = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 1;
                value as u16
            }
            0x19 => {
                let bytes = input
                    .get(pos..pos + 2)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 2;
                u16::from_be_bytes(bytes.try_into().unwrap())
            }
            0x1a => {
                let bytes = input
                    .get(pos..pos + 4)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 4;
                u16::try_from(u32::from_be_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, pos))?
            }
            0x1b => {
                let bytes = input
                    .get(pos..pos + 8)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 8;
                u16::try_from(u64::from_be_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, pos))?
            }
            0x1c..=0x1e => return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            _ => return Err(Error::new(ErrorKind::UnexpectedType, at)),
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
fn decode_u32_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<u32>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 20)
            && encoded[0] == 0x1a
            && encoded[5] == 0x1a
            && encoded[10] == 0x1a
            && encoded[15] == 0x1a
        {
            values.extend_from_slice(&[
                u32::from_be_bytes([encoded[1], encoded[2], encoded[3], encoded[4]]),
                u32::from_be_bytes([encoded[6], encoded[7], encoded[8], encoded[9]]),
                u32::from_be_bytes([encoded[11], encoded[12], encoded[13], encoded[14]]),
                u32::from_be_bytes([encoded[16], encoded[17], encoded[18], encoded[19]]),
            ]);
            pos += 20;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        if initial >> 5 != 0 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let value = match initial & 31 {
            value @ 0..=23 => value as u32,
            24 => {
                let value = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 1;
                value as u32
            }
            25 => {
                let bytes = input
                    .get(pos..pos + 2)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 2;
                u16::from_be_bytes(bytes.try_into().unwrap()) as u32
            }
            26 => {
                let bytes = input
                    .get(pos..pos + 4)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 4;
                u32::from_be_bytes(bytes.try_into().unwrap())
            }
            27 => {
                let bytes = input
                    .get(pos..pos + 8)
                    .ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 8;
                u32::try_from(u64::from_be_bytes(bytes.try_into().unwrap()))
                    .map_err(|_| Error::new(ErrorKind::IntegerOverflow, pos))?
            }
            28..=30 => return Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            31 => return Err(Error::new(ErrorKind::UnexpectedType, at)),
            _ => unreachable!(),
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
macro_rules! define_integer_array_into {
    ($name:ident, $owned:ident, $element:ty, $body:ident) => {
        /// Decodes into `output`, clearing its contents while retaining capacity.
        ///
        /// On failure, `output` is empty and remains reusable.
        pub fn $name(input: &[u8], output: &mut Vec<$element>) -> Result<()> {
            output.clear();
            let result = (|| {
                let mut decoder = SliceDecoder::new(input);
                let (major, length, at) = decoder.header()?;
                if major != 4 {
                    return Err(Error::new(ErrorKind::UnexpectedType, at));
                }
                if let Some(length) = length {
                    let length = checked_collection_len(&decoder, length, at)?;
                    decoder.pos = $body(input, decoder.position(), length, output)?;
                    decoder.finish()
                } else {
                    output.extend($owned(input)?);
                    Ok(())
                }
            })();
            if result.is_err() {
                output.clear();
            }
            result
        }
    };
}

#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_u8_array_into,
    from_slice_u8_array,
    u8,
    decode_u8_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_u16_array_into,
    from_slice_u16_array,
    u16,
    decode_u16_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_u32_array_into,
    from_slice_u32_array,
    u32,
    decode_u32_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_u64_array_into,
    from_slice_u64_array,
    u64,
    decode_u64_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_i8_array_into,
    from_slice_i8_array,
    i8,
    decode_i8_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_i16_array_into,
    from_slice_i16_array,
    i16,
    decode_i16_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_i32_array_into,
    from_slice_i32_array,
    i32,
    decode_i32_body
);
#[cfg(feature = "alloc")]
define_integer_array_into!(
    from_slice_i64_array_into,
    from_slice_i64_array,
    i64,
    decode_i64_body
);

/// Decodes one CBOR array of booleans without per-element Serde dispatch.
///
/// Definite and indefinite arrays are accepted. Every element must be the
/// single-byte CBOR encoding of `false` or `true`.
#[cfg(feature = "alloc")]
pub fn from_slice_bool_array(input: &[u8]) -> Result<Vec<bool>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_bool_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        let initial = decoder.peek()?;
        if initial == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        decoder.pos += 1;
        match initial {
            0xf4 => values.push(false),
            0xf5 => values.push(true),
            _ => return Err(Error::new(ErrorKind::UnexpectedType, decoder.pos - 1)),
        }
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of unsigned integers directly into `u8` values.
///
/// Definite and indefinite arrays are accepted, and elements may use any CBOR
/// unsigned argument width. Values above `u8::MAX`, negative integers, and
/// other value types are rejected.
#[cfg(feature = "alloc")]
pub fn from_slice_u8_array(input: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_u8_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        let value = decoder.unsigned()?;
        values.push(
            u8::try_from(value)
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, decoder.position()))?,
        );
    }
    decoder.finish()?;
    Ok(values)
}

#[cfg(feature = "alloc")]
fn decode_u8_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<u8>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded[0] == 0x18
            && encoded[2] == 0x18
            && encoded[4] == 0x18
            && encoded[6] == 0x18
        {
            values.extend_from_slice(&[encoded[1], encoded[3], encoded[5], encoded[7]]);
            pos += 8;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        let value = match initial {
            0x00..=0x17 => {
                pos += 1;
                initial
            }
            0x18 => {
                let value = *input
                    .get(pos + 1)
                    .ok_or(Error::new(ErrorKind::Eof, pos + 1))?;
                pos += 2;
                value
            }
            _ => decode_u8_uncommon(input, &mut pos, initial, at)?,
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_u8_uncommon(input: &[u8], pos: &mut usize, initial: u8, at: usize) -> Result<u8> {
    *pos += 1;
    match initial {
        0x19 => {
            let bytes = input
                .get(*pos..*pos + 2)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 2;
            u8::try_from(u16::from_be_bytes(bytes.try_into().unwrap()))
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, *pos))
        }
        0x1a => {
            let bytes = input
                .get(*pos..*pos + 4)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 4;
            u8::try_from(u32::from_be_bytes(bytes.try_into().unwrap()))
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, *pos))
        }
        0x1b => {
            let bytes = input
                .get(*pos..*pos + 8)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 8;
            u8::try_from(u64::from_be_bytes(bytes.try_into().unwrap()))
                .map_err(|_| Error::new(ErrorKind::IntegerOverflow, *pos))
        }
        0x1c..=0x1e => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
        _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
    }
}

#[cfg(feature = "alloc")]
fn decode_bool_body(
    input: &[u8],
    start: usize,
    length: usize,
    values: &mut Vec<bool>,
) -> Result<usize> {
    let end = start
        .checked_add(length)
        .ok_or(Error::new(ErrorKind::Eof, start))?;
    let body = input
        .get(start..end)
        .ok_or(Error::new(ErrorKind::Eof, input.len()))?;
    values.reserve(length);
    let mut index = 0;
    while index + 8 <= body.len() {
        let encoded = &body[index..index + 8];
        if encoded[0] & 0xfe != 0xf4
            || encoded[1] & 0xfe != 0xf4
            || encoded[2] & 0xfe != 0xf4
            || encoded[3] & 0xfe != 0xf4
            || encoded[4] & 0xfe != 0xf4
            || encoded[5] & 0xfe != 0xf4
            || encoded[6] & 0xfe != 0xf4
            || encoded[7] & 0xfe != 0xf4
        {
            let invalid = encoded
                .iter()
                .position(|initial| *initial & 0xfe != 0xf4)
                .unwrap();
            return Err(Error::new(
                ErrorKind::UnexpectedType,
                start + index + invalid,
            ));
        }
        values.extend_from_slice(&[
            encoded[0] == 0xf5,
            encoded[1] == 0xf5,
            encoded[2] == 0xf5,
            encoded[3] == 0xf5,
            encoded[4] == 0xf5,
            encoded[5] == 0xf5,
            encoded[6] == 0xf5,
            encoded[7] == 0xf5,
        ]);
        index += 8;
    }
    for (tail, initial) in body[index..].iter().copied().enumerate() {
        match initial {
            0xf4 => values.push(false),
            0xf5 => values.push(true),
            _ => return Err(Error::new(ErrorKind::UnexpectedType, start + index + tail)),
        }
    }
    Ok(end)
}

/// Decodes a boolean array into `output`, retaining its allocation for reuse.
///
/// On failure, `output` is empty and remains reusable.
#[cfg(feature = "alloc")]
pub fn from_slice_bool_array_into(input: &[u8], output: &mut Vec<bool>) -> Result<()> {
    output.clear();
    let result = (|| {
        let mut decoder = SliceDecoder::new(input);
        let (major, length, at) = decoder.header()?;
        if major != 4 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        if let Some(length) = length {
            let length = checked_collection_len(&decoder, length, at)?;
            decoder.pos = decode_bool_body(input, decoder.position(), length, output)?;
            decoder.finish()
        } else {
            output.extend(from_slice_bool_array(input)?);
            Ok(())
        }
    })();
    if result.is_err() {
        output.clear();
    }
    result
}

#[cfg(feature = "alloc")]
macro_rules! decode_integer_argument {
    ($input:expr, $pos:ident, $additional:expr, $at:expr) => {
        match $additional {
            value @ 0..=23 => value as u64,
            24 => {
                let value = *$input.get($pos).ok_or(Error::new(ErrorKind::Eof, $pos))?;
                $pos += 1;
                value as u64
            }
            25 => {
                let bytes = $input
                    .get($pos..$pos + 2)
                    .ok_or(Error::new(ErrorKind::Eof, $pos))?;
                $pos += 2;
                u16::from_be_bytes(bytes.try_into().unwrap()) as u64
            }
            26 => {
                let bytes = $input
                    .get($pos..$pos + 4)
                    .ok_or(Error::new(ErrorKind::Eof, $pos))?;
                $pos += 4;
                u32::from_be_bytes(bytes.try_into().unwrap()) as u64
            }
            27 => {
                let bytes = $input
                    .get($pos..$pos + 8)
                    .ok_or(Error::new(ErrorKind::Eof, $pos))?;
                $pos += 8;
                u64::from_be_bytes(bytes.try_into().unwrap())
            }
            28..=30 => return Err(Error::new(ErrorKind::InvalidAdditionalInfo, $at)),
            31 => return Err(Error::new(ErrorKind::UnexpectedType, $at)),
            _ => unreachable!(),
        }
    };
}

#[cfg(feature = "alloc")]
fn decode_u64_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<u64>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && input.get(pos) == Some(&0x1b)
            && let Some(encoded) = input.get(pos..pos + 36)
            && encoded[9] == 0x1b
            && encoded[18] == 0x1b
            && encoded[27] == 0x1b
        {
            values.extend_from_slice(&[
                u64::from_be_bytes(encoded[1..9].try_into().unwrap()),
                u64::from_be_bytes(encoded[10..18].try_into().unwrap()),
                u64::from_be_bytes(encoded[19..27].try_into().unwrap()),
                u64::from_be_bytes(encoded[28..36].try_into().unwrap()),
            ]);
            pos += 36;
            remaining -= 4;
            continue;
        }
        if remaining >= 8
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded.iter().all(|initial| *initial <= 0x17)
        {
            values.extend(encoded.iter().map(|value| *value as u64));
            pos += 8;
            remaining -= 8;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded[0] == 0x18
            && encoded[2] == 0x18
            && encoded[4] == 0x18
            && encoded[6] == 0x18
        {
            values.extend_from_slice(&[
                encoded[1] as u64,
                encoded[3] as u64,
                encoded[5] as u64,
                encoded[7] as u64,
            ]);
            pos += 8;
            remaining -= 4;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 12)
            && encoded[0] == 0x19
            && encoded[3] == 0x19
            && encoded[6] == 0x19
            && encoded[9] == 0x19
        {
            values.extend_from_slice(&[
                u16::from_be_bytes([encoded[1], encoded[2]]) as u64,
                u16::from_be_bytes([encoded[4], encoded[5]]) as u64,
                u16::from_be_bytes([encoded[7], encoded[8]]) as u64,
                u16::from_be_bytes([encoded[10], encoded[11]]) as u64,
            ]);
            pos += 12;
            remaining -= 4;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 20)
            && encoded[0] == 0x1a
            && encoded[5] == 0x1a
            && encoded[10] == 0x1a
            && encoded[15] == 0x1a
        {
            values.extend_from_slice(&[
                u32::from_be_bytes(encoded[1..5].try_into().unwrap()) as u64,
                u32::from_be_bytes(encoded[6..10].try_into().unwrap()) as u64,
                u32::from_be_bytes(encoded[11..15].try_into().unwrap()) as u64,
                u32::from_be_bytes(encoded[16..20].try_into().unwrap()) as u64,
            ]);
            pos += 20;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        if initial >> 5 != 0 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let value = decode_integer_argument!(input, pos, initial & 31, at);
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
fn decode_i64_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<i64>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && input.get(pos).is_some_and(|initial| initial & 0xdf == 0x1b)
            && let Some(encoded) = input.get(pos..pos + 36)
            && encoded[9] & 0xdf == 0x1b
            && encoded[18] & 0xdf == 0x1b
            && encoded[27] & 0xdf == 0x1b
            && encoded[1] & 0x80 == 0
            && encoded[10] & 0x80 == 0
            && encoded[19] & 0x80 == 0
            && encoded[28] & 0x80 == 0
        {
            let first = u64::from_be_bytes(encoded[1..9].try_into().unwrap()) as i64;
            let second = u64::from_be_bytes(encoded[10..18].try_into().unwrap()) as i64;
            let third = u64::from_be_bytes(encoded[19..27].try_into().unwrap()) as i64;
            let fourth = u64::from_be_bytes(encoded[28..36].try_into().unwrap()) as i64;
            values.extend_from_slice(&[
                if encoded[0] == 0x1b { first } else { !first },
                if encoded[9] == 0x1b { second } else { !second },
                if encoded[18] == 0x1b { third } else { !third },
                if encoded[27] == 0x1b { fourth } else { !fourth },
            ]);
            pos += 36;
            remaining -= 4;
            continue;
        }
        if remaining >= 8
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded
                .iter()
                .all(|initial| *initial <= 0x17 || (0x20..=0x37).contains(initial))
        {
            values.extend(encoded.iter().map(|initial| {
                let argument = (initial & 31) as i64;
                if initial >> 5 == 0 {
                    argument
                } else {
                    !argument
                }
            }));
            pos += 8;
            remaining -= 8;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded[0] & 0xdf == 0x18
            && encoded[2] & 0xdf == 0x18
            && encoded[4] & 0xdf == 0x18
            && encoded[6] & 0xdf == 0x18
        {
            let decode = |initial: u8, argument: u8| {
                if initial == 0x18 {
                    argument as i64
                } else {
                    !(argument as i64)
                }
            };
            values.extend_from_slice(&[
                decode(encoded[0], encoded[1]),
                decode(encoded[2], encoded[3]),
                decode(encoded[4], encoded[5]),
                decode(encoded[6], encoded[7]),
            ]);
            pos += 8;
            remaining -= 4;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 12)
            && encoded[0] & 0xdf == 0x19
            && encoded[3] & 0xdf == 0x19
            && encoded[6] & 0xdf == 0x19
            && encoded[9] & 0xdf == 0x19
        {
            let first = u16::from_be_bytes([encoded[1], encoded[2]]) as i64;
            let second = u16::from_be_bytes([encoded[4], encoded[5]]) as i64;
            let third = u16::from_be_bytes([encoded[7], encoded[8]]) as i64;
            let fourth = u16::from_be_bytes([encoded[10], encoded[11]]) as i64;
            values.extend_from_slice(&[
                if encoded[0] == 0x19 { first } else { !first },
                if encoded[3] == 0x19 { second } else { !second },
                if encoded[6] == 0x19 { third } else { !third },
                if encoded[9] == 0x19 { fourth } else { !fourth },
            ]);
            pos += 12;
            remaining -= 4;
            continue;
        }
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 20)
            && encoded[0] & 0xdf == 0x1a
            && encoded[5] & 0xdf == 0x1a
            && encoded[10] & 0xdf == 0x1a
            && encoded[15] & 0xdf == 0x1a
        {
            let first = u32::from_be_bytes(encoded[1..5].try_into().unwrap()) as i64;
            let second = u32::from_be_bytes(encoded[6..10].try_into().unwrap()) as i64;
            let third = u32::from_be_bytes(encoded[11..15].try_into().unwrap()) as i64;
            let fourth = u32::from_be_bytes(encoded[16..20].try_into().unwrap()) as i64;
            values.extend_from_slice(&[
                if encoded[0] == 0x1a { first } else { !first },
                if encoded[5] == 0x1a { second } else { !second },
                if encoded[10] == 0x1a { third } else { !third },
                if encoded[15] == 0x1a { fourth } else { !fourth },
            ]);
            pos += 20;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        let major = initial >> 5;
        if major > 1 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let argument = decode_integer_argument!(input, pos, initial & 31, at);
        if argument > i64::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, pos));
        }
        let value = if major == 0 {
            argument as i64
        } else {
            !(argument as i64)
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_i8_uncommon_argument(
    input: &[u8],
    pos: &mut usize,
    additional: u8,
    at: usize,
) -> Result<u64> {
    match additional {
        25 => {
            let bytes = input
                .get(*pos..*pos + 2)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 2;
            Ok(u16::from_be_bytes(bytes.try_into().unwrap()) as u64)
        }
        26 => {
            let bytes = input
                .get(*pos..*pos + 4)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 4;
            Ok(u32::from_be_bytes(bytes.try_into().unwrap()) as u64)
        }
        27 => {
            let bytes = input
                .get(*pos..*pos + 8)
                .ok_or(Error::new(ErrorKind::Eof, *pos))?;
            *pos += 8;
            Ok(u64::from_be_bytes(bytes.try_into().unwrap()))
        }
        28..=30 => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
        _ => Err(Error::new(ErrorKind::UnexpectedType, at)),
    }
}

/// Decodes one CBOR array of native integers directly into `i16` values.
///
/// Definite and indefinite arrays are accepted. Positive and negative CBOR
/// integers may use any argument width; values outside the `i16` range are
/// rejected.
#[cfg(feature = "alloc")]
pub fn from_slice_i16_array(input: &[u8]) -> Result<Vec<i16>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_i16_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        values.push(decode_i16_array_element(&mut decoder)?);
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes one CBOR array of native integers directly into `i8` values.
///
/// Definite and indefinite arrays are accepted. Positive and negative CBOR
/// integers may use any argument width; values outside the `i8` range are
/// rejected.
#[cfg(feature = "alloc")]
pub fn from_slice_i8_array(input: &[u8]) -> Result<Vec<i8>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            let mut values = Vec::new();
            let end = decode_i8_body(input, decoder.position(), length, &mut values)?;
            decoder.pos = end;
            decoder.finish()?;
            return Ok(values);
        }
        None => Vec::new(),
    };
    loop {
        if decoder.peek()? == 0xff {
            decoder.pos += 1;
            break;
        }
        if values.len() == decoder.options.max_collection_len {
            return Err(Error::new(ErrorKind::CollectionLimit, at));
        }
        values.push(decode_i8_array_element(&mut decoder)?);
    }
    decoder.finish()?;
    Ok(values)
}

#[cfg(feature = "alloc")]
fn decode_i8_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<i8>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 8)
            && encoded[0] & 0xdf == 0x18
            && encoded[2] & 0xdf == 0x18
            && encoded[4] & 0xdf == 0x18
            && encoded[6] & 0xdf == 0x18
            && (encoded[1] | encoded[3] | encoded[5] | encoded[7]) & 0x80 == 0
        {
            values.extend_from_slice(&[
                if encoded[0] == 0x18 {
                    encoded[1] as i8
                } else {
                    !(encoded[1] as i8)
                },
                if encoded[2] == 0x18 {
                    encoded[3] as i8
                } else {
                    !(encoded[3] as i8)
                },
                if encoded[4] == 0x18 {
                    encoded[5] as i8
                } else {
                    !(encoded[5] as i8)
                },
                if encoded[6] == 0x18 {
                    encoded[7] as i8
                } else {
                    !(encoded[7] as i8)
                },
            ]);
            pos += 8;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        let major = initial >> 5;
        if major > 1 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let additional = initial & 31;
        let argument = match additional {
            value @ 0..=23 => value as u64,
            24 => {
                let value = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
                pos += 1;
                value as u64
            }
            _ => decode_i8_uncommon_argument(input, &mut pos, additional, at)?,
        };
        if argument > i8::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, pos));
        }
        let value = if major == 0 {
            argument as i8
        } else {
            !(argument as i8)
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
fn decode_i16_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<i16>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 12)
            && encoded[0] & 0xdf == 0x19
            && encoded[3] & 0xdf == 0x19
            && encoded[6] & 0xdf == 0x19
            && encoded[9] & 0xdf == 0x19
            && (encoded[1] | encoded[4] | encoded[7] | encoded[10]) & 0x80 == 0
        {
            let first = u16::from_be_bytes([encoded[1], encoded[2]]) as i16;
            let second = u16::from_be_bytes([encoded[4], encoded[5]]) as i16;
            let third = u16::from_be_bytes([encoded[7], encoded[8]]) as i16;
            let fourth = u16::from_be_bytes([encoded[10], encoded[11]]) as i16;
            values.extend_from_slice(&[
                if encoded[0] == 0x19 { first } else { !first },
                if encoded[3] == 0x19 { second } else { !second },
                if encoded[6] == 0x19 { third } else { !third },
                if encoded[9] == 0x19 { fourth } else { !fourth },
            ]);
            pos += 12;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        let major = initial >> 5;
        if major > 1 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let argument = decode_integer_argument!(input, pos, initial & 31, at);
        if argument > i16::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, pos));
        }
        let value = if major == 0 {
            argument as i16
        } else {
            !(argument as i16)
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
fn decode_i32_body(
    input: &[u8],
    mut pos: usize,
    length: usize,
    values: &mut Vec<i32>,
) -> Result<usize> {
    values.reserve(length);
    let mut remaining = length;
    while remaining != 0 {
        if remaining >= 4
            && let Some(encoded) = input.get(pos..pos + 12)
            && encoded[0] & 0xdf == 0x19
            && encoded[3] & 0xdf == 0x19
            && encoded[6] & 0xdf == 0x19
            && encoded[9] & 0xdf == 0x19
        {
            let first = u16::from_be_bytes([encoded[1], encoded[2]]) as i32;
            let second = u16::from_be_bytes([encoded[4], encoded[5]]) as i32;
            let third = u16::from_be_bytes([encoded[7], encoded[8]]) as i32;
            let fourth = u16::from_be_bytes([encoded[10], encoded[11]]) as i32;
            values.extend_from_slice(&[
                if encoded[0] == 0x19 { first } else { !first },
                if encoded[3] == 0x19 { second } else { !second },
                if encoded[6] == 0x19 { third } else { !third },
                if encoded[9] == 0x19 { fourth } else { !fourth },
            ]);
            pos += 12;
            remaining -= 4;
            continue;
        }
        let at = pos;
        let initial = *input.get(pos).ok_or(Error::new(ErrorKind::Eof, pos))?;
        pos += 1;
        let major = initial >> 5;
        if major > 1 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        let argument = decode_integer_argument!(input, pos, initial & 31, at);
        if argument > i32::MAX as u64 {
            return Err(Error::new(ErrorKind::IntegerOverflow, pos));
        }
        let value = if major == 0 {
            argument as i32
        } else {
            !(argument as i32)
        };
        values.push(value);
        remaining -= 1;
    }
    Ok(pos)
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_i32_array_element(decoder: &mut SliceDecoder<'_>) -> Result<i32> {
    let initial = decoder.peek()?;
    decoder.integer_i32_basic(initial)
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_i16_array_element(decoder: &mut SliceDecoder<'_>) -> Result<i16> {
    let initial = decoder.peek()?;
    decoder.integer_i16_basic(initial)
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_i8_array_element(decoder: &mut SliceDecoder<'_>) -> Result<i8> {
    let initial = decoder.peek()?;
    decoder.integer_i8_basic(initial)
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_i64_array_element(decoder: &mut SliceDecoder<'_>) -> Result<i64> {
    let initial = decoder.peek()?;
    decoder.integer_i64_basic(initial)
}

/// Decodes one CBOR array of numeric values directly into binary32 values.
///
/// Definite and indefinite arrays are accepted, as are unsigned and negative
/// integers and all three CBOR floating-point widths. Homogeneous binary32
/// arrays use a specialized path that avoids per-element Serde dispatch.
#[cfg(feature = "alloc")]
pub fn from_slice_f32_array(input: &[u8]) -> Result<Vec<f32>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            if let Some(byte_length) = length.checked_mul(5)
                && let Some(end) = decoder.position().checked_add(byte_length)
                && let Some(body) = input.get(decoder.position()..end)
                && {
                    let mut values = Vec::new();
                    if decode_fixed_f32_body(body, length, &mut values) {
                        decoder.pos = end;
                        decoder.finish()?;
                        return Ok(values);
                    }
                    false
                }
            {
                unreachable!();
            }
            Vec::with_capacity(length)
        }
        None => Vec::new(),
    };
    if let Some(length) = length {
        for _ in 0..length {
            values.push(decode_f32_array_element(&mut decoder)?);
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if values.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            values.push(decode_f32_array_element(&mut decoder)?);
        }
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes a numeric array into `output`, retaining its allocation for reuse.
///
/// On failure, `output` is empty and remains reusable.
#[cfg(feature = "alloc")]
pub fn from_slice_f32_array_into(input: &[u8], output: &mut Vec<f32>) -> Result<()> {
    output.clear();
    let result = (|| {
        let mut decoder = SliceDecoder::new(input);
        let (major, length, at) = decoder.header()?;
        if major != 4 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        if let Some(length) = length {
            let length = checked_collection_len(&decoder, length, at)?;
            if let Some(byte_length) = length.checked_mul(5)
                && let Some(end) = decoder.position().checked_add(byte_length)
                && let Some(body) = input.get(decoder.position()..end)
                && decode_fixed_f32_body(body, length, output)
            {
                decoder.pos = end;
                return decoder.finish();
            }
            output.reserve(length);
            for _ in 0..length {
                output.push(decode_f32_array_element(&mut decoder)?);
            }
        } else {
            loop {
                if decoder.peek()? == 0xff {
                    decoder.pos += 1;
                    break;
                }
                if output.len() == decoder.options.max_collection_len {
                    return Err(Error::new(ErrorKind::CollectionLimit, at));
                }
                output.push(decode_f32_array_element(&mut decoder)?);
            }
        }
        decoder.finish()
    })();
    if result.is_err() {
        output.clear();
    }
    result
}

#[cfg(feature = "alloc")]
#[inline]
fn decode_fixed_f32_body(body: &[u8], length: usize, values: &mut Vec<f32>) -> bool {
    let original_len = values.len();
    values.reserve(length);
    let mut pos = 0;
    let mut remaining = length;
    while remaining >= 4 {
        let encoded = &body[pos..pos + 20];
        if encoded[0] != 0xfa || encoded[5] != 0xfa || encoded[10] != 0xfa || encoded[15] != 0xfa {
            values.truncate(original_len);
            return false;
        }
        values.extend_from_slice(&[
            f32::from_bits(u32::from_be_bytes(encoded[1..5].try_into().unwrap())),
            f32::from_bits(u32::from_be_bytes(encoded[6..10].try_into().unwrap())),
            f32::from_bits(u32::from_be_bytes(encoded[11..15].try_into().unwrap())),
            f32::from_bits(u32::from_be_bytes(encoded[16..20].try_into().unwrap())),
        ]);
        pos += 20;
        remaining -= 4;
    }
    while remaining != 0 {
        let encoded = &body[pos..pos + 5];
        if encoded[0] != 0xfa {
            values.truncate(original_len);
            return false;
        }
        values.push(f32::from_bits(u32::from_be_bytes(
            encoded[1..5].try_into().unwrap(),
        )));
        pos += 5;
        remaining -= 1;
    }
    true
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_f32_array_element(decoder: &mut SliceDecoder<'_>) -> Result<f32> {
    let initial = decoder.peek()?;
    if initial == 0xfa {
        return Ok(decoder.float_basic(initial)? as f32);
    }
    match initial >> 5 {
        0 => Ok(decoder.unsigned()? as f32),
        1 => Ok(decoder.integer()? as f32),
        7 => Ok(decoder.float()? as f32),
        _ => Err(Error::new(ErrorKind::UnexpectedType, decoder.position())),
    }
}

/// Decodes one CBOR array of numeric values directly into binary64 values.
///
/// Definite and indefinite arrays are accepted, as are unsigned and negative
/// integers and all three CBOR floating-point widths. Homogeneous binary64
/// arrays use a specialized path that avoids per-element Serde dispatch.
#[cfg(feature = "alloc")]
pub fn from_slice_f64_array(input: &[u8]) -> Result<Vec<f64>> {
    let mut decoder = SliceDecoder::new(input);
    let (major, length, at) = decoder.header()?;
    if major != 4 {
        return Err(Error::new(ErrorKind::UnexpectedType, at));
    }
    let mut values = match length {
        Some(length) => {
            let length = checked_collection_len(&decoder, length, at)?;
            if let Some(byte_length) = length.checked_mul(9)
                && let Some(end) = decoder.position().checked_add(byte_length)
                && let Some(body) = input.get(decoder.position()..end)
                && {
                    let mut values = Vec::new();
                    if decode_fixed_f64_body(body, length, &mut values) {
                        decoder.pos = end;
                        decoder.finish()?;
                        return Ok(values);
                    }
                    false
                }
            {
                unreachable!();
            }
            Vec::with_capacity(length)
        }
        None => Vec::new(),
    };
    if let Some(length) = length {
        for _ in 0..length {
            values.push(decode_f64_array_element(&mut decoder)?);
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if values.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            values.push(decode_f64_array_element(&mut decoder)?);
        }
    }
    decoder.finish()?;
    Ok(values)
}

/// Decodes a numeric array into `output`, retaining its allocation for reuse.
///
/// On failure, `output` is empty and remains reusable.
#[cfg(feature = "alloc")]
pub fn from_slice_f64_array_into(input: &[u8], output: &mut Vec<f64>) -> Result<()> {
    output.clear();
    let result = (|| {
        let mut decoder = SliceDecoder::new(input);
        let (major, length, at) = decoder.header()?;
        if major != 4 {
            return Err(Error::new(ErrorKind::UnexpectedType, at));
        }
        if let Some(length) = length {
            let length = checked_collection_len(&decoder, length, at)?;
            if let Some(byte_length) = length.checked_mul(9)
                && let Some(end) = decoder.position().checked_add(byte_length)
                && let Some(body) = input.get(decoder.position()..end)
                && decode_fixed_f64_body(body, length, output)
            {
                decoder.pos = end;
                return decoder.finish();
            }
            output.reserve(length);
            for _ in 0..length {
                output.push(decode_f64_array_element(&mut decoder)?);
            }
        } else {
            loop {
                if decoder.peek()? == 0xff {
                    decoder.pos += 1;
                    break;
                }
                if output.len() == decoder.options.max_collection_len {
                    return Err(Error::new(ErrorKind::CollectionLimit, at));
                }
                output.push(decode_f64_array_element(&mut decoder)?);
            }
        }
        decoder.finish()
    })();
    if result.is_err() {
        output.clear();
    }
    result
}

#[cfg(feature = "alloc")]
#[inline]
fn decode_fixed_f64_body(body: &[u8], length: usize, values: &mut Vec<f64>) -> bool {
    let original_len = values.len();
    values.reserve(length);
    let mut pos = 0;
    let mut remaining = length;
    while remaining >= 4 {
        let encoded = &body[pos..pos + 36];
        if encoded[0] != 0xfb || encoded[9] != 0xfb || encoded[18] != 0xfb || encoded[27] != 0xfb {
            values.truncate(original_len);
            return false;
        }
        values.extend_from_slice(&[
            f64::from_bits(u64::from_be_bytes(encoded[1..9].try_into().unwrap())),
            f64::from_bits(u64::from_be_bytes(encoded[10..18].try_into().unwrap())),
            f64::from_bits(u64::from_be_bytes(encoded[19..27].try_into().unwrap())),
            f64::from_bits(u64::from_be_bytes(encoded[28..36].try_into().unwrap())),
        ]);
        pos += 36;
        remaining -= 4;
    }
    while remaining != 0 {
        let encoded = &body[pos..pos + 9];
        if encoded[0] != 0xfb {
            values.truncate(original_len);
            return false;
        }
        values.push(f64::from_bits(u64::from_be_bytes(
            encoded[1..9].try_into().unwrap(),
        )));
        pos += 9;
        remaining -= 1;
    }
    true
}

#[cfg(feature = "alloc")]
#[inline(always)]
fn decode_f64_array_element(decoder: &mut SliceDecoder<'_>) -> Result<f64> {
    let initial = decoder.peek()?;
    if initial == 0xfb {
        return decoder.float_basic(initial);
    }
    match initial >> 5 {
        0 => Ok(decoder.unsigned()? as f64),
        1 => Ok(decoder.integer()? as f64),
        7 => decoder.float(),
        _ => Err(Error::new(ErrorKind::UnexpectedType, decoder.position())),
    }
}

#[cfg(feature = "alloc")]
fn checked_collection_len(decoder: &SliceDecoder<'_>, length: u64, at: usize) -> Result<usize> {
    if length > decoder.options.max_collection_len as u64 {
        return Err(Error::new(ErrorKind::CollectionLimit, at));
    }
    usize::try_from(length).map_err(|_| Error::new(ErrorKind::CollectionLimit, at))
}

#[cfg(feature = "alloc")]
#[inline(never)]
pub(crate) fn decode_owned_value(input: &[u8]) -> Result<Value> {
    let mut decoder = SliceDecoder::new(input);
    let value = decode_owned_value_at(&mut decoder, 0)?;
    decoder.finish()?;
    Ok(value)
}

#[cfg(feature = "alloc")]
fn decode_owned_value_at(decoder: &mut SliceDecoder<'_>, depth: usize) -> Result<Value> {
    if depth > decoder.options.max_depth {
        return Err(Error::new(ErrorKind::DepthLimit, decoder.pos));
    }

    let initial = decoder.peek()?;
    let (major, argument, at) = decoder.header()?;
    match major {
        0 => Ok(Value::Unsigned(
            argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))?,
        )),
        1 => Ok(Value::Negative(
            -1 - argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))? as i128,
        )),
        2 | 3 => decode_owned_string(decoder, major, argument, at),
        4 => decode_owned_array(decoder, argument, depth, at),
        5 => decode_owned_map(decoder, argument, depth, at),
        6 => {
            let tag = argument.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))?;
            Ok(Value::Tag(
                tag,
                Box::new(decode_owned_value_at(decoder, depth + 1)?),
            ))
        }
        7 => match initial & 31 {
            20 => Ok(Value::Bool(false)),
            21 => Ok(Value::Bool(true)),
            22 => Ok(Value::Null),
            23 => Ok(Value::Undefined),
            24 if argument.unwrap() >= 32 => Ok(Value::Simple(argument.unwrap() as u8)),
            24 => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            25 => Ok(Value::Float(half_to_f64(argument.unwrap() as u16))),
            26 => Ok(Value::Float(f32::from_bits(argument.unwrap() as u32) as f64)),
            27 => Ok(Value::Float(f64::from_bits(argument.unwrap()))),
            n @ 0..=19 => Ok(Value::Simple(n)),
            31 => Err(Error::new(ErrorKind::UnexpectedBreak, at)),
            _ => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
        },
        _ => unreachable!(),
    }
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_owned_string(
    decoder: &mut SliceDecoder<'_>,
    major: u8,
    length: Option<u64>,
    at: usize,
) -> Result<Value> {
    if let Some(length) = length {
        let length = checked_collection_len(decoder, length, at)?;
        let bytes = decoder.take(length)?;
        return if major == 2 {
            Ok(Value::Bytes(bytes.to_vec()))
        } else {
            Ok(Value::Text(String::from(validated_str(bytes, at)?)))
        };
    }

    if decoder.options.validation == Validation::Deterministic {
        return Err(Error::new(ErrorKind::NonDeterministic, at));
    }
    if major == 2 {
        let mut joined = Vec::new();
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            let chunk_at = decoder.pos;
            let (chunk_major, chunk_length, _) = decoder.header()?;
            if chunk_major != major {
                return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
            }
            let chunk_length =
                chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
            let total = (joined.len() as u64)
                .checked_add(chunk_length)
                .ok_or(Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            checked_collection_len(decoder, total, chunk_at)?;
            let chunk_length = usize::try_from(chunk_length)
                .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            joined.extend_from_slice(decoder.take(chunk_length)?);
        }
        Ok(Value::Bytes(joined))
    } else {
        let mut joined = String::new();
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            let chunk_at = decoder.pos;
            let (chunk_major, chunk_length, _) = decoder.header()?;
            if chunk_major != major {
                return Err(Error::new(ErrorKind::UnexpectedType, chunk_at));
            }
            let chunk_length =
                chunk_length.ok_or(Error::new(ErrorKind::UnexpectedType, chunk_at))?;
            let total = (joined.len() as u64)
                .checked_add(chunk_length)
                .ok_or(Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            checked_collection_len(decoder, total, chunk_at)?;
            let chunk_length = usize::try_from(chunk_length)
                .map_err(|_| Error::new(ErrorKind::CollectionLimit, chunk_at))?;
            joined.push_str(validated_str(decoder.take(chunk_length)?, chunk_at)?);
        }
        Ok(Value::Text(joined))
    }
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_owned_array(
    decoder: &mut SliceDecoder<'_>,
    length: Option<u64>,
    depth: usize,
    at: usize,
) -> Result<Value> {
    let mut values = match length {
        Some(length) => Vec::with_capacity(checked_collection_len(decoder, length, at)?),
        None => {
            if decoder.options.validation == Validation::Deterministic {
                return Err(Error::new(ErrorKind::NonDeterministic, at));
            }
            Vec::new()
        }
    };

    if let Some(length) = length {
        for _ in 0..length {
            values.push(decode_owned_value_at(decoder, depth + 1)?);
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if values.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            values.push(decode_owned_value_at(decoder, depth + 1)?);
        }
    }
    Ok(Value::Array(values))
}

#[cfg(feature = "alloc")]
#[inline(never)]
fn decode_owned_map(
    decoder: &mut SliceDecoder<'_>,
    length: Option<u64>,
    depth: usize,
    at: usize,
) -> Result<Value> {
    let mut entries = match length {
        Some(length) => Vec::with_capacity(checked_collection_len(decoder, length, at)?),
        None => {
            if decoder.options.validation == Validation::Deterministic {
                return Err(Error::new(ErrorKind::NonDeterministic, at));
            }
            Vec::new()
        }
    };

    if let Some(length) = length {
        for _ in 0..length {
            let key = decode_owned_value_at(decoder, depth + 1)?;
            let value = decode_owned_value_at(decoder, depth + 1)?;
            entries.push((key, value));
        }
    } else {
        loop {
            if decoder.peek()? == 0xff {
                decoder.pos += 1;
                break;
            }
            if entries.len() == decoder.options.max_collection_len {
                return Err(Error::new(ErrorKind::CollectionLimit, at));
            }
            let key = decode_owned_value_at(decoder, depth + 1)?;
            if decoder.peek()? == 0xff {
                return Err(Error::new(ErrorKind::UnexpectedBreak, decoder.pos));
            }
            let value = decode_owned_value_at(decoder, depth + 1)?;
            entries.push((key, value));
        }
    }
    Ok(Value::Map(entries))
}

/// A streaming iterator over the tokens in CBOR input.
pub struct Parser<'de> {
    decoder: SliceDecoder<'de>,
}
impl<'de> Parser<'de> {
    /// Creates a parser using [`DecodeOptions::default`].
    pub fn new(input: &'de [u8]) -> Self {
        Self {
            decoder: SliceDecoder::new(input),
        }
    }
    /// Creates a parser with explicit decoding options.
    pub fn with_options(input: &'de [u8], options: DecodeOptions) -> Self {
        Self {
            decoder: SliceDecoder::with_options(input, options),
        }
    }
    /// Returns the byte offset of the next event.
    pub fn position(&self) -> usize {
        self.decoder.position()
    }
    /// Returns the input bytes not yet consumed by the parser.
    pub fn remaining(&self) -> &'de [u8] {
        self.decoder.remaining()
    }
    #[cfg(feature = "serde")]
    pub(crate) fn peek_initial(&self) -> Result<u8> {
        self.decoder.peek()
    }
    #[cfg(feature = "serde")]
    pub(crate) fn read_float(&mut self, initial: u8) -> Result<f64> {
        if self.decoder.options.validation == Validation::Deterministic {
            self.decoder.float()
        } else {
            self.decoder.float_basic(initial)
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_f32(&mut self) -> Result<f32> {
        if self.decoder.options.validation == Validation::Deterministic {
            Ok(self.decoder.float()? as f32)
        } else {
            self.decoder.f32_basic()
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_unsigned(&mut self, initial: u8) -> Result<u64> {
        if self.decoder.options.validation == Validation::Deterministic {
            self.decoder.unsigned()
        } else {
            self.decoder.unsigned_basic(initial)
        }
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_i64(&mut self, initial: u8) -> Result<i64> {
        self.decoder.integer_i64_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_i32(&mut self, initial: u8) -> Result<i32> {
        self.decoder.integer_i32_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_i16(&mut self, initial: u8) -> Result<i16> {
        self.decoder.integer_i16_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_i8(&mut self, initial: u8) -> Result<i8> {
        self.decoder.integer_i8_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_bool(&mut self, initial: u8) -> Result<bool> {
        self.decoder.bool_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn consume_one(&mut self) {
        self.decoder.pos += 1;
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_u8_one_byte(&mut self) -> Result<u8> {
        self.decoder.unsigned_u8_one_byte()
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_u8(&mut self, initial: u8) -> Result<u8> {
        self.decoder.unsigned_u8_basic(initial)
    }
    #[cfg(feature = "serde")]
    #[inline(always)]
    pub(crate) fn read_collection(
        &mut self,
        initial: u8,
        expected_major: u8,
    ) -> Result<Option<u64>> {
        if self.decoder.options.validation == Validation::Deterministic {
            let (major, length, at) = self.decoder.header()?;
            if major != expected_major {
                return Err(Error::new(ErrorKind::UnexpectedType, at));
            }
            Ok(length)
        } else {
            self.decoder.collection_basic(initial, expected_major)
        }
    }
    #[cfg(feature = "serde")]
    pub(crate) fn read_text(&mut self, initial: u8) -> Result<&'de str> {
        self.decoder.text_basic(initial)
    }
    #[cfg(feature = "serde")]
    pub(crate) fn read_bytes(&mut self, initial: u8) -> Result<&'de [u8]> {
        self.decoder.bytes_basic(initial, 2)
    }
    #[cfg(feature = "serde")]
    pub(crate) fn skip_item(&mut self) -> Result<()> {
        self.decoder.skip()
    }
    #[cfg(feature = "serde")]
    pub(crate) fn is_deterministic(&self) -> bool {
        self.decoder.options.validation == Validation::Deterministic
    }
    #[cfg(feature = "serde")]
    pub(crate) fn raw_range(&self, start: usize, end: usize) -> &'de [u8] {
        &self.decoder.input[start..end]
    }
}
impl<'de> Iterator for Parser<'de> {
    type Item = Result<Event<'de>>;
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.decoder.pos == self.decoder.input.len() {
            return None;
        }
        Some(parse_event(&mut self.decoder))
    }
}

#[inline]
fn parse_event<'de>(d: &mut SliceDecoder<'de>) -> Result<Event<'de>> {
    let initial = d.peek()?;
    if initial == 0xff {
        d.pos += 1;
        return Ok(Event::Break);
    }
    let (major, arg, at) = d.header()?;
    match major {
        0 => Ok(Event::Unsigned(arg.unwrap())),
        1 => Ok(Event::Negative(-1 - arg.unwrap() as i128)),
        2 | 3 => {
            let Some(n) = arg else {
                return Ok(if major == 2 {
                    Event::IndefiniteBytes
                } else {
                    Event::IndefiniteText
                });
            };
            let n = n as usize;
            let bytes = d.take(n)?;
            if major == 2 {
                Ok(Event::Bytes(bytes))
            } else {
                Ok(Event::Text(validated_str(bytes, at)?))
            }
        }
        4 => Ok(Event::Array(arg)),
        5 => Ok(Event::Map(arg)),
        6 => Ok(Event::Tag(
            arg.ok_or(Error::new(ErrorKind::InvalidAdditionalInfo, at))?,
        )),
        7 => {
            if let Some(value) = arg {
                d.validate_deterministic_float(initial & 31, value, at)?;
            }
            match initial & 31 {
                20 => Ok(Event::Bool(false)),
                21 => Ok(Event::Bool(true)),
                22 => Ok(Event::Null),
                23 => Ok(Event::Undefined),
                24 if arg.unwrap() >= 32 => Ok(Event::Simple(arg.unwrap() as u8)),
                24 => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
                25 => Ok(Event::Float(half_to_f64(arg.unwrap() as u16))),
                26 => Ok(Event::Float(f32::from_bits(arg.unwrap() as u32) as f64)),
                27 => Ok(Event::Float(f64::from_bits(arg.unwrap()))),
                n @ 0..=19 => Ok(Event::Simple(n)),
                _ => Err(Error::new(ErrorKind::InvalidAdditionalInfo, at)),
            }
        }
        _ => unreachable!(),
    }
}

fn half_to_f64(bits: u16) -> f64 {
    let sign = ((bits as u32) & 0x8000) << 16;
    let exponent = ((bits >> 10) & 0x1f) as u32;
    let fraction = (bits & 0x03ff) as u32;
    let float_bits = match exponent {
        0 if fraction == 0 => sign,
        0 => {
            let shift = fraction.leading_zeros() - 21;
            let normalized = (fraction << shift) & 0x03ff;
            let exponent = 113 - shift;
            sign | (exponent << 23) | (normalized << 13)
        }
        31 => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(float_bits) as f64
}

/// An iterator over the raw items in a CBOR sequence.
pub struct SequenceDecoder<'de> {
    decoder: SliceDecoder<'de>,
    index: usize,
}
impl<'de> SequenceDecoder<'de> {
    /// Creates a sequence decoder using [`DecodeOptions::default`].
    pub fn new(input: &'de [u8]) -> Self {
        Self {
            decoder: SliceDecoder::new(input),
            index: 0,
        }
    }
    /// Creates a sequence decoder with explicit decoding options.
    pub fn with_options(input: &'de [u8], options: DecodeOptions) -> Self {
        Self {
            decoder: SliceDecoder::with_options(input, options),
            index: 0,
        }
    }
}
impl<'de> Iterator for SequenceDecoder<'de> {
    type Item = Result<RawValue<'de>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.decoder.pos == self.decoder.input.len() {
            return None;
        }
        let result = if is_complete_one_byte_item(self.decoder.input[self.decoder.pos]) {
            let start = self.decoder.pos;
            self.decoder.pos += 1;
            Ok(RawValue(&self.decoder.input[start..self.decoder.pos]))
        } else {
            self.decoder.raw()
        }
        .map_err(|error| error.with_item(self.index));
        match result {
            Ok(raw) => {
                self.index += 1;
                Some(Ok(raw))
            }
            Err(e) => {
                self.decoder.pos = self.decoder.input.len();
                Some(Err(e))
            }
        }
    }
}
