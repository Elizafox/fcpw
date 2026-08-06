use crate::{Error, ErrorKind, Result};

/// A destination that accepts encoded CBOR bytes.
pub trait Output {
    /// Appends all of `bytes` or returns an error if the destination cannot.
    fn write_all(&mut self, bytes: &[u8]) -> Result<()>;
}

#[cfg(feature = "alloc")]
impl Output for alloc::vec::Vec<u8> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        self.extend_from_slice(bytes);
        Ok(())
    }
}

impl<T: Output + ?Sized> Output for &mut T {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        (**self).write_all(bytes)
    }
}

/// A fixed-capacity [`Output`] backed by a mutable byte slice.
pub struct SliceOutput<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl<'a> SliceOutput<'a> {
    /// Creates an empty output that writes into `buf`.
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0 }
    }

    /// Returns the number of bytes written.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether no bytes have been written.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Output for SliceOutput<'_> {
    fn write_all(&mut self, bytes: &[u8]) -> Result<()> {
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(Error::new(ErrorKind::OutputTooSmall, self.len))?;
        let dst = self
            .buf
            .get_mut(self.len..end)
            .ok_or(Error::new(ErrorKind::OutputTooSmall, self.len))?;
        dst.copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
}

/// A low-level encoder for writing individual CBOR data items.
pub struct Encoder<W> {
    output: W,
}

impl<W: Output> Encoder<W> {
    /// Creates an encoder writing to `output`.
    pub fn new(output: W) -> Self {
        Self { output }
    }

    /// Finishes encoding and returns the output destination.
    pub fn into_inner(self) -> W {
        self.output
    }

    fn head(&mut self, major: u8, n: u64) -> Result<()> {
        let mut b = [0u8; 9];
        let len = if n < 24 {
            b[0] = major << 5 | n as u8;
            1
        } else if n <= u8::MAX as u64 {
            b[0] = major << 5 | 24;
            b[1] = n as u8;
            2
        } else if n <= u16::MAX as u64 {
            b[0] = major << 5 | 25;
            b[1..3].copy_from_slice(&(n as u16).to_be_bytes());
            3
        } else if n <= u32::MAX as u64 {
            b[0] = major << 5 | 26;
            b[1..5].copy_from_slice(&(n as u32).to_be_bytes());
            5
        } else {
            b[0] = major << 5 | 27;
            b[1..9].copy_from_slice(&n.to_be_bytes());
            9
        };
        self.output.write_all(&b[..len])
    }

    /// Encodes an unsigned integer.
    pub fn unsigned(&mut self, n: u64) -> Result<()> {
        self.head(0, n)
    }

    /// Encodes a negative integer, rejecting nonnegative values.
    pub fn negative(&mut self, n: i64) -> Result<()> {
        if n >= 0 {
            return Err(Error::new(ErrorKind::UnexpectedType, 0));
        }
        self.head(1, !n as u64)
    }

    /// Encodes a signed integer.
    #[inline]
    pub fn signed(&mut self, n: i64) -> Result<()> {
        if n >= 0 {
            self.head(0, n as u64)
        } else {
            self.head(1, !n as u64)
        }
    }

    /// Encodes an integer representable by CBOR major types 0 or 1.
    pub fn integer(&mut self, n: i128) -> Result<()> {
        if n >= 0 {
            self.unsigned(u64::try_from(n).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?)
        } else {
            let arg = (-1i128)
                .checked_sub(n)
                .ok_or(Error::new(ErrorKind::IntegerOverflow, 0))?;
            self.head(
                1,
                u64::try_from(arg).map_err(|_| Error::new(ErrorKind::IntegerOverflow, 0))?,
            )
        }
    }

    /// Encodes a definite-length byte string.
    pub fn bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.head(2, bytes.len() as u64)?;
        self.output.write_all(bytes)
    }

    /// Encodes a definite-length UTF-8 text string.
    pub fn text(&mut self, text: &str) -> Result<()> {
        self.head(3, text.len() as u64)?;
        self.output.write_all(text.as_bytes())
    }

    /// Encodes the header of a definite-length array.
    pub fn array(&mut self, len: usize) -> Result<()> {
        self.head(4, len as u64)
    }

    /// Encodes the header of a definite-length map.
    pub fn map(&mut self, len: usize) -> Result<()> {
        self.head(5, len as u64)
    }

    /// Encodes a semantic tag.
    pub fn tag(&mut self, tag: u64) -> Result<()> {
        self.head(6, tag)
    }

    /// Encodes an unassigned simple value.
    pub fn simple(&mut self, value: u8) -> Result<()> {
        if value < 20 {
            self.output.write_all(&[0xe0 | value])
        } else if value < 32 {
            Err(Error::new(ErrorKind::InvalidAdditionalInfo, 0))
        } else {
            self.output.write_all(&[0xf8, value])
        }
    }

    /// Encodes a Boolean value.
    pub fn bool(&mut self, value: bool) -> Result<()> {
        self.output.write_all(&[if value { 0xf5 } else { 0xf4 }])
    }

    /// Encodes the null value.
    pub fn null(&mut self) -> Result<()> {
        self.output.write_all(&[0xf6])
    }

    /// Encodes the undefined value.
    pub fn undefined(&mut self) -> Result<()> {
        self.output.write_all(&[0xf7])
    }

    /// Encodes `value` at its source binary32 width without narrowing it.
    pub fn f32(&mut self, value: f32) -> Result<()> {
        let bytes = value.to_bits().to_be_bytes();
        self.output
            .write_all(&[0xfa, bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    /// Encodes `value` at its source binary64 width without narrowing it.
    pub fn f64(&mut self, value: f64) -> Result<()> {
        let bytes = value.to_bits().to_be_bytes();
        self.output.write_all(&[
            0xfb, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    /// Encodes `value` at the shortest exact CBOR float width and canonicalizes NaN.
    pub fn f32_preferred(&mut self, value: f32) -> Result<()> {
        self.f64_preferred(value as f64)
    }

    /// Encodes `value` at the shortest exact CBOR float width and canonicalizes NaN.
    pub fn f64_preferred(&mut self, value: f64) -> Result<()> {
        if value.is_nan() {
            return self.output.write_all(&[0xf9, 0x7e, 0x00]);
        }
        let narrowed = value as f32;
        if narrowed as f64 == value {
            if let Some(half) = exact_half(narrowed) {
                let bytes = half.to_be_bytes();
                return self.output.write_all(&[0xf9, bytes[0], bytes[1]]);
            }
            let bytes = narrowed.to_bits().to_be_bytes();
            return self
                .output
                .write_all(&[0xfa, bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        let bytes = value.to_bits().to_be_bytes();
        self.output.write_all(&[
            0xfb, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    /// Writes already encoded CBOR bytes without validation.
    pub fn raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.output.write_all(bytes)
    }
}

pub(crate) fn exact_half(value: f32) -> Option<u16> {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let fraction = bits & 0x7f_ffff;
    let half = if exponent == 255 {
        if fraction == 0 {
            sign | 0x7c00
        } else {
            sign | 0x7e00
        }
    } else {
        let half_exponent = exponent - 127 + 15;
        if half_exponent >= 31 {
            return None;
        } else if half_exponent <= 0 {
            if exponent == 0 && fraction == 0 {
                sign
            } else if half_exponent < -10 {
                return None;
            } else {
                let mantissa = fraction | 0x80_0000;
                let shift = 14 - half_exponent;
                if mantissa & ((1 << shift) - 1) != 0 {
                    return None;
                }
                sign | (mantissa >> shift) as u16
            }
        } else {
            if fraction & 0x1fff != 0 {
                return None;
            }
            sign | ((half_exponent as u16) << 10) | (fraction >> 13) as u16
        }
    };

    Some(half)
}
