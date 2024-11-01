use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::{self, Display, Write};

use charset::decode_latin1;
use email::rfc2045::Rfc2045Parser;
use email::FromHeader;
use email_encoding::headers::EmailWriter;

fn allowed_str(s: &str) -> bool {
    s.bytes().all(allowed_char)
}

const fn allowed_char(c: u8) -> bool {
    c >= 1 && c <= 9 || c == 11 || c == 12 || c >= 14 && c <= 127
}

struct HeaderValueEncoder<'a> {
    writer: EmailWriter<'a>,
    encode_buf: String,
}

impl<'a> HeaderValueEncoder<'a> {
    fn encode(name: &str, value: &'a str, f: &'a mut impl fmt::Write) -> fmt::Result {
        let encoder = Self::new(name, f);
        encoder.format(value.split_inclusive(' '))
    }

    fn new(name: &str, writer: &'a mut dyn Write) -> Self {
        let line_len = name.len() + ": ".len();
        let writer = EmailWriter::new(writer, line_len, 0, false, false);

        Self {
            writer,
            encode_buf: String::new(),
        }
    }

    fn format(mut self, words_iter: impl Iterator<Item = &'a str>) -> fmt::Result {
        for next_word in words_iter {
            let allowed = allowed_str(next_word);

            if allowed {
                // This word only contains allowed characters

                // the next word is allowed, but we may have accumulated some words to encode
                self.flush_encode_buf()?;

                self.writer.folding().write_str(next_word)?;
            } else {
                // This word contains unallowed characters
                self.encode_buf.push_str(next_word);
            }
        }

        self.flush_encode_buf()?;

        Ok(())
    }

    fn flush_encode_buf(&mut self) -> fmt::Result {
        if self.encode_buf.is_empty() {
            // nothing to encode
            return Ok(());
        }

        let prefix = self.encode_buf.trim_end_matches(' ');
        email_encoding::headers::rfc2047::encode(prefix, &mut self.writer)?;

        let spaces = self.encode_buf.len() - prefix.len();
        for _ in 0..spaces {
            self.writer.space();
        }

        self.encode_buf.clear();
        Ok(())
    }
}

fn encode_header_value(key: &str, value: &str, encoded_value: &mut String) {
    HeaderValueEncoder::encode(key, value, encoded_value).unwrap();
}

pub trait Header<'a>: Clone + Display {
    fn new<K: ToString, V: ToString>(key: K, value: V) -> Self;
    fn from(raw_bytes: &'a [u8], key_end: usize, value_start: usize, value_end: usize) -> Self;
    fn get_key_raw(&self) -> &[u8];
    fn get_value_raw(&self) -> &[u8];
    fn set_value_raw(&mut self, value: Vec<u8>);
    fn serialize(&self) -> Vec<Cow<[u8]>>;

    fn get_key(&self) -> Cow<str> {
        decode_latin1(self.get_key_raw())
    }

    fn key_eq(&self, key: &str) -> bool {
        self.get_key().eq_ignore_ascii_case(key)
    }

    fn get_value_ref(&self) -> Cow<str> {
        // RFC 6532 says that header values can be UTF-8. Let's try that first, and
        // fall back to latin1 if that fails
        let value = self.get_value_raw();
        match std::str::from_utf8(value) {
            Ok(s) => Cow::Borrowed(s),
            Err(_) => decode_latin1(value),
        }
    }

    fn get_value(&self) -> String {
        let raw_value = self.get_value_ref();
        let mut value = String::new();
        let mut newline = false;
        for c in raw_value.chars() {
            match (c, newline) {
                ('\n' | '\r', false) => {
                    newline = true;
                }
                ('\n' | '\r' | '\t' | ' ', true) => (),
                _ => {
                    if newline {
                        newline = false;
                        value.push(' ');
                    }
                    value.push(c);
                }
            }
        }
        FromHeader::from_header(value).expect("String can always be decoded")
    }

    fn set_value<V: ToString>(&mut self, value: V) {
        let value = value.to_string();
        assert!(!value.is_empty());
        let mut encoded_value = String::new();
        encode_header_value(&self.get_key(), &value, &mut encoded_value);
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
    pub params: HashMap<T, T>,
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
                        let mut encoded_value = String::new();
                        encode_header_value(&key, &value.to_string(), &mut encoded_value);
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
            "content-type" | "content-disposition" => Rfc2045Parser::new(&header.get_value()).consume_all(),
            _ => (header.get_value(), HashMap::new()),
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

impl<'a> Header<'a> for CowHeader<'a> {
    fn new<K: ToString, V: ToString>(key: K, value: V) -> Self {
        let key = key.to_string();
        let value = value.to_string();
        assert!(!key.is_empty());
        assert!(!value.is_empty());
        let mut encoded_value = String::new();
        encode_header_value(&key, &value, &mut encoded_value);

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

    fn serialize(&self) -> Vec<Cow<[u8]>> {
        vec![Cow::Borrowed(&self.raw_bytes)]
    }
}

impl Display for CowHeader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.get_key(), self.get_value_ref())
    }
}

enum HeaderParseState {
    Initial,
    Key,
    PreValue,
    Value,
    ValueNewline,
}

pub fn parse_header<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(H, usize), &'static str> {
    let mut it = raw_data.iter();
    let mut ix = 0;
    let mut c = match it.next() {
        None => return Err("Empty string provided"),
        Some(v) => *v,
    };

    let mut ix_key_end = None;
    let mut ix_value_start = 0;
    let mut ix_value_end = 0;

    let mut state = HeaderParseState::Initial;
    loop {
        match state {
            HeaderParseState::Initial => {
                if c == b' ' {
                    return Err(
                        "Header cannot start with a space; it is likely an overhanging line from a previous header",
                    );
                };
                state = HeaderParseState::Key;
                continue;
            }
            HeaderParseState::Key => {
                if c == b':' {
                    ix_key_end = Some(ix);
                    state = HeaderParseState::PreValue;
                } else if c == b'\n' {
                    // Technically this is invalid. We'll handle it gracefully
                    // since it does appear to happen in the wild and other
                    // MTAs deal with it. Our handling is to just treat everything
                    // encountered so far on this line as the header key, and
                    // leave the value empty.
                    ix_key_end = Some(ix);
                    ix_value_start = ix;
                    ix_value_end = ix;
                    ix += 1;
                    break;
                }
            }
            HeaderParseState::PreValue => {
                if c != b' ' {
                    ix_value_start = ix;
                    ix_value_end = ix;
                    state = HeaderParseState::Value;
                    continue;
                }
            }
            HeaderParseState::Value => {
                if c == b'\n' {
                    state = HeaderParseState::ValueNewline;
                } else if c != b'\r' {
                    ix_value_end = ix + 1;
                }
            }
            HeaderParseState::ValueNewline => {
                if c == b' ' || c == b'\t' {
                    state = HeaderParseState::Value;
                    continue;
                } else {
                    break;
                }
            }
        }
        ix += 1;
        c = match it.next() {
            None => break,
            Some(v) => *v,
        };
    }
    match ix_key_end {
        Some(v) => Ok((H::from(&raw_data[0..ix], v, ix_value_start, ix_value_end), ix)),

        None => Ok((
            // Technically this is invalid. We'll handle it gracefully
            // since we handle the analogous situation above. Our handling
            // is to just treat everything encountered on this line as
            // the header key, and leave the value empty.
            H::from(&raw_data[0..ix], ix, ix, ix),
            ix,
        )),
    }
}
