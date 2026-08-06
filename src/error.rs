use core::fmt;

/// The result type returned by this crate.
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
/// The category of a CBOR encoding or decoding error.
pub enum ErrorKind {
    /// The input ended before the current item was complete.
    Eof,

    /// An initial byte contained reserved or invalid additional information.
    InvalidAdditionalInfo,

    /// The encountered CBOR type did not match the requested type.
    UnexpectedType,

    /// A break code appeared outside its permitted position.
    UnexpectedBreak,

    /// A text string was not valid UTF-8.
    InvalidUtf8,

    /// An integer cannot be represented by the requested Rust type.
    IntegerOverflow,

    /// The configured nesting-depth limit was exceeded.
    DepthLimit,

    /// The configured collection-length limit was exceeded.
    CollectionLimit,

    /// Bytes remain after the expected item.
    TrailingData,

    /// Input violated deterministic CBOR encoding requirements.
    NonDeterministic,

    /// A deterministic map contained duplicate encoded keys.
    DuplicateKey,

    /// The provided output buffer lacks sufficient capacity.
    OutputTooSmall,

    /// An underlying reader or writer reported an I/O error.
    #[cfg(feature = "std")]
    Io,

    /// An underlying serializer reported an opaque error.
    Message,
}

#[cfg_attr(not(feature = "std"), derive(Clone, Copy, Eq, PartialEq))]
#[derive(Debug)]
/// An error with its category and byte offset.
pub struct Error {
    kind: ErrorKind,
    offset: usize,
    item: Option<usize>,
    #[cfg(feature = "std")]
    io: Option<std::io::Error>,
}

impl Error {
    /// Creates an error of `kind` at the given byte `offset`.
    pub const fn new(kind: ErrorKind, offset: usize) -> Self {
        Self {
            kind,
            offset,
            item: None,
            #[cfg(feature = "std")]
            io: None,
        }
    }

    /// Returns this error's category.
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the byte offset at which the error was detected.
    pub const fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the zero-based sequence item index, when available.
    pub const fn item_index(&self) -> Option<usize> {
        self.item
    }

    /// Returns the underlying I/O error, when this error came from a reader or writer.
    #[cfg(feature = "std")]
    pub fn io_error(&self) -> Option<&std::io::Error> {
        self.io.as_ref()
    }

    #[cfg(feature = "std")]
    pub(crate) fn from_io(error: std::io::Error, offset: usize) -> Self {
        Self {
            kind: ErrorKind::Io,
            offset,
            item: None,
            io: Some(error),
        }
    }
    pub(crate) const fn with_item(mut self, item: usize) -> Self {
        self.item = Some(item);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(feature = "std")]
        if let Some(error) = &self.io {
            write!(f, "I/O error at byte {}: {error}", self.offset)?;
        } else {
            write!(f, "{:?} at byte {}", self.kind, self.offset)?;
        }
        #[cfg(not(feature = "std"))]
        write!(f, "{:?} at byte {}", self.kind, self.offset)?;
        if let Some(item) = self.item {
            write!(f, " (sequence item {item})")?;
        }
        Ok(())
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.io
            .as_ref()
            .map(|error| error as &(dyn std::error::Error + 'static))
    }
}
