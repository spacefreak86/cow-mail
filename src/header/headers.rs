use super::{Header, parse_header};
use crate::CowMailError;

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


pub fn parse_headers<'a, H: Header<'a>>(raw_data: &'a [u8]) -> Result<(Vec<H>, usize), CowMailError> {
    let mut headers: Vec<H> = Vec::new();
    let mut ix = 0;
    loop {
        if ix >= raw_data.len() {
            break;
        } else if raw_data[ix] == b'\n' {
            ix += 1;
            break;
        } else if raw_data[ix] == b'\r' {
            if ix + 1 < raw_data.len() && raw_data[ix + 1] == b'\n' {
                ix += 2;
                break;
            } else {
                return Err(CowMailError::Generic(
                    "Headers were followed by an unexpected lone CR character!".to_string(),
                ));
            }
        }
        let (header, ix_next) = parse_header(&raw_data[ix..])?;
        headers.push(header);
        ix += ix_next;
    }
    Ok((headers, ix))
}