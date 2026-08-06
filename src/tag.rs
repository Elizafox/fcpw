//! Typed support for CBOR semantic tags.

use serde::{Deserialize, Serialize};

use crate::{Event, Parser};

/// A Serde value with an optional outer CBOR semantic tag.
///
/// Unknown tag numbers are preserved. When the input is untagged, `tag` is
/// `None`. Nested `Tagged` values preserve tags from outermost to innermost.
#[derive(Clone, Debug, PartialEq)]
pub struct Tagged<T> {
    /// The outer semantic tag, or `None` for an untagged value.
    pub tag: Option<u64>,
    /// The value enclosed by the tag.
    pub value: T,
}

impl<T> Tagged<T> {
    /// Creates a tagged or explicitly untagged value.
    pub const fn new(tag: Option<u64>, value: T) -> Self {
        Self { tag, value }
    }

    /// Creates a value carrying `tag`.
    pub const fn with_tag(tag: u64, value: T) -> Self {
        Self::new(Some(tag), value)
    }
}

impl<T: Serialize> Serialize for Tagged<T> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::Error as _;
        let Some(tag) = self.tag else {
            return self.value.serialize(serializer);
        };

        // Serde has no native tag representation. Encode on the optimized Vec
        // path, prepend the tag header in place, and pass the completed item
        // through the raw-CBOR marker understood by FCPW serializers.
        let mut bytes = crate::to_vec(&self.value).map_err(S::Error::custom)?;
        let mut header = [0; 9];
        let header_len = encode_tag_header(tag, &mut header);
        let payload_len = bytes.len();
        bytes.resize(payload_len + header_len, 0);
        bytes.copy_within(..payload_len, header_len);
        bytes[..header_len].copy_from_slice(&header[..header_len]);
        serializer
            .serialize_newtype_struct(crate::value::VALUE_MARKER, &serde_bytes::Bytes::new(&bytes))
    }
}

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Tagged<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(
            crate::value::VALUE_MARKER,
            TaggedVisitor(core::marker::PhantomData),
        )
    }
}

struct TaggedVisitor<T>(core::marker::PhantomData<T>);

impl<'de, T: serde::de::DeserializeOwned> serde::de::Visitor<'de> for TaggedVisitor<T> {
    type Value = Tagged<T>;

    fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("an encoded CBOR value with an optional semantic tag")
    }

    fn visit_newtype_struct<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_bytes(self)
    }

    fn visit_borrowed_bytes<E: serde::de::Error>(self, bytes: &'de [u8]) -> Result<Self::Value, E> {
        decode_tagged(bytes)
    }

    fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Self::Value, E> {
        decode_tagged(bytes)
    }

    fn visit_byte_buf<E: serde::de::Error>(
        self,
        bytes: alloc::vec::Vec<u8>,
    ) -> Result<Self::Value, E> {
        decode_tagged(&bytes)
    }
}

fn decode_tagged<T: serde::de::DeserializeOwned, E: serde::de::Error>(
    bytes: &[u8],
) -> Result<Tagged<T>, E> {
    let mut parser = Parser::new(bytes);
    let tag = match parser.next().transpose().map_err(E::custom)? {
        Some(Event::Tag(tag)) => Some(tag),
        _ => None,
    };
    let payload = tag.map_or(bytes, |_| parser.remaining());
    crate::from_slice(payload)
        .map(|value| Tagged { tag, value })
        .map_err(E::custom)
}

fn encode_tag_header(tag: u64, output: &mut [u8; 9]) -> usize {
    if tag < 24 {
        output[0] = 0xc0 | tag as u8;
        1
    } else if tag <= u8::MAX as u64 {
        output[0] = 0xd8;
        output[1] = tag as u8;
        2
    } else if tag <= u16::MAX as u64 {
        output[0] = 0xd9;
        output[1..3].copy_from_slice(&(tag as u16).to_be_bytes());
        3
    } else if tag <= u32::MAX as u64 {
        output[0] = 0xda;
        output[1..5].copy_from_slice(&(tag as u32).to_be_bytes());
        5
    } else {
        output[0] = 0xdb;
        output[1..].copy_from_slice(&tag.to_be_bytes());
        9
    }
}
