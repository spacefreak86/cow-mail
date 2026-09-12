#![forbid(unsafe_code)]

pub mod body;
pub mod header;

#[cfg(test)]
mod tests;

pub use body::{BodyContent, ContentTransferEncoding};
use header::CowHeader;
pub use header::{Header, HeaderMap, parse_headers};

use mailparse::{
    DispositionType, MailParseError, ParsedContentDisposition, ParsedContentType, body::Body,
    parse_content_disposition, parse_content_type,
};
pub use mediatype::{MediaType, names, values};
use mediatype::{MediaTypeBuf, Name, ReadParams, WriteParams, names::*, values::UTF_8};
use rand::{distr::Alphanumeric, rng, RngExt};
use std::borrow::Cow;
use std::ops::Deref;

#[derive(Debug)]
pub enum CowMailError {
    Generic(String),
    MailParseError(MailParseError),
    MissingContentTypeHeader,
}

impl std::error::Error for CowMailError {}

impl std::fmt::Display for CowMailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generic(error) => write!(f, "{}", error),
            Self::MailParseError(error) => write!(f, "{}", error),
            Self::MissingContentTypeHeader => write!(f, "missing ContentType header"),
        }
    }
}

impl From<MailParseError> for CowMailError {
    fn from(error: MailParseError) -> Self {
        Self::MailParseError(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyType {
    HTML,
    PLAIN,
}

impl BodyType {
    /// The other body type: the two form a `multipart/alternative` pair.
    pub fn other(self) -> Self {
        match self {
            Self::PLAIN => Self::HTML,
            Self::HTML => Self::PLAIN,
        }
    }
}

impl From<BodyType> for Name<'_> {
    fn from(btype: BodyType) -> Self {
        match btype {
            BodyType::PLAIN => PLAIN,
            BodyType::HTML => HTML,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    SinglePart,
    MultiPart(String),
}

#[derive(Debug, Default)]
pub struct ContentType {
    ctype: ParsedContentType,
}

impl ContentType {
    pub fn default_conditional(in_multipart_digest: bool) -> Self {
        let mut ctype = ParsedContentType::default();
        if in_multipart_digest {
            ctype.mimetype = "message/rfc822".to_string();
        }
        Self { ctype }
    }
}

impl Clone for ContentType {
    fn clone(&self) -> Self {
        Self {
            ctype: ParsedContentType {
                mimetype: self.ctype.mimetype.clone(),
                charset: self.ctype.charset.clone(),
                params: self.ctype.params.clone(),
            },
        }
    }
}

impl Deref for ContentType {
    type Target = ParsedContentType;

    fn deref(&self) -> &Self::Target {
        &self.ctype
    }
}

impl From<ParsedContentType> for ContentType {
    fn from(ctype: ParsedContentType) -> Self {
        Self { ctype }
    }
}

impl From<&str> for ContentType {
    fn from(value: &str) -> Self {
        parse_content_type(value).into()
    }
}

pub trait Mail<'a, T: Header<'a> + 'a>: HeaderMap<'a, T> {
    fn clear(&mut self);
    fn clear_content(&mut self);
    fn default_ctype(&self) -> ContentType;
    fn raw_body(&self) -> &[u8];
    fn raw_body_mut(&mut self) -> &mut Cow<'a, [u8]>;

    fn content_type(&self) -> ContentType {
        self.find_header("Content-Type")
            .map(|h| ContentType::from(h.get_value_ref().as_ref()))
            .unwrap_or_else(|| self.default_ctype())
    }

    fn message_type(&self) -> MessageType {
        let ctype = self.content_type();
        match ctype.mimetype.parse::<MediaTypeBuf>() {
            Ok(mtype) => {
                let mut res = MessageType::SinglePart;
                if mtype.ty() == MULTIPART
                    && let Some(boundary) = ctype.params.get("boundary")
                {
                    res = MessageType::MultiPart(boundary.clone());
                }
                res
            }
            Err(_) => MessageType::SinglePart,
        }
    }

    fn is_multipart(&self) -> bool {
        match self.message_type() {
            MessageType::SinglePart => false,
            MessageType::MultiPart(_) => true,
        }
    }

    fn content_disposition(&self) -> Option<ParsedContentDisposition> {
        if let Some(header) = self.find_header("Content-Disposition") {
            let value = header.get_value();
            return Some(parse_content_disposition(&value));
        }
        None
    }

    fn is_attachment(&self) -> bool {
        match self.content_disposition() {
            Some(disp) => matches!(disp.disposition, DispositionType::Attachment),
            None => false,
        }
    }

    fn get_content(&self) -> Result<String, CowMailError> {
        let ctype = self.content_type();
        let transfer_encoding = self
            .find_value("Content-Transfer-Encoding")
            .map(|encoding| encoding.to_lowercase());
        match Body::new(self.raw_body(), &ctype, &transfer_encoding) {
            Body::Base64(body) | Body::QuotedPrintable(body) => Ok(body.get_decoded_as_string()?),
            Body::SevenBit(body) | Body::EightBit(body) => Ok(body.get_as_string()?),
            Body::Binary(body) => Ok(body.get_as_string()?),
        }
    }

    fn get_content_raw(&self) -> Result<Vec<u8>, CowMailError> {
        let ctype = self.content_type();
        let transfer_encoding = self
            .find_value("Content-Transfer-Encoding")
            .map(|encoding| encoding.to_lowercase());
        match Body::new(self.raw_body(), &ctype, &transfer_encoding) {
            Body::Base64(body) | Body::QuotedPrintable(body) => Ok(body.get_decoded()?),
            Body::SevenBit(body) | Body::EightBit(body) => Ok(Vec::from(body.get_raw())),
            Body::Binary(body) => Ok(Vec::from(body.get_raw())),
        }
    }
}

pub fn generate_boundary() -> String {
    rng()
        .sample_iter(&Alphanumeric)
        .take(68)
        .map(char::from)
        .collect::<String>()
}

/// The parsed Content-Type of a part, used to prevent excessive calls go get_mimetype during traversal.
struct PartInfo {
    ctype: ContentType,
    media: MediaTypeBuf,
}

impl PartInfo {
    fn ty(&self) -> Name<'_> {
        self.media.ty()
    }

    fn subty(&self) -> Name<'_> {
        self.media.subty()
    }

    fn boundary(&self) -> Option<&String> {
        self.ctype.params.get("boundary")
    }

    /// The `start` parameter of a multipart/related, naming its root part.
    fn start(&self) -> Option<&String> {
        self.ctype.params.get("start")
    }

    fn is_multipart(&self) -> bool {
        self.ty() == MULTIPART && self.boundary().is_some()
    }
}

/// Result of checking whether a MIME part is a text body (plain/html).
enum BodyCheckStatus {
    /// This part is a text/plain or text/html body (not an attachment)
    IsBody,
    /// This is a leaf node but not a body (attachment, other content type, etc.)
    NotBody,
    /// This is a multipart container - subparts need to be checked
    NeedsTraversal,
}

#[derive(Debug, Clone)]
pub struct CowMail<'a, H: Header<'a>> {
    pub bytes_orig: &'a [u8],
    pub header_bytes_orig: &'a [u8],
    pub headers: Vec<H>,
    pub body_bytes: Cow<'a, [u8]>,
    pub body_bytes_orig: &'a [u8],
    pub subparts: Vec<CowMail<'a, H>>,
    pub end_bytes: &'a [u8],
    pub default_ctype: ContentType,
}

impl<'a> Default for CowMail<'a, CowHeader<'a>> {
    fn default() -> Self {
        CowMail::new()
    }
}

impl<'a, H: Header<'a> + 'a> CowMail<'a, H> {
    pub fn new() -> Self {
        Self {
            bytes_orig: &[],
            header_bytes_orig: &[],
            headers: Vec::new(),
            body_bytes: Cow::Borrowed(&[]),
            body_bytes_orig: &[],
            subparts: Vec::new(),
            end_bytes: b"\r\n",
            default_ctype: ContentType::default(),
        }
    }

    fn part_info(&self) -> PartInfo {
        let ctype = self.content_type();
        let media = ctype
            .mimetype
            .parse::<MediaTypeBuf>()
            .unwrap_or_else(|_| MediaTypeBuf::new(TEXT, PLAIN));
        PartInfo { ctype, media }
    }

    pub fn set_content<T: ToString>(&mut self, content: T, ctype: Option<MediaType>) {
        let content = content.to_string();

        let mut ctype = ctype.unwrap_or(MediaType::new(TEXT, PLAIN));
        if !content.is_ascii() && ctype.get_param(CHARSET).is_none() {
            ctype.set_param(CHARSET, UTF_8);
        }

        self.set_content_raw(content, Some(ctype));
    }

    pub fn set_content_raw<B: Into<BodyContent>>(&mut self, content: B, ctype: Option<MediaType>) {
        self.clear_content();

        if let Some(ctype) = ctype {
            self.headers.add_header("Content-Type", ctype);
        }

        let (bytes, encoding) = body::encode(content);
        if encoding != ContentTransferEncoding::SevenBit {
            self.headers.add_header("Content-Transfer-Encoding", encoding);
        }

        self.body_bytes = Cow::Owned(bytes);
    }

    pub fn get_filename(&self) -> Option<String> {
        self.content_disposition()
            .and_then(|disp| disp.params.get("filename").cloned())
            .or_else(|| self.content_type().params.get("name").cloned())
    }

    pub fn get_boundary(&self) -> Option<String> {
        self.headers
            .find_header("Content-Type")
            .and_then(|h| h.get_param("boundary"))
    }

    pub fn set_boundary<T: ToString>(&mut self, boundary: T) -> Result<(), CowMailError> {
        let header = self
            .headers
            .find_header_mut("Content-Type")
            .ok_or(CowMailError::MissingContentTypeHeader)?;
        header.set_param("boundary", boundary.to_string());
        Ok(())
    }

    /// Check if this MIME part is a text body (plain/html), not a body, or needs traversal.
    fn check_body_status(&self, info: &PartInfo, bodytype: Option<BodyType>) -> BodyCheckStatus {
        match info.is_multipart() {
            true if self.subparts.is_empty() => BodyCheckStatus::NotBody,
            true => BodyCheckStatus::NeedsTraversal,
            false => {
                if info.ty() != TEXT || self.is_attachment() {
                    return BodyCheckStatus::NotBody;
                }

                let subtype = info.subty();
                let is_body = match bodytype {
                    Some(bodytype) => subtype == Name::from(bodytype),
                    None => subtype == PLAIN || subtype == HTML,
                };

                match is_body {
                    true => BodyCheckStatus::IsBody,
                    false => BodyCheckStatus::NotBody,
                }
            }
        }
    }

    fn get_related_candidate(&self, info: &PartInfo) -> Option<usize> {
        info.start().and_then(|start| {
            self.subparts.iter().position(|part| {
                part.headers
                    .find_value("Content-ID")
                    .map(|cid| cid.eq(start))
                    .unwrap_or(false)
            })
        })
    }

    /// The path of subpart indices leading to the body part of `bodytype`.
    /// An empty path means this part is itself the body.
    fn find_body_path(&self, bodytype: BodyType) -> Option<Vec<usize>> {
        let mut path = Vec::new();
        self.push_body_path(bodytype, &mut path).then_some(path)
    }

    fn push_body_path(&self, bodytype: BodyType, path: &mut Vec<usize>) -> bool {
        let info = self.part_info();
        match self.check_body_status(&info, Some(bodytype)) {
            BodyCheckStatus::IsBody => true,
            BodyCheckStatus::NotBody => false,
            BodyCheckStatus::NeedsTraversal => {
                // A multipart/related names its root part with the `start` parameter.
                // When it resolves, that part is the only candidate.
                if info.subty() == RELATED
                    && let Some(candidate) = self.get_related_candidate(&info)
                {
                    path.push(candidate);
                    if self.subparts[candidate].push_body_path(bodytype, path) {
                        return true;
                    }
                    path.pop();
                    return false;
                }

                for (ix, subpart) in self.subparts.iter().enumerate() {
                    path.push(ix);
                    if subpart.push_body_path(bodytype, path) {
                        return true;
                    }
                    path.pop();
                }
                false
            }
        }
    }

    pub fn get_body_part(&self, bodytype: BodyType) -> Option<&CowMail<'a, H>> {
        let path = self.find_body_path(bodytype)?;
        Some(self.get_part_at_path(&path))
    }

    pub fn get_body_part_mut(&mut self, bodytype: BodyType) -> Option<&mut CowMail<'a, H>> {
        let path = self.find_body_path(bodytype)?;
        Some(self.get_part_at_path_mut(&path))
    }

    fn make_multipart(&mut self, to_type: Name, boundary: Option<String>) {
        let info = self.part_info();
        if info.ty() == MULTIPART && info.subty() == to_type {
            return;
        }

        let mut subpart = CowMail::new();

        let mut ix = 0;
        while ix < self.headers.len() {
            if self.headers[ix].key_starts_with("content-") {
                subpart.headers.push(self.headers.remove(ix));
            } else {
                ix += 1;
            }
        }
        match &mut self.body_bytes {
            Cow::Borrowed(body_bytes) => subpart.body_bytes = Cow::Borrowed(*body_bytes),
            Cow::Owned(body_bytes) => subpart.body_bytes.to_mut().append(body_bytes),
        };
        self.body_bytes = Cow::Borrowed(&[]);

        subpart.subparts.append(&mut self.subparts);

        subpart.end_bytes = self.end_bytes;
        self.end_bytes = b"\r\n";

        if self.headers.find_header("MIME-Version").is_none() {
            self.headers.add_header("MIME-Version", "1.0");
        }

        self.headers.add_header(
            "Content-Type",
            format!(
                "multipart/{to_type}; boundary=\"{}\"",
                boundary.unwrap_or_else(generate_boundary)
            ),
        );

        self.subparts.push(subpart);
    }

    pub fn make_mixed(&mut self, boundary: Option<String>) {
        self.make_multipart(MIXED, boundary)
    }

    pub fn make_alternative(&mut self, boundary: Option<String>) {
        self.make_multipart(ALTERNATIVE, boundary)
    }

    pub fn make_related(&mut self, boundary: Option<String>) {
        self.make_multipart(RELATED, boundary)
    }

    /// `make_multipart` already returns early when this part is that multipart
    /// subtype, so it covers both wrapping and appending to an existing container.
    fn add_multipart(&mut self, subpart: CowMail<'a, H>, subtype: Name) {
        self.make_multipart(subtype, None);
        self.subparts.push(subpart);
    }

    pub fn add_related(&mut self, mut subpart: CowMail<'a, H>) {
        if subpart.headers.find_header("Content-Disposition").is_none() {
            subpart.headers.add_header("Content-Disposition", "inline");
        }
        self.add_multipart(subpart, RELATED)
    }

    pub fn add_alternative(&mut self, subpart: CowMail<'a, H>) {
        self.add_multipart(subpart, ALTERNATIVE)
    }

    pub fn add_attachment(&mut self, mut subpart: CowMail<'a, H>) {
        if subpart.headers.find_header("Content-Disposition").is_none() {
            subpart.headers.add_header("Content-Disposition", "attachment");
        }
        self.add_multipart(subpart, MIXED)
    }

    fn get_part_at_path(&self, path: &[usize]) -> &Self {
        let mut current = self;
        for &idx in path {
            current = &current.subparts[idx];
        }
        current
    }

    fn get_part_at_path_mut(&mut self, path: &[usize]) -> &mut Self {
        let mut current = self;
        for &idx in path {
            current = &mut current.subparts[idx];
        }
        current
    }

    fn ctype_for_body(body_type: BodyType) -> Option<MediaType<'static>> {
        match body_type {
            BodyType::PLAIN => None,
            BodyType::HTML => Some(MediaType::new(TEXT, HTML)),
        }
    }

    fn insert_new_part(&mut self, new_part: Self, body_type: BodyType) {
        match body_type {
            BodyType::PLAIN => self.subparts.insert(0, new_part),
            BodyType::HTML => self.subparts.push(new_part),
        }
    }

    fn insert_body_no_existing<T: ToString>(&mut self, content: T, body_type: BodyType) {
        let ctype = Self::ctype_for_body(body_type);

        if !self.is_multipart() {
            self.make_mixed(None);
        }

        let info = self.part_info();
        let subtype = info.subty();

        if subtype == RELATED && body_type == BodyType::PLAIN {
            self.make_alternative(None);
        }

        let mut new_part = Self::new();
        new_part.set_content(content, ctype);
        self.subparts.insert(0, new_part);
    }

    fn insert_body_with_existing<T: ToString>(&mut self, content: T, body_type: BodyType) {
        let ctype = Self::ctype_for_body(body_type);

        let mut new_part = Self::new();
        new_part.set_content(content, ctype);

        let path = self
            .find_body_path(body_type.other())
            .expect("caller has checked that the other body exists");

        let Some((&body_index, container_path)) = path.split_last() else {
            // The message itself is the other body
            self.make_alternative(None);
            self.insert_new_part(new_part, body_type);
            return;
        };

        let container = self.get_part_at_path_mut(container_path);
        let info = container.part_info();
        let subtype = info.subty();

        if subtype == ALTERNATIVE {
            container.insert_new_part(new_part, body_type);
        } else if subtype == RELATED {
            container.make_alternative(None);
            container.insert_new_part(new_part, body_type);
        } else {
            // Any other container: pair the existing body with the new one
            // in an alternative of their own.
            let body_part = &mut container.subparts[body_index];
            body_part.make_alternative(None);
            body_part.insert_new_part(new_part, body_type);
        }
    }

    fn set_body_part<T: ToString>(&mut self, content: T, body_type: BodyType) -> Result<(), CowMailError> {
        let ctype = Self::ctype_for_body(body_type);

        // Case 1: Target body already exists -> update it
        if let Some(part) = self.get_body_part_mut(body_type) {
            part.set_content(content, ctype);
            return Ok(());
        }

        // Case 2: No body at all -> insert directly
        if self.get_body_part(body_type.other()).is_none() {
            self.insert_body_no_existing(content, body_type);
            return Ok(());
        }

        // Case 3: Other body exists -> create alternative structure
        self.insert_body_with_existing(content, body_type);
        Ok(())
    }

    pub fn set_text_body<T: ToString>(&mut self, content: T) -> Result<(), CowMailError> {
        self.set_body_part(content, BodyType::PLAIN)
    }

    pub fn set_html_body<T: ToString>(&mut self, content: T) -> Result<(), CowMailError> {
        self.set_body_part(content, BodyType::HTML)
    }

    pub fn set_body<T: ToString>(&mut self, text: T, html: T) -> Result<(), CowMailError> {
        self.set_text_body(text)?;
        self.set_html_body(html)
    }

    pub fn serialize_headers(&self) -> Vec<Cow<'_, [u8]>> {
        let mut res: Vec<Cow<[u8]>> = self.headers.iter().flat_map(|h| h.serialize()).collect();
        res.push(Cow::Borrowed(b"\r\n"));
        res
    }

    pub fn serialize_body(&self) -> Vec<Cow<'_, [u8]>> {
        match self.message_type() {
            MessageType::SinglePart => match self.body_bytes {
                Cow::Borrowed(body_bytes) => vec![Cow::Borrowed(body_bytes)],
                Cow::Owned(_) => vec![Cow::Borrowed(&self.body_bytes), Cow::Borrowed(self.end_bytes)],
            },
            MessageType::MultiPart(boundary) => {
                let delimiter = [b"--", boundary.as_bytes(), b"\r\n"].concat();
                let closing = [b"--", boundary.as_bytes(), b"--"].concat();

                std::iter::once(vec![Cow::Borrowed(&*self.body_bytes)])
                    .chain(self.subparts.iter().map(|p| {
                        let mut res = vec![Cow::Owned(delimiter.clone())];
                        res.append(&mut p.serialize());
                        res
                    }))
                    .chain(std::iter::once(vec![
                        Cow::Owned(closing),
                        Cow::Borrowed(self.end_bytes),
                    ]))
                    .flatten()
                    .collect()
            }
        }
    }

    pub fn serialize(&self) -> Vec<Cow<'_, [u8]>> {
        let mut res = self.serialize_headers();
        res.append(&mut self.serialize_body());
        res
    }
}

impl<'a, H: Header<'a> + 'a> HeaderMap<'a, H> for CowMail<'a, H> {
    fn iter_headers(&'a self, key: &str) -> impl Iterator<Item = &'a H> {
        self.headers.iter_headers(key)
    }

    fn iter_headers_mut(&'a mut self, key: &str) -> impl Iterator<Item = &'a mut H> {
        self.headers.iter_headers_mut(key)
    }

    fn find_header(&self, key: &str) -> Option<&H> {
        self.headers.find_header(key)
    }

    fn find_header_mut(&mut self, key: &str) -> Option<&mut H> {
        self.headers.find_header_mut(key)
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
    fn iter_values(&'a self, key: &str) -> impl Iterator<Item = String> {
        self.headers.iter_values(key)
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
        self.headers.find_value(key)
    }

    /// Examples
    /// ```
    ///     use email_message::{CowMail, HeaderMap};
    ///     let mut mail = CowMail::default();
    ///     mail.add_header("Subject", "Test");
    ///     assert_eq!(mail.find_value("Subject").unwrap(), "Test");
    /// ```
    fn add_header<K: ToString, V: ToString>(&mut self, key: K, value: V) {
        self.headers.add_header(key, value)
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
        self.headers.insert_header(key, value, index)
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
        self.headers.replace_all_headers(key, value)
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
        self.headers.replace_header(key, value, index)
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
        self.headers.remove_all_headers(key)
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
        self.headers.remove_header(key, index)
    }
}

impl<'a, H: Header<'a> + 'a> Mail<'a, H> for CowMail<'a, H> {
    fn clear(&mut self) {
        self.headers.clear();
        self.body_bytes = Cow::Borrowed(&[]);
        self.subparts.clear();
        self.end_bytes = b"\r\n";
    }

    fn clear_content(&mut self) {
        self.headers.retain(|h| !h.key_starts_with("content-"));
        self.body_bytes = Cow::Borrowed(&[]);
        self.subparts.clear();
        self.end_bytes = b"\r\n";
    }

    fn default_ctype(&self) -> ContentType {
        self.default_ctype.clone()
    }

    fn raw_body(&self) -> &[u8] {
        &self.body_bytes
    }

    fn raw_body_mut(&mut self) -> &mut Cow<'a, [u8]> {
        &mut self.body_bytes
    }
}

impl<'a> TryFrom<&'a [u8]> for CowMail<'a, CowHeader<'a>> {
    type Error = &'static str;

    fn try_from(raw_data: &'a [u8]) -> Result<Self, Self::Error> {
        parse_mail(raw_data)
    }
}

pub struct CowMailReader<'a> {
    headers: Box<dyn Iterator<Item = Cow<'a, [u8]>> + 'a>,
    body: Box<dyn Iterator<Item = Cow<'a, [u8]>> + 'a>,
    current: Option<std::io::Cursor<Cow<'a, [u8]>>>,
}

impl<'a> CowMailReader<'a> {
    pub fn new<H: Header<'a>>(msg: &'a CowMail<'a, H>) -> CowMailReader<'a> {
        CowMailReader::from(msg)
    }

    pub fn skip_headers(&mut self) {
        self.headers = Box::new(std::iter::empty());
    }

    pub fn skip_body(&mut self) {
        self.body = Box::new(std::iter::empty());
    }
}

impl<'a, H: Header<'a>> From<&'a CowMail<'a, H>> for CowMailReader<'a> {
    fn from(msg: &'a CowMail<'a, H>) -> Self {
        CowMailReader {
            headers: Box::new(msg.serialize_headers().into_iter()),
            body: Box::new(msg.serialize_body().into_iter()),
            current: None,
        }
    }
}

impl std::io::Read for CowMailReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let buf_size = buf.len();
        let mut bytes_written: usize = 0;

        while buf_size - bytes_written > 0 {
            if let Some(cur) = &mut self.current {
                let chunk_size = cur.read(&mut buf[bytes_written..])?;
                if chunk_size > 0 {
                    bytes_written += chunk_size;
                    continue;
                }
                self.current = None;
            }

            match self.headers.next().or_else(|| self.body.next()) {
                Some(data) => self.current = Some(std::io::Cursor::new(data)),
                None => break,
            }
        }
        Ok(bytes_written)
    }
}

pub fn parse_mail<'a, H: Header<'a> + 'a>(raw_data: &'a [u8]) -> Result<CowMail<'a, H>, &'static str> {
    parse_mail_recursive(raw_data, false)
}

fn parse_mail_recursive<'a, H: Header<'a> + 'a>(
    raw_data: &'a [u8],
    in_multipart_digest: bool,
) -> Result<CowMail<'a, H>, &'static str> {
    let (headers, body_idx) = parse_headers::<H>(raw_data)?;

    let mut body_end_idx = raw_data.len();
    let mut end_bytes: &[u8] = &[];

    if body_idx + 2 < raw_data.len() && &raw_data[(raw_data.len() - 2)..] == b"\r\n" {
        body_end_idx = raw_data.len() - 2;
        end_bytes = b"\r\n";
    }

    let default_ctype = ContentType::default_conditional(in_multipart_digest);

    let ctype = headers
        .find_value("Content-Type")
        .map(|s| ContentType::from(s.as_str()))
        .unwrap_or(default_ctype.clone());

    let mut mail = CowMail {
        bytes_orig: raw_data,
        header_bytes_orig: &raw_data[0..body_idx],
        headers,
        body_bytes: Cow::Borrowed(&raw_data[body_idx..body_end_idx]),
        body_bytes_orig: &raw_data[body_idx..body_end_idx],
        subparts: Vec::new(),
        end_bytes,
        default_ctype,
    };

    if ctype.mimetype.starts_with("multipart/") && ctype.params.contains_key("boundary") && raw_data.len() > body_idx {
        let in_multipart_digest = ctype.mimetype == "multipart/digest";
        let boundary = String::from("--") + &ctype.params["boundary"];
        if let Some(body_end_idx) = find_from_u8_line_prefix(raw_data, body_idx, boundary.as_bytes()) {
            mail.body_bytes = Cow::Borrowed(&raw_data[body_idx..body_end_idx]);
            let mut boundary_end_idx = body_end_idx + boundary.len();
            while let Some(part_start_idx) = find_from_u8(raw_data, boundary_end_idx, b"\n").map(|v| v + 1) {
                let part_end_idx =
                    find_from_u8_line_prefix(raw_data, part_start_idx, boundary.as_bytes()).unwrap_or(raw_data.len());
                mail.subparts.push(parse_mail_recursive(
                    &raw_data[part_start_idx..part_end_idx],
                    in_multipart_digest,
                )?);
                boundary_end_idx = part_end_idx + boundary.len();
                if boundary_end_idx + 2 > raw_data.len()
                    || (raw_data[boundary_end_idx] == b'-' && raw_data[boundary_end_idx + 1] == b'-')
                {
                    boundary_end_idx += 2;
                    if boundary_end_idx < raw_data.len() {
                        mail.end_bytes = &raw_data[boundary_end_idx..];
                    }
                    break;
                }
            }
        }
    }
    Ok(mail)
}

fn find_from_u8(line: &[u8], ix_start: usize, key: &[u8]) -> Option<usize> {
    assert!(!key.is_empty());
    assert!(ix_start < line.len());
    if line.len() < key.len() {
        return None;
    }
    let ix_end = line.len() - key.len();
    if ix_start <= ix_end {
        for i in ix_start..ix_end {
            if line[i] == key[0] {
                let mut success = true;
                for j in 1..key.len() {
                    if line[i + j] != key[j] {
                        success = false;
                        break;
                    }
                }
                if success {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Like find_from_u8, but additionally filters such that `key` is at the start
/// of a line (preceded by `\n`) or at the start of the search space.
fn find_from_u8_line_prefix(line: &[u8], ix_start: usize, key: &[u8]) -> Option<usize> {
    let mut start = ix_start;
    while let Some(ix) = find_from_u8(line, start, key) {
        if ix == ix_start || line[ix - 1] == b'\n' {
            return Some(ix);
        }
        start = ix + 1;
    }
    None
}
