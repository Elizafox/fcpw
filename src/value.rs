//! Dynamic CBOR values and conversions.
//!
//! ```
//! use fcpw::Value;
//!
//! let mut value = Value::from(vec![Value::from("first")]);
//! value.as_array_mut().unwrap().push(Value::from(2_u8));
//! assert_eq!(value.as_array().unwrap()[0].as_text(), Some("first"));
//! assert_eq!(u16::try_from(value.into_array().unwrap()[1].clone()).unwrap(), 2);
//! ```
//!
//! Rust integers through `u64` and `i64` convert to the native CBOR integer
//! variants. Fallible conversions back to Rust integer types are range checked:
//! a numeric value outside the destination range reports
//! [`ErrorKind::IntegerOverflow`](crate::ErrorKind::IntegerOverflow), while a
//! non-integer reports [`ErrorKind::UnexpectedType`](crate::ErrorKind::UnexpectedType).
//! Semantic tags 2 and 3 are not implicitly interpreted as bignums.

use alloc::{borrow::Cow, string::String, vec::Vec};

use crate::{Encoder, Error, ErrorKind, Output, Result};

#[cfg(feature = "serde")]
pub(crate) const VALUE_MARKER: &str = "$fcpw::Value";

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// A dynamically typed CBOR value that borrows strings when possible.
pub enum BorrowedValue<'de> {
    /// An unsigned integer.
    Unsigned(u64),
    /// A negative integer.
    Negative(i128),
    /// A byte string, borrowed for definite-length input when possible.
    Bytes(Cow<'de, [u8]>),
    /// A text string, borrowed for definite-length input when possible.
    Text(Cow<'de, str>),
    /// An array of values.
    Array(Vec<Self>),
    /// An ordered collection of key-value pairs.
    Map(Vec<(Self, Self)>),
    /// A semantic tag and its tagged value.
    Tag(u64, alloc::boxed::Box<Self>),
    /// An unassigned simple value.
    Simple(u8),
    /// A Boolean value.
    Bool(bool),
    /// The null value.
    Null,
    /// The undefined value.
    Undefined,
    /// A floating-point value represented as binary64.
    Float(f64),
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
/// An owned, dynamically typed CBOR value.
pub enum Value {
    /// An unsigned integer.
    Unsigned(u64),
    /// A negative integer.
    Negative(i128),
    /// A byte string.
    Bytes(Vec<u8>),
    /// A UTF-8 text string.
    Text(String),
    /// An array of values.
    Array(Vec<Self>),
    /// An ordered collection of key-value pairs.
    Map(Vec<(Self, Self)>),
    /// A semantic tag and its tagged value.
    Tag(u64, alloc::boxed::Box<Self>),
    /// An unassigned simple value.
    Simple(u8),
    /// A Boolean value.
    Bool(bool),
    /// The null value.
    Null,
    /// The undefined value.
    Undefined,
    /// A floating-point value represented as binary64.
    Float(f64),
}

impl<'de> BorrowedValue<'de> {
    /// Decodes exactly one dynamic value, borrowing from `input` when possible.
    pub fn decode(input: &'de [u8]) -> Result<Self> {
        crate::decode::decode_borrowed_value(input)
    }
}

impl From<BorrowedValue<'_>> for Value {
    fn from(v: BorrowedValue<'_>) -> Self {
        match v {
            BorrowedValue::Unsigned(v) => Self::Unsigned(v),
            BorrowedValue::Negative(v) => Self::Negative(v),
            BorrowedValue::Bytes(v) => Self::Bytes(v.into_owned()),
            BorrowedValue::Text(v) => Self::Text(v.into_owned()),
            BorrowedValue::Array(v) => Self::Array(v.into_iter().map(Into::into).collect()),
            BorrowedValue::Map(v) => {
                Self::Map(v.into_iter().map(|(k, v)| (k.into(), v.into())).collect())
            }
            BorrowedValue::Tag(t, v) => Self::Tag(t, alloc::boxed::Box::new((*v).into())),
            BorrowedValue::Simple(v) => Self::Simple(v),
            BorrowedValue::Bool(v) => Self::Bool(v),
            BorrowedValue::Null => Self::Null,
            BorrowedValue::Undefined => Self::Undefined,
            BorrowedValue::Float(v) => Self::Float(v),
        }
    }
}

impl From<Value> for BorrowedValue<'static> {
    fn from(value: Value) -> Self {
        match value {
            Value::Unsigned(v) => Self::Unsigned(v),
            Value::Negative(v) => Self::Negative(v),
            Value::Bytes(v) => Self::Bytes(Cow::Owned(v)),
            Value::Text(v) => Self::Text(Cow::Owned(v)),
            Value::Array(v) => Self::Array(v.into_iter().map(Into::into).collect()),
            Value::Map(v) => Self::Map(v.into_iter().map(|(k, v)| (k.into(), v.into())).collect()),
            Value::Tag(t, v) => Self::Tag(t, alloc::boxed::Box::new((*v).into())),
            Value::Simple(v) => Self::Simple(v),
            Value::Bool(v) => Self::Bool(v),
            Value::Null => Self::Null,
            Value::Undefined => Self::Undefined,
            Value::Float(v) => Self::Float(v),
        }
    }
}

impl Value {
    /// Returns `true` if this is an [`Unsigned`](Self::Unsigned) value.
    pub const fn is_unsigned(&self) -> bool {
        matches!(self, Self::Unsigned(_))
    }

    /// Returns `true` if this is an unsigned or negative integer.
    ///
    /// Tags 2 and 3 (bignums) are deliberately not treated as integers. Their
    /// interpretation depends on the tagged byte string and must be explicit.
    pub const fn is_integer(&self) -> bool {
        matches!(self, Self::Unsigned(_) | Self::Negative(_))
    }

    /// Returns `true` if this is a byte string.
    pub const fn is_bytes(&self) -> bool {
        matches!(self, Self::Bytes(_))
    }
    /// Returns `true` if this is a text string.
    pub const fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }
    /// Returns `true` if this is an array.
    pub const fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
    /// Returns `true` if this is a map.
    pub const fn is_map(&self) -> bool {
        matches!(self, Self::Map(_))
    }
    /// Returns `true` if this is a tagged value.
    pub const fn is_tag(&self) -> bool {
        matches!(self, Self::Tag(_, _))
    }
    /// Returns `true` if this is an unassigned simple value.
    pub const fn is_simple(&self) -> bool {
        matches!(self, Self::Simple(_))
    }
    /// Returns `true` if this is a Boolean.
    pub const fn is_bool(&self) -> bool {
        matches!(self, Self::Bool(_))
    }
    /// Returns `true` if this is null.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    /// Returns `true` if this is undefined.
    pub const fn is_undefined(&self) -> bool {
        matches!(self, Self::Undefined)
    }
    /// Returns `true` if this is a floating-point value.
    pub const fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    /// Returns the unsigned integer, if present.
    pub const fn as_unsigned(&self) -> Option<u64> {
        if let Self::Unsigned(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    /// Returns the integer as an `i128`, if present and representable.
    ///
    /// This accepts both native integer variants but not bignum tags.
    pub fn as_integer(&self) -> Option<i128> {
        match self {
            Self::Unsigned(value) => Some(i128::from(*value)),
            Self::Negative(value) => Some(*value),
            _ => None,
        }
    }

    /// Borrows the byte string, if present.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        if let Self::Bytes(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Mutably borrows the byte string, if present.
    pub fn as_bytes_mut(&mut self) -> Option<&mut Vec<u8>> {
        if let Self::Bytes(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Borrows the text string, if present.
    pub fn as_text(&self) -> Option<&str> {
        if let Self::Text(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Mutably borrows the text string, if present.
    pub fn as_text_mut(&mut self) -> Option<&mut String> {
        if let Self::Text(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Borrows the array, if present.
    pub fn as_array(&self) -> Option<&[Self]> {
        if let Self::Array(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Mutably borrows the array, if present.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Self>> {
        if let Self::Array(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Borrows the ordered map entries, if present.
    ///
    /// The slice retains insertion/wire order and may contain duplicate keys.
    pub fn as_map(&self) -> Option<&[(Self, Self)]> {
        if let Self::Map(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Mutably borrows the ordered map entries, if present.
    pub fn as_map_mut(&mut self) -> Option<&mut Vec<(Self, Self)>> {
        if let Self::Map(value) = self {
            Some(value)
        } else {
            None
        }
    }
    /// Returns the tag number and tagged value, if present.
    pub fn as_tag(&self) -> Option<(u64, &Self)> {
        if let Self::Tag(tag, value) = self {
            Some((*tag, value))
        } else {
            None
        }
    }
    /// Returns the simple value, if present.
    pub const fn as_simple(&self) -> Option<u8> {
        if let Self::Simple(value) = self {
            Some(*value)
        } else {
            None
        }
    }
    /// Returns the Boolean, if present.
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(value) = self {
            Some(*value)
        } else {
            None
        }
    }
    /// Returns the floating-point value, if present.
    pub const fn as_float(&self) -> Option<f64> {
        if let Self::Float(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    /// Extracts the byte string without cloning, or returns the original value.
    pub fn into_bytes(self) -> core::result::Result<Vec<u8>, Self> {
        if let Self::Bytes(value) = self {
            Ok(value)
        } else {
            Err(self)
        }
    }
    /// Extracts the text string without cloning, or returns the original value.
    pub fn into_text(self) -> core::result::Result<String, Self> {
        if let Self::Text(value) = self {
            Ok(value)
        } else {
            Err(self)
        }
    }
    /// Extracts the array without cloning, or returns the original value.
    pub fn into_array(self) -> core::result::Result<Vec<Self>, Self> {
        if let Self::Array(value) = self {
            Ok(value)
        } else {
            Err(self)
        }
    }
    /// Extracts the ordered map entries without cloning, or returns the original value.
    pub fn into_map(self) -> core::result::Result<Vec<(Self, Self)>, Self> {
        if let Self::Map(value) = self {
            Ok(value)
        } else {
            Err(self)
        }
    }

    /// Encodes this value through a low-level [`Encoder`].
    pub fn encode<W: Output>(&self, e: &mut Encoder<W>) -> Result<()> {
        match self {
            Self::Unsigned(v) => e.unsigned(*v),
            Self::Negative(v) => e.integer(*v),
            Self::Bytes(v) => e.bytes(v),
            Self::Text(v) => e.text(v),
            Self::Array(v) => {
                e.array(v.len())?;
                for x in v {
                    x.encode(e)?;
                }
                Ok(())
            }
            Self::Map(v) => {
                e.map(v.len())?;
                for (k, x) in v {
                    k.encode(e)?;
                    x.encode(e)?;
                }
                Ok(())
            }
            Self::Tag(t, v) => {
                e.tag(*t)?;
                v.encode(e)
            }
            Self::Simple(v) => e.simple(*v),
            Self::Bool(v) => e.bool(*v),
            Self::Null => e.null(),
            Self::Undefined => e.undefined(),
            Self::Float(v) => e.f64(*v),
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}
impl From<()> for Value {
    fn from((): ()) -> Self {
        Self::Null
    }
}
impl From<f32> for Value {
    fn from(value: f32) -> Self {
        Self::Float(value.into())
    }
}
impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}
impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}
impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::Text(value.into())
    }
}
impl From<&[u8]> for Value {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.into())
    }
}
impl From<Vec<u8>> for Value {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}
impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}
impl From<Vec<(Value, Value)>> for Value {
    fn from(value: Vec<(Value, Value)>) -> Self {
        Self::Map(value)
    }
}

macro_rules! from_unsigned {
    ($($ty:ty),+ $(,)?) => {$(
        impl From<$ty> for Value {
            fn from(value: $ty) -> Self { Self::Unsigned(value.into()) }
        }
    )+};
}
from_unsigned!(u8, u16, u32, u64);

macro_rules! from_signed {
    ($($ty:ty),+ $(,)?) => {$(
        impl From<$ty> for Value {
            fn from(value: $ty) -> Self {
                if value >= 0 { Self::Unsigned(value as u64) } else { Self::Negative(value as i128) }
            }
        }
    )+};
}
from_signed!(i8, i16, i32, i64);

fn unexpected_type() -> Error {
    Error::new(ErrorKind::UnexpectedType, 0)
}
fn integer_overflow() -> Error {
    Error::new(ErrorKind::IntegerOverflow, 0)
}

impl TryFrom<Value> for bool {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        if let Value::Bool(value) = value {
            Ok(value)
        } else {
            Err(unexpected_type())
        }
    }
}
impl TryFrom<Value> for f64 {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        if let Value::Float(value) = value {
            Ok(value)
        } else {
            Err(unexpected_type())
        }
    }
}
impl TryFrom<Value> for String {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        value.into_text().map_err(|_| unexpected_type())
    }
}
impl TryFrom<Value> for Vec<u8> {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        value.into_bytes().map_err(|_| unexpected_type())
    }
}
impl TryFrom<Value> for Vec<Value> {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        value.into_array().map_err(|_| unexpected_type())
    }
}
impl TryFrom<Value> for Vec<(Value, Value)> {
    type Error = Error;
    fn try_from(value: Value) -> Result<Self> {
        value.into_map().map_err(|_| unexpected_type())
    }
}

macro_rules! try_into_unsigned {
    ($($ty:ty),+ $(,)?) => {$(
        impl TryFrom<Value> for $ty {
            type Error = Error;
            fn try_from(value: Value) -> Result<Self> {
                match value {
                    Value::Unsigned(value) => <$ty>::try_from(value).map_err(|_| integer_overflow()),
                    Value::Negative(_) => Err(integer_overflow()),
                    _ => Err(unexpected_type()),
                }
            }
        }
    )+};
}
try_into_unsigned!(u8, u16, u32, u64, u128);

macro_rules! try_into_signed {
    ($($ty:ty),+ $(,)?) => {$(
        impl TryFrom<Value> for $ty {
            type Error = Error;
            fn try_from(value: Value) -> Result<Self> {
                match value {
                    Value::Unsigned(value) => <$ty>::try_from(value).map_err(|_| integer_overflow()),
                    Value::Negative(value) => <$ty>::try_from(value).map_err(|_| integer_overflow()),
                    _ => Err(unexpected_type()),
                }
            }
        }
    )+};
}
try_into_signed!(i8, i16, i32, i64, i128);

#[cfg(feature = "serde")]
impl serde::Serialize for Value {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> core::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        match self {
            Self::Unsigned(value) => serializer.serialize_u64(*value),
            Self::Negative(value) => serializer.serialize_i128(*value),
            Self::Bytes(value) => serializer.serialize_bytes(value),
            Self::Text(value) => serializer.serialize_str(value),
            Self::Array(value) => value.serialize(serializer),
            Self::Map(value) => {
                use serde::ser::SerializeMap as _;
                let mut map = serializer.serialize_map(Some(value.len()))?;
                for (key, value) in value {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Null => serializer.serialize_unit(),
            Self::Float(value) => serializer.serialize_f64(*value),
            // These values have no lossless representation in Serde's data model.
            // FCPW serializers recognize the marker and copy the enclosed CBOR item.
            Self::Tag(_, _) | Self::Simple(_) | Self::Undefined => {
                let mut bytes = alloc::vec::Vec::new();
                self.encode(&mut Encoder::new(&mut bytes))
                    .map_err(S::Error::custom)?;
                serializer.serialize_newtype_struct(VALUE_MARKER, &serde_bytes::Bytes::new(&bytes))
            }
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for BorrowedValue<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> core::result::Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        match self {
            Self::Unsigned(value) => serializer.serialize_u64(*value),
            Self::Negative(value) => serializer.serialize_i128(*value),
            Self::Bytes(value) => serializer.serialize_bytes(value),
            Self::Text(value) => serializer.serialize_str(value),
            Self::Array(value) => value.serialize(serializer),
            Self::Map(value) => {
                use serde::ser::SerializeMap as _;
                let mut map = serializer.serialize_map(Some(value.len()))?;
                for (key, value) in value {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Null => serializer.serialize_unit(),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::Tag(_, _) | Self::Simple(_) | Self::Undefined => {
                let mut bytes = alloc::vec::Vec::new();
                Value::from(self.clone())
                    .encode(&mut Encoder::new(&mut bytes))
                    .map_err(S::Error::custom)?;
                serializer.serialize_newtype_struct(VALUE_MARKER, &serde_bytes::Bytes::new(&bytes))
            }
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Value {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> core::result::Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(VALUE_MARKER, ValueVisitor)
    }
}

#[cfg(feature = "serde")]
struct ValueVisitor;

#[cfg(feature = "serde")]
impl<'de> serde::de::Visitor<'de> for ValueVisitor {
    type Value = Value;
    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a CBOR value")
    }
    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> core::result::Result<Value, D::Error> {
        struct BytesVisitor;
        impl<'de> serde::de::Visitor<'de> for BytesVisitor {
            type Value = Value;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("an encoded CBOR value")
            }
            fn visit_borrowed_bytes<E: serde::de::Error>(
                self,
                bytes: &'de [u8],
            ) -> core::result::Result<Value, E> {
                crate::decode::decode_owned_value(bytes).map_err(E::custom)
            }
            fn visit_bytes<E: serde::de::Error>(
                self,
                bytes: &[u8],
            ) -> core::result::Result<Value, E> {
                crate::decode::decode_owned_value(bytes).map_err(E::custom)
            }
            fn visit_byte_buf<E: serde::de::Error>(
                self,
                bytes: Vec<u8>,
            ) -> core::result::Result<Value, E> {
                crate::decode::decode_owned_value(&bytes).map_err(E::custom)
            }
        }
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

#[cfg(feature = "serde")]
/// Converts a serializable value into a dynamic CBOR value without wire encoding.
pub fn to_value<T: serde::Serialize + ?Sized>(value: &T) -> Result<Value> {
    value.serialize(crate::serde_codec::ValueSerializer)
}

#[cfg(feature = "serde")]
/// Converts a dynamic CBOR value into an owned Serde value without wire encoding.
pub fn from_value<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    T::deserialize(BorrowedValue::from(value))
}
