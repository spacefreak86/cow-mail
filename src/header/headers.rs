use mailparse::MailHeader;

use super::{Header, build_header};
use crate::{CowMailError, extend_over_newline};

pub trait HeaderMap<'a, H: Header<'a> + 'a> {
    fn iter_headers(&'a self, key: &str) -> impl Iterator<Item = &'a H>;
    fn iter_headers_mut(&'a mut self, key: &str) -> impl Iterator<Item = &'a mut H>;
    fn iter_values(&'a self, key: &str) -> impl Iterator<Item = String>;
    fn find_header(&self, key: &str) -> Option<&H>;
    fn find_header_mut(&mut self, key: &str) -> Option<&mut H>;
    fn find_value(&self, key: &str) -> Option<String>;
    fn add_header<K: ToString, V: ToString>(&mut self, key: K, value: V);
    fn insert_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize);
    fn replace_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize);
    fn replace_all_headers<K: ToString, V: ToString>(&mut self, key: K, value: V);
    fn remove_header(&mut self, key: &str, index: usize);
    fn remove_all_headers(&mut self, key: &str);
}

impl<'a, H: Header<'a> + 'a> HeaderMap<'a, H> for Vec<H> {
    fn iter_headers(&'a self, key: &str) -> impl Iterator<Item = &'a H> {
        self.iter().filter(|h| h.key_eq(key))
    }

    fn iter_headers_mut(&'a mut self, key: &str) -> impl Iterator<Item = &'a mut H> {
        self.iter_mut()
            .filter(|h| h.key_eq(key))
    }

    fn iter_values(&'a self, key: &str) -> impl Iterator<Item = String> {
        self.iter_headers(key).map(|h| h.get_value())
    }

    fn find_header(&self, key: &str) -> Option<&H> {
        self.iter().find(|h| h.key_eq(key))
    }

    fn find_header_mut(&mut self, key: &str) -> Option<&mut H> {
        self.iter_mut().find(|h| h.key_eq(key))
    }

    fn find_value(&self, key: &str) -> Option<String> {
        self.find_header(key).map(|h| h.get_value())
    }

    fn add_header<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        self.push(H::new(key, value))
    }

    fn insert_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize) {
        let index = std::cmp::min(index, self.len());
        let header = H::new(key, value);
        self.insert(index, header);
    }

    fn replace_all_headers<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        let key = key.to_string();
        for header in self {
            if !header.get_key().eq_ignore_ascii_case(&key) {
                continue;
            }
            header.set_value(value.to_string());
        }
    }

    fn replace_header<K: ToString, V: ToString>(&mut self, key: K, value: V, index: usize) {
        let key = key.to_string();
        let mut count: usize = 0;
        for header in self {
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

    fn remove_all_headers(&mut self, key: &str) {
        self.retain(|h| !h.key_eq(key));
    }

    fn remove_header(&mut self, key: &str, index: usize) {
        if let Some((index, _)) = self
            .iter()
            .enumerate()
            .filter(move |h| h.1.key_eq(key))
            .nth(index)
        {
            self.remove(index);
        }
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