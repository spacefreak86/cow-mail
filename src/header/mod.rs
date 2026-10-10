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

pub trait Header<'a>: Clone + Display {
    fn new<K: ToString, V: ToString>(key: K, value: V) -> Self;
    fn from(raw_bytes: &'a [u8], key_end: usize, value_start: usize, value_end: usize) -> Self;
    fn get_key_raw(&self) -> &[u8];
    fn get_value_raw(&self) -> &[u8];
    fn set_value_raw(&mut self, value: Vec<u8>);
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

    fn get_key(&self) -> Cow<'_, str> {
        decode_latin1(self.get_key_raw())
    }

    fn key_eq(&self, key: &str) -> bool {
        self.get_key().eq_ignore_ascii_case(key)
    }

    fn key_starts_with(&self, prefix: &str) -> bool {
        let key = self.get_key_raw();
        let prefix = prefix.as_bytes();
        key.len() >= prefix.len() && key[..prefix.len()].eq_ignore_ascii_case(prefix)
    }

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

    fn set_value<V: ToString>(&mut self, value: V) {
        let value = value.to_string();
        assert!(!value.is_empty());
        let encoded_value = encode_header_value(&self.get_key(), &value);
        self.set_value_raw(encoded_value.into_bytes());
    }

    fn get_param(&self, key: &str) -> Option<String>
    where
        Self: Sized,
    {
        let header = RFC2045HeaderValue::from(self);
        header.params.get(key).cloned()
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CowHeader<'a> {
    raw_bytes: Cow<'a, [u8]>,
    key_end: usize,
    value_start: usize,
    value_end: usize,
}

impl CowHeader<'_> {
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

pub fn parse_header<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(H, usize), CowMailError> {
    let (parsed, _) = mailparse::parse_header(raw_data)?;
    build_header(raw_data, &parsed, None)
}