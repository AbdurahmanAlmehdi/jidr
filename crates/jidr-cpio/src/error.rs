/// Everything that can go wrong while reading, editing or writing a ramdisk.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("truncated input at byte {offset}: expected {what}")]
    Truncated { offset: usize, what: &'static str },

    #[error("bad cpio magic at byte {offset} (expected 070701 or 070702)")]
    BadMagic { offset: usize },

    #[error("invalid hex in cpio header field `{field}` at byte {offset}")]
    BadHeader { offset: usize, field: &'static str },

    #[error("cpio entry name at byte {offset} is not NUL-terminated UTF-8")]
    BadName { offset: usize },

    #[error("cpio archive ends without a TRAILER!!! entry")]
    MissingTrailer,

    #[error("checksum mismatch for cpio entry `{name}`")]
    Checksum { name: String },

    #[error("`{name}` is too large for a cpio entry (limit 4 GiB)")]
    TooLarge { name: String },

    #[error("cpio entry `{0}` not found")]
    NotFound(String),

    #[error("cpio entry `{0}` already exists")]
    Exists(String),

    #[error("unknown compression format at byte {offset}")]
    UnknownFormat { offset: usize },

    #[error("{format} data is corrupt: {message}")]
    Corrupt {
        format: &'static str,
        message: String,
    },

    #[error("decompressed data exceeds the {limit}-byte limit")]
    Limit { limit: usize },

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
