//! The [`HeaderMap`] collection trait and header-block parsing.

use mailparse::MailHeader;

use super::{Header, build_header};
use crate::{CowMailError, extend_over_newline, parse_error};

/// Find, iterate and edit a collection of headers by key.
///
/// Implementors only have to hand out their headers; everything else is
/// provided. Keys match case-insensitively throughout, and an `index` argument
/// counts occurrences of `key`, not positions in the collection.
///
/// [`headers_mut`](Self::headers_mut) hands out a `&mut Vec<H>` rather than a slice, because
/// inserting and removing go through it — the one constraint this trait puts on an implementor's
/// storage. Implemented for `Vec<H>` and for [`CowMail`](crate::CowMail).
pub trait HeaderMap<'a, H: Header<'a> + 'a> {
    /// The headers, in the order they appear in the message.
    fn headers(&self) -> &[H];

    /// The headers, mutably. A `Vec` rather than a slice, because inserting and
    /// removing go through it.
    fn headers_mut(&mut self) -> &mut Vec<H>;

    /// Every header with this key, in message order.
    fn iter_headers<'b>(&'b self, key: &str) -> impl Iterator<Item = &'b H>
    where
        H: 'b,
    {
        self.headers().iter().filter(move |h| h.key_eq(key))
    }

    /// Every header with this key, mutably, in message order.
    fn iter_headers_mut<'b>(&'b mut self, key: &str) -> impl Iterator<Item = &'b mut H>
    where
        H: 'b,
    {
        self.headers_mut().iter_mut().filter(move |h| h.key_eq(key))
    }

    /// The decoded value of every header with this key, in message order.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, HeaderMap};
    ///     let mail = CowMail::try_from(concat!(
    ///             "Subject: Test\n",
    ///             "Subject: Test1\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     let mut values = mail.iter_values("Subject");
    ///     assert_eq!(values.next().unwrap(), "Test");
    ///     assert_eq!(values.next().unwrap(), "Test1");
    ///     assert!(values.next().is_none());
    /// ```
    fn iter_values<'b>(&'b self, key: &str) -> impl Iterator<Item = String>
    where
        H: 'b,
    {
        self.iter_headers(key).map(|h| h.get_value())
    }

    /// The first header with this key.
    ///
    /// Use [`iter_headers`](Self::iter_headers) when a key can occur more than once, as `Received`
    /// does.
    fn find_header(&self, key: &str) -> Option<&H> {
        self.headers().iter().find(|h| h.key_eq(key))
    }

    /// The first header with this key, mutably.
    fn find_header_mut(&mut self, key: &str) -> Option<&mut H> {
        self.headers_mut().iter_mut().find(|h| h.key_eq(key))
    }

    /// The decoded value of the first header with this key.
    ///
    /// Unfolded and RFC 2047-decoded — see [`Header::get_value`](crate::Header::get_value).
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, HeaderMap};
    ///     let mail = CowMail::try_from(concat!(
    ///             "Subject: Test\n",
    ///             "Subject: Test1",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     assert_eq!(mail.find_value("Subject").unwrap(), "Test");
    /// ```
    fn find_value(&self, key: &str) -> Option<String> {
        self.find_header(key).map(|h| h.get_value())
    }

    /// Append a header, encoding `value` as needed.
    ///
    /// Duplicates are not checked for: adding a key that is already there leaves two of them. Use
    /// [`replace_header`](Self::replace_header) to change an existing one.
    ///
    /// # Panics
    ///
    /// If `key` or `value` converts to an empty string.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, HeaderMap};
    ///     let mut mail = CowMail::default();
    ///     mail.add_header("Subject", "Test");
    ///     assert_eq!(mail.find_value("Subject").unwrap(), "Test");
    /// ```
    fn add_header<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        self.headers_mut().push(H::new(key, value))
    }

    /// Insert a header at `index` in the header list, encoding `value` as needed.
    ///
    /// Unlike everywhere else in this trait, `index` is a position in the list, not a count of
    /// occurrences of `key`. An index past the end appends.
    ///
    /// # Panics
    ///
    /// If `key` or `value` converts to an empty string.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, Header, HeaderMap};
    ///     let mut mail = CowMail::try_from(concat!(
    ///             "Subject: Test\n",
    ///             "To: \"John Doe\" <john.doe@example.com>\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     mail.insert_header("From", "\"me\" <me@example.com>", 1);
    ///     assert_eq!(mail.headers[0].get_key(), "Subject");
    ///     assert_eq!(mail.headers[1].get_key(), "From");
    ///     assert_eq!(mail.headers[2].get_key(), "To");
    /// ```
    fn insert_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize) {
        let headers = self.headers_mut();
        let index = std::cmp::min(index, headers.len());
        let header = H::new(key, value);
        headers.insert(index, header);
    }

    /// Replace the value of the `index`th header with this key, counting from the first one.
    ///
    /// Does nothing when there is no such occurrence. The key keeps the spelling it had.
    ///
    /// # Panics
    ///
    /// If `value` converts to an empty string.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, Header, HeaderMap};
    ///     let mut mail = CowMail::try_from(concat!(
    ///             "Subject: Test1\n",
    ///             "To: \"John Doe\" <john.doe@example.com>\n",
    ///             "Subject: Test2\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     mail.replace_header("Subject", "Test3", 0);
    ///     mail.replace_header("Subject", "Test4", 1);
    ///     assert_eq!(mail.headers[0].get_value_ref(), "Test3");
    ///     assert_eq!(mail.headers[1].get_value_ref(), "\"John Doe\" <john.doe@example.com>");
    ///     assert_eq!(mail.headers[2].get_value_ref(), "Test4");
    /// ```
    fn replace_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize) {
        let key = key.to_string();
        let mut count: usize = 0;
        for header in self.headers_mut() {
            if !header.get_key().eq_ignore_ascii_case(&key) {
                continue;
            }
            if count == index {
                header.set_value(value);
                break;
            }
            count += 1;
        }
    }

    /// Replace the value of every header with this key.
    ///
    /// Does nothing when there is none.
    ///
    /// # Panics
    ///
    /// If `value` converts to an empty string.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, Header, HeaderMap};
    ///     let mut mail = CowMail::try_from(concat!(
    ///             "Subject: Test\n",
    ///             "To: \"John Doe\" <john.doe@example.com>\n",
    ///             "Subject: Test1\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     mail.replace_all_headers("Subject", "Test2");
    ///     assert_eq!(mail.headers[0].get_value_ref(), "Test2");
    ///     assert_eq!(mail.headers[1].get_value_ref(), "\"John Doe\" <john.doe@example.com>");
    ///     assert_eq!(mail.headers[2].get_value_ref(), "Test2");
    /// ```
    fn replace_all_headers<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        let key = key.to_string();
        for header in self.headers_mut() {
            if !header.get_key().eq_ignore_ascii_case(&key) {
                continue;
            }
            header.set_value(value.to_string());
        }
    }

    /// Remove the `index`th header with this key, counting from the first one.
    ///
    /// Does nothing when there is no such occurrence.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, Header, HeaderMap};
    ///     let mut mail = CowMail::try_from(concat!(
    ///             "Subject: Test1\n",
    ///             "To: \"John Doe\" <john.doe@example.com>\n",
    ///             "Subject: Test2\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     mail.remove_header("Subject", 1);
    ///     assert_eq!(mail.headers.len(), 2);
    ///     assert_eq!(mail.headers[0].get_value_ref(), "Test1");
    /// ```
    fn remove_header(&mut self, key: &str, index: usize) {
        let found = self
            .headers()
            .iter()
            .enumerate()
            .filter(|h| h.1.key_eq(key))
            .nth(index)
            .map(|(index, _)| index);

        if let Some(index) = found {
            self.headers_mut().remove(index);
        }
    }

    /// Remove every header with this key.
    ///
    /// # Examples
    /// ```
    ///     use cow_mail::{CowMail, Header, HeaderMap};
    ///     let mut mail = CowMail::try_from(concat!(
    ///             "Subject: Test1\n",
    ///             "To: \"John Doe\" <john.doe@example.com>\n",
    ///             "Subject: Test2\n",
    ///             "\n",
    ///             "This is a test message").as_bytes())
    ///         .unwrap();
    ///     mail.remove_all_headers("Subject");
    ///     assert_eq!(mail.headers.len(), 1);
    ///     assert_eq!(mail.headers[0].get_value_ref(), "\"John Doe\" <john.doe@example.com>");
    /// ```
    fn remove_all_headers(&mut self, key: &str) {
        self.headers_mut().retain(|h| !h.key_eq(key));
    }
}

/// Implemented for the header list itself, which is what makes `mail.headers.add_header(..)` work
/// alongside `mail.add_header(..)` — useful when the borrow checker objects to going through the
/// message.
impl<'a, H: Header<'a> + 'a> HeaderMap<'a, H> for Vec<H> {
    fn headers(&self) -> &[H] {
        self
    }

    fn headers_mut(&mut self) -> &mut Vec<H> {
        self
    }
}

/// Rebuild the header lines mailparse has already scanned, so that a part's
/// header block is only walked once. `parsed` has to be the headers of
/// `raw_data`, so that their key and value slices point into it. Returns the
/// headers and the index the body starts at.
pub(crate) fn build_headers<'a, H: Header<'a>>(
    raw_data: &'a [u8],
    parsed: &[MailHeader<'a>],
) -> Result<(Vec<H>, usize), CowMailError> {
    let mut headers = Vec::with_capacity(parsed.len());
    let mut line_end = 0;

    for (ix, header) in parsed.iter().enumerate() {
        let (header, end) = build_header(raw_data, header, parsed.get(ix + 1))?;
        headers.push(header);
        line_end = end;
    }

    // Whatever terminates the block: the blank line, or nothing at all when the
    // message ends with its headers.
    Ok((headers, extend_over_newline(raw_data, line_end)))
}

/// Parse a header block, borrowing from `raw_data`.
///
/// Returns the headers and the index the body starts at, i.e. just past the blank line that ends
/// the block — or the end of the buffer, for a message that stops at its headers.
///
/// # Errors
///
/// [`CowMailError::Parse`] when the block is malformed.
///
/// # Examples
///
/// ```
/// use cow_mail::{CowHeader, HeaderMap, parse_headers};
///
/// let raw = b"Subject: Hi\r\nTo: nobody\r\n\r\nbody\r\n";
/// let (headers, body_idx) = parse_headers::<CowHeader>(&raw[..]).unwrap();
///
/// assert_eq!(headers.len(), 2);
/// assert_eq!(headers.find_value("Subject").unwrap(), "Hi");
/// assert_eq!(&raw[body_idx..], b"body\r\n");
/// ```
pub fn parse_headers<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(Vec<H>, usize), CowMailError> {
    let (parsed, _) = mailparse::parse_headers(raw_data).map_err(parse_error)?;
    build_headers(raw_data, &parsed)
}
