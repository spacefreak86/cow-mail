use mailparse::MailHeader;

use super::{Header, build_header};
use crate::{CowMailError, extend_over_newline};

/// Find, iterate and edit a collection of headers by key.
///
/// Implementors only have to hand out their headers; everything else is
/// provided. Keys match case-insensitively throughout, and an `index` argument
/// counts occurrences of `key`, not positions in the collection.
pub trait HeaderMap<'a, H: Header<'a> + 'a> {
    /// The headers, in the order they appear in the message.
    fn headers(&self) -> &[H];

    /// The headers, mutably. A `Vec` rather than a slice, because inserting and
    /// removing go through it.
    fn headers_mut(&mut self) -> &mut Vec<H>;

    fn iter_headers<'b>(&'b self, key: &str) -> impl Iterator<Item = &'b H>
    where
        H: 'b,
    {
        self.headers().iter().filter(move |h| h.key_eq(key))
    }

    fn iter_headers_mut<'b>(&'b mut self, key: &str) -> impl Iterator<Item = &'b mut H>
    where
        H: 'b,
    {
        self.headers_mut().iter_mut().filter(move |h| h.key_eq(key))
    }

    /// Examples
    /// ```
    ///     use email_message::{CowMail, HeaderMap};
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

    fn find_header(&self, key: &str) -> Option<&H> {
        self.headers().iter().find(|h| h.key_eq(key))
    }

    fn find_header_mut(&mut self, key: &str) -> Option<&mut H> {
        self.headers_mut().iter_mut().find(|h| h.key_eq(key))
    }

    /// Examples
    /// ```
    ///     use email_message::{CowMail, HeaderMap};
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

    /// Examples
    /// ```
    ///     use email_message::{CowMail, HeaderMap};
    ///     let mut mail = CowMail::default();
    ///     mail.add_header("Subject", "Test");
    ///     assert_eq!(mail.find_value("Subject").unwrap(), "Test");
    /// ```
    fn add_header<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        self.headers_mut().push(H::new(key, value))
    }

    /// Examples
    /// ```
    ///     use email_message::{CowMail, Header, HeaderMap};
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

    /// Examples
    /// ```
    ///     use email_message::{CowMail, Header, HeaderMap};
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

    /// Examples
    /// ```
    ///     use email_message::{CowMail, Header, HeaderMap};
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

    /// Examples
    /// ```
    ///     use email_message::{CowMail, Header, HeaderMap};
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

    /// Examples
    /// ```
    ///     use email_message::{CowMail, Header, HeaderMap};
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

pub fn parse_headers<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(Vec<H>, usize), CowMailError> {
    let (parsed, _) = mailparse::parse_headers(raw_data)?;
    build_headers(raw_data, &parsed)
}