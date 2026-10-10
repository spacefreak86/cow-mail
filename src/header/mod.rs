//! Header lines.
//!
//! A header is abstracted by the [`Header`] trait, which is generic over the storage so that a
//! message can be parsed into whatever representation the caller needs. [`CowHeader`] is the
//! default implementation: it keeps the raw line — key, separator, value and trailing newline —
//! as a [`Cow`], and becomes owned only when the value is changed.
//!
//! [`HeaderMap`] is the collection side: finding, iterating and editing headers by key. It is
//! implemented for `Vec<H>` and for [`CowMail`](crate::CowMail).

mod encoding;
mod headers;

pub use encoding::encode_header_value;
pub use headers::{HeaderMap, parse_headers};
pub(crate) use headers::build_headers;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt::{self, Display};

use charset::decode_latin1;
use mailparse::{MailHeader, parse_content_type};

use crate::{CowMailError, offset_in};

/// One header line, abstracted over how it is stored.
///
/// A message is parsed into whatever type implements this, which is what makes
/// [`CowMail`](crate::CowMail) generic over its headers. [`CowHeader`] is the implementation this
/// crate provides and the one [`CowMail::try_from`](crate::CowMail::try_from) uses.
///
/// # Implementing
///
/// Six methods carry the storage: [`new`] and [`from`] construct, [`get_key_raw`] /
/// [`get_value_raw`] / [`set_value_raw`] access, and [`serialize`] writes the line back out.
/// Everything else — case-insensitive key comparison, RFC 2047 decoding, parameter handling — is
/// provided on top of those.
///
/// [`from`] is handed the whole line plus the three offsets that cut it up, so an implementation
/// can keep the line verbatim and borrow from it; one that stores key and value separately can
/// slice them out and discard the rest, at the cost of no longer round-tripping the line exactly.
/// An implementation that does keep the line should also override [`get_raw`], which otherwise
/// rebuilds it.
///
/// [`new`]: Self::new
/// [`from`]: Self::from
/// [`get_key_raw`]: Self::get_key_raw
/// [`get_value_raw`]: Self::get_value_raw
/// [`set_value_raw`]: Self::set_value_raw
/// [`serialize`]: Self::serialize
/// [`get_raw`]: Self::get_raw
pub trait Header<'a>: Clone + Display {
    /// A new header from a key and a value.
    ///
    /// The value is RFC 2047-encoded and folded as needed — see [`encode_header_value`] — so pass
    /// it as the text it should decode to, not pre-encoded.
    ///
    /// # Panics
    ///
    /// If `key` or `value` is empty.
    fn new<K: ToString, V: ToString>(key: K, value: V) -> Self;

    /// A header from a line that was already parsed.
    ///
    /// `raw_bytes` is the whole line, trailing newline and any folded continuation lines included.
    /// The offsets index into it: the key is `..key_end`, the value `value_start..value_end`. What
    /// lies between them is the `:` and the whitespace after it, and what follows `value_end` is
    /// the line ending.
    fn from(raw_bytes: &'a [u8], key_end: usize, value_start: usize, value_end: usize) -> Self;

    /// The key as it appears in the message, undecoded.
    fn get_key_raw(&self) -> &[u8];

    /// The value as it appears in the message: still RFC 2047-encoded, still folded.
    ///
    /// Use [`get_value`](Self::get_value) for the decoded, unfolded text.
    fn get_value_raw(&self) -> &[u8];

    /// Replace the value with `value`, which is written out verbatim.
    ///
    /// No encoding or folding is applied — the caller is responsible for the result being a valid
    /// header value. [`set_value`](Self::set_value) is the encoding one.
    ///
    /// # Panics
    ///
    /// If `value` is empty.
    fn set_value_raw(&mut self, value: Vec<u8>);

    /// The header line as it should appear in the message, trailing newline included.
    ///
    /// A list of chunks, for the same reason as [`CowMail::serialize`](crate::CowMail::serialize):
    /// an unmodified header stays borrowed from the message it was parsed from.
    fn serialize(&self) -> Vec<Cow<'_, [u8]>>;

    /// The raw header line, key included. Implementations holding the data
    /// should override this to borrow it instead of rebuilding it.
    fn get_raw(&self) -> Cow<'_, [u8]> {
        let key = self.get_key_raw();
        let value = self.get_value_raw();
        let mut raw = Vec::with_capacity(key.len() + 2 + value.len());
        raw.extend_from_slice(key);
        raw.extend_from_slice(b": ");
        raw.extend_from_slice(value);
        Cow::Owned(raw)
    }

    /// The key as text, decoded as latin1 — which cannot fail, and header keys are ASCII anyway.
    fn get_key(&self) -> Cow<'_, str> {
        decode_latin1(self.get_key_raw())
    }

    /// Whether this header's key is `key`, compared case-insensitively as RFC 5322 requires.
    fn key_eq(&self, key: &str) -> bool {
        self.get_key().eq_ignore_ascii_case(key)
    }

    /// Whether this header's key starts with `prefix`, compared case-insensitively.
    ///
    /// Used to find the `Content-*` headers that [`Mail::clear_content`](crate::Mail::clear_content)
    /// drops and [`CowMail::make_mixed`](crate::CowMail::make_mixed) moves into a subpart.
    fn key_starts_with(&self, prefix: &str) -> bool {
        let key = self.get_key_raw();
        let prefix = prefix.as_bytes();
        key.len() >= prefix.len() && key[..prefix.len()].eq_ignore_ascii_case(prefix)
    }

    /// The raw value as text, borrowed when possible.
    ///
    /// RFC 6532 allows UTF-8 in header values, so that is tried first, with a latin1 fallback for
    /// anything else. The value is *not* RFC 2047-decoded and *not* unfolded — this is the value as
    /// written. Use [`get_value`](Self::get_value) for the decoded text.
    fn get_value_ref(&self) -> Cow<'_, str> {
        // RFC 6532 says that header values can be UTF-8. Let's try that first, and
        // fall back to latin1 if that fails
        let value = self.get_value_raw();
        match std::str::from_utf8(value) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => decode_latin1(value),
        }
    }

    /// The header value, unfolded and RFC 2047 decoded.
    fn get_value(&self) -> String {
        match mailparse::parse_header(&self.get_raw()) {
            Ok((header, _)) => header.get_value(),
            // Only reachable for a malformed header; hand back the value as it is
            // rather than losing it.
            Err(_) => self.get_value_ref().into_owned(),
        }
    }

    /// Replace the value with `value`, RFC 2047-encoding and folding it as needed.
    ///
    /// Pass the text the value should decode to. The line is rewritten with CRLF, so a header
    /// changed this way ends up with CRLF even in a message that otherwise uses bare LF.
    ///
    /// # Panics
    ///
    /// If `value` converts to an empty string.
    fn set_value<V: ToString>(&mut self, value: V) {
        let value = value.to_string();
        assert!(!value.is_empty());
        let encoded_value = encode_header_value(&self.get_key(), &value);
        self.set_value_raw(encoded_value.into_bytes());
    }

    /// The value of the RFC 2045 parameter `key`, if this header has one.
    ///
    /// Only `Content-Type` and `Content-Disposition` are treated as carrying parameters; for any
    /// other header this is always `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use cow_mail::{CowHeader, Header};
    ///
    /// let header = CowHeader::new("Content-Type", "text/plain; charset=utf-8");
    /// assert_eq!(header.get_param("charset").unwrap(), "utf-8");
    /// assert!(header.get_param("boundary").is_none());
    /// ```
    fn get_param(&self, key: &str) -> Option<String>
    where
        Self: Sized,
    {
        let header = RFC2045HeaderValue::from(self);
        header.params.get(key).cloned()
    }

    /// Set the RFC 2045 parameter `key` to `value`, replacing it if it is already there.
    ///
    /// The whole value is rewritten, which has two visible consequences: the parameters come back
    /// in alphabetical order, and the mimetype is lowercased — both because the value round-trips
    /// through [`mailparse::parse_content_type`]. Parameter values are always quoted.
    ///
    /// # Panics
    ///
    /// If `key` or `value` converts to an empty string.
    fn set_param<K: ToString, V: ToString>(&mut self, key: K, value: V)
    where
        Self: Sized,
    {
        let key = key.to_string();
        let value = value.to_string();
        assert!(!key.is_empty());
        assert!(!value.is_empty());
        let mut rfc2045_value = RFC2045HeaderValue::from(&*self);
        rfc2045_value.params.insert(key, value);
        self.set_value(rfc2045_value.to_string());
    }
}

struct RFC2045HeaderValue<T> {
    pub value: T,
    pub params: BTreeMap<T, T>,
}

impl<T: ToString> Display for RFC2045HeaderValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.params.is_empty() {
            true => write!(f, "{}", self.value.to_string()),
            false => {
                let params = self
                    .params
                    .iter()
                    .map(|(key, value)| {
                        let key = key.to_string();
                        let encoded_value = encode_header_value(&key, &value.to_string());
                        format!("{key}=\"{encoded_value}\"")
                    })
                    .collect::<Vec<_>>();
                write!(f, "{}; {}", self.value.to_string(), params.join("; "))
            }
        }
    }
}

impl<'a, T: Header<'a>> From<&'_ T> for RFC2045HeaderValue<String> {
    fn from(header: &T) -> Self {
        let key = header.get_key();
        let (value, params) = match key.to_ascii_lowercase().as_str() {
            "content-type" | "content-disposition" => {
                let parsed = parse_content_type(&header.get_value());
                (parsed.mimetype, parsed.params)
            }
            _ => (header.get_value(), BTreeMap::new()),
        };

        RFC2045HeaderValue { value, params }
    }
}

/// A header line stored as it appears in the message.
///
/// The default [`Header`] implementation, and the one a message parsed with
/// [`CowMail::try_from`](crate::CowMail::try_from) is built from. It keeps the whole line — key,
/// separator, value, trailing newline, and any folded continuation lines — as a [`Cow`], plus the
/// offsets of the key and the value within it. A parsed header borrows that line and serializes
/// back to exactly the bytes it came from; changing the value clones it once and rewrites it with
/// CRLF. [`is_owned`](Self::is_owned) tells the two apart.
///
/// # Examples
///
/// ```
/// use cow_mail::{CowHeader, Header};
///
/// let header = CowHeader::new("Subject", "Grüße");
///
/// // Non-ASCII is RFC 2047-encoded on the way in, and decodes back out.
/// assert_eq!(header.get_value_ref(), "=?utf-8?b?R3LDvMOfZQ==?=");
/// assert_eq!(header.get_value(), "Grüße");
/// assert_eq!(header.get_key(), "Subject");
/// assert!(header.is_owned());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowHeader<'a> {
    raw_bytes: Cow<'a, [u8]>,
    key_end: usize,
    value_start: usize,
    value_end: usize,
}

impl CowHeader<'_> {
    /// Whether this header owns its line, i.e. it was built or modified rather than parsed.
    ///
    /// A parsed header that has not been touched borrows from the message and is not owned.
    pub fn is_owned(&self) -> bool {
        match self.raw_bytes {
            Cow::Owned(_) => true,
            Cow::Borrowed(_) => false,
        }
    }
}

impl<'a> Header<'a> for CowHeader<'a> {
    fn new<K: ToString, V: ToString>(key: K, value: V) -> Self {
        let key = key.to_string();
        let value = value.to_string();
        assert!(!key.is_empty());
        assert!(!value.is_empty());
        let encoded_value = encode_header_value(&key, &value);

        let mut key_bytes = key.into_bytes();
        let mut value_bytes = encoded_value.into_bytes();

        let mut raw_bytes: Vec<u8> = Vec::with_capacity(key_bytes.len() + value_bytes.len() + 4);
        raw_bytes.append(&mut key_bytes);
        let key_end = raw_bytes.len();
        raw_bytes.extend(b": ");
        let value_start = raw_bytes.len();
        raw_bytes.append(&mut value_bytes);
        let value_end = raw_bytes.len();
        raw_bytes.extend(b"\r\n");

        Self {
            raw_bytes: Cow::Owned(raw_bytes),
            key_end,
            value_start,
            value_end,
        }
    }

    fn from(raw_bytes: &'a [u8], key_end: usize, value_start: usize, value_end: usize) -> Self {
        Self {
            raw_bytes: Cow::Borrowed(raw_bytes),
            key_end,
            value_start,
            value_end,
        }
    }

    fn get_raw(&self) -> Cow<'_, [u8]> {
        Cow::Borrowed(&self.raw_bytes)
    }

    fn get_key_raw(&self) -> &[u8] {
        &self.raw_bytes[0..self.key_end]
    }

    fn get_value_raw(&self) -> &[u8] {
        &self.raw_bytes[self.value_start..self.value_end]
    }

    fn set_value_raw(&mut self, mut value: Vec<u8>) {
        assert!(!value.is_empty());
        let raw_bytes = self.raw_bytes.to_mut();
        raw_bytes.truncate(self.value_start);
        raw_bytes.append(&mut value);
        self.value_end = raw_bytes.len();
        raw_bytes.extend(b"\r\n");
    }

    fn serialize(&self) -> Vec<Cow<'_, [u8]>> {
        vec![Cow::Borrowed(&self.raw_bytes)]
    }
}

impl Display for CowHeader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.get_key(), self.get_value_ref())
    }
}


/// End of the — possibly folded — header line whose value ends at `value_end`:
/// past the first newline that is not followed by continuation whitespace. Only
/// `\r`, `\n`, spaces and tabs can appear between the end of a value and the end
/// of its line; anything else would have moved `value_end` along.
fn header_line_end(raw_data: &[u8], value_end: usize) -> usize {
    let mut ix = value_end;
    loop {
        let Some(newline_ix) = raw_data[ix..].iter().position(|&byte| byte == b'\n') else {
            return raw_data.len();
        };
        ix += newline_ix + 1;
        if !matches!(raw_data.get(ix), Some(b' ' | b'\t')) {
            return ix;
        }
    }
}

/// Rebuild the header line mailparse scanned into `parsed`, which has to be a
/// header of `raw_data`, so that its key and value slices point into it.
///
/// A `MailHeader` keeps the key and the value, but not the line they came from,
/// which is what we serialize. The key always starts at the first byte of its
/// line, and a line ends where the next one's key begins — hence `next`; the
/// last header of a block has none and needs a look ahead instead. Returns the
/// header and the index just past its line.
fn build_header<'a, H: Header<'a>>(
    raw_data: &'a [u8],
    parsed: &MailHeader<'a>,
    next: Option<&MailHeader<'a>>,
) -> Result<(H, usize), CowMailError> {
    let offset_of = |bytes: &[u8]| {
        offset_in(raw_data, bytes).ok_or(CowMailError::HeaderNotASubslice)
    };

    let key = parsed.get_key_raw();
    let value = parsed.get_value_raw();

    let line_start = offset_of(key)?;
    let value_start = offset_of(value)?;
    let value_end = value_start + value.len();

    let line_end = match next {
        Some(next) => offset_of(next.get_key_raw())?,
        None => header_line_end(raw_data, value_end),
    };

    let header = H::from(
        &raw_data[line_start..line_end],
        key.len(),
        value_start - line_start,
        value_end - line_start,
    );

    Ok((header, line_end))
}

/// Parse the first header line of `raw_data`, borrowing from it.
///
/// Returns the header and the index just past its line, so the next one starts there. Folded
/// continuation lines belong to the header and are included.
///
/// # Errors
///
/// [`CowMailError::MailParseError`] when [`mailparse::parse_header`] rejects the line.
///
/// # Examples
///
/// ```
/// use cow_mail::{CowHeader, Header, parse_header};
///
/// let raw = b"Subject: Hi\r\nTo: nobody\r\n";
/// let (header, next) = parse_header::<CowHeader>(&raw[..]).unwrap();
///
/// assert_eq!(header.get_key(), "Subject");
/// assert_eq!(header.get_value(), "Hi");
/// assert_eq!(&raw[next..], b"To: nobody\r\n");
/// ```
pub fn parse_header<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(H, usize), CowMailError> {
    let (parsed, _) = mailparse::parse_header(raw_data)?;
    build_header(raw_data, &parsed, None)
}