//#![forbid(unsafe_code)]

pub mod header;
pub mod headers;
use header::CowHeader;
pub use header::Header;
use headers::parse_headers;
pub use headers::HeaderMap;

use lettre::message::header::ContentTransferEncoding;
use lettre::message::MaybeString;
use mailparse::{
    body::Body, parse_content_disposition, parse_content_type, DispositionType, MailParseError,
    ParsedContentDisposition, ParsedContentType,
};
pub use mediatype::{names, values, MediaType};
use mediatype::{names::*, values::UTF_8, MediaTypeBuf, Name, ReadParams, WriteParams};
use rand::{distr::Alphanumeric, rng, Rng as _};
use std::borrow::Cow;
use std::boxed::Box;
use std::collections::VecDeque;
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
                if mtype.ty() == MULTIPART {
                    if let Some(boundary) = ctype.params.get("boundary") {
                        res = MessageType::MultiPart(boundary.clone());
                    }
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
        let transfer_encoding = self.find_value("Content-Transfer-Encoding");
        match Body::new(self.raw_body(), &ctype, &transfer_encoding) {
            Body::Base64(body) | Body::QuotedPrintable(body) => Ok(body.get_decoded_as_string()?),
            Body::SevenBit(body) | Body::EightBit(body) => Ok(body.get_as_string()?),
            Body::Binary(body) => Ok(body.get_as_string()?),
        }
    }

    fn get_content_raw(&self) -> Result<Vec<u8>, CowMailError> {
        let ctype = self.content_type();
        let transfer_encoding = self.find_value("Content-Transfer-Encoding");
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

    fn get_mime_type(&self) -> MediaTypeBuf {
        self.content_type()
            .mimetype
            .parse::<MediaTypeBuf>()
            .unwrap_or_else(|_| MediaTypeBuf::new(TEXT, PLAIN))
    }

    pub fn set_content<T: ToString>(&mut self, content: T, ctype: Option<MediaType>) {
        self.clear_content();
        let content = content.to_string();

        let mut ctype = ctype.unwrap_or(MediaType::new(TEXT, PLAIN));
        if !content.is_ascii() && ctype.get_param(CHARSET).is_none() {
            ctype.set_param(CHARSET, UTF_8);
        }

        self.headers.add_header("Content-Type", ctype);

        let body = lettre::message::Body::new(content);
        let encoding = body.encoding();
        if encoding != ContentTransferEncoding::SevenBit {
            self.headers.add_header("Content-Transfer-Encoding", encoding);
        }

        self.body_bytes = Cow::Owned(body.into_vec());
    }

    pub fn set_content_raw<B: Into<MaybeString>>(&mut self, content: B, ctype: Option<MediaType>) {
        self.clear_content();
        let body = lettre::message::Body::new(content);

        if let Some(ctype) = ctype {
            self.headers.add_header("Content-Type", ctype);
        }

        let encoding = body.encoding();
        if encoding != ContentTransferEncoding::SevenBit {
            self.headers.add_header("Content-Transfer-Encoding", encoding);
        }

        self.body_bytes = Cow::Owned(body.into_vec());
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

    fn contains_body(&self, mime: MediaType, bodytype: BodyType) -> Option<bool> {
        match self.is_multipart() {
            false => Some(!self.is_attachment() && mime.ty == TEXT && mime.subty == Name::from(bodytype)),
            _ if self.subparts.is_empty() => Some(false),
            _ => None,
        }
    }

    fn get_related_candidate(&self) -> Option<usize> {
        self.content_type().params.get("start").and_then(|start| {
            self.subparts.iter().position(|part| {
                part.headers
                    .find_value("Content-ID")
                    .map(|cid| cid.eq(start))
                    .unwrap_or(false)
            })
        })
    }

    pub fn get_body_part(&self, bodytype: BodyType) -> Option<&CowMail<'a, H>> {
        let mime = self.get_mime_type();

        match self.contains_body(mime.essence(), bodytype) {
            Some(res) => match res {
                true => Some(self),
                false => None,
            },
            None => {
                if mime.subty() == RELATED {
                    if let Some(candidate) = self.get_related_candidate() {
                        return self.subparts[candidate].get_body_part(bodytype);
                    }
                }
                self.subparts.iter().find_map(|p| p.get_body_part(bodytype))
            }
        }
    }

    pub fn get_body_part_mut(&mut self, bodytype: BodyType) -> Option<&mut CowMail<'a, H>> {
        let mime = self.get_mime_type();
        match self.contains_body(mime.essence(), bodytype) {
            Some(res) => match res {
                true => Some(self),
                false => None,
            },
            None => {
                if mime.subty() == RELATED {
                    if let Some(candidate) = self.get_related_candidate() {
                        return self.subparts[candidate].get_body_part_mut(bodytype);
                    }
                }
                self.subparts.iter_mut().find_map(|p| p.get_body_part_mut(bodytype))
            }
        }
    }

    fn make_multipart(&mut self, to_type: Name, boundary: Option<String>) -> Result<(), CowMailError> {
        let ctype = self.get_mime_type();
        if ctype.ty() == MULTIPART {
            let subtype = ctype.subty();
            if subtype == to_type
                || (to_type == ALTERNATIVE && subtype == MIXED)
                || (to_type == RELATED && [ALTERNATIVE, MIXED].contains(&subtype))
            {
                return Err(CowMailError::Generic(format!(
                    "cannot convert {} to multipart/{}",
                    ctype.essence(),
                    to_type
                )));
            }
        }

        let mut subpart = CowMail::new();

        let mut ix = 0;
        while ix < self.headers.len() {
            if self.headers[ix].get_key().to_ascii_lowercase().starts_with("content-") {
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
        Ok(())
    }

    pub fn make_mixed(&mut self, boundary: Option<String>) -> Result<(), CowMailError> {
        self.make_multipart(MIXED, boundary)
    }

    pub fn make_alternative(&mut self, boundary: Option<String>) -> Result<(), CowMailError> {
        self.make_multipart(ALTERNATIVE, boundary)
    }

    pub fn make_related(&mut self, boundary: Option<String>) -> Result<(), CowMailError> {
        self.make_multipart(RELATED, boundary)
    }

    fn add_multipart(&mut self, subpart: CowMail<'a, H>, ctype: MediaType) -> Result<(), CowMailError> {
        match self.is_multipart() {
            false => {
                self.make_multipart(ctype.subty, None)?;
                self.subparts.push(subpart);
            }
            true => {
                let existing_ctype = self.get_mime_type();
                if existing_ctype.ty() == MULTIPART && existing_ctype != ctype {
                    self.make_multipart(ctype.subty, None)?;
                    self.subparts.push(subpart);
                } else {
                    self.subparts.push(subpart);
                }
            }
        }
        Ok(())
    }

    pub fn add_related(&mut self, mut subpart: CowMail<'a, H>) -> Result<(), CowMailError> {
        if subpart.headers.find_header("Content-Disposition").is_none() {
            subpart.headers.add_header("Content-Disposition", "inline");
        }
        self.add_multipart(subpart, MediaType::new(MULTIPART, RELATED))
    }

    pub fn add_alternative(&mut self, subpart: CowMail<'a, H>) -> Result<(), CowMailError> {
        self.add_multipart(subpart, MediaType::new(MULTIPART, ALTERNATIVE))
    }

    pub fn add_attachment(&mut self, mut subpart: CowMail<'a, H>) -> Result<(), CowMailError> {
        if subpart.headers.find_header("Content-Disposition").is_none() {
            subpart.headers.add_header("Content-Disposition", "attachment");
        }
        self.add_multipart(subpart, MediaType::new(MULTIPART, MIXED))
    }

    fn contains_plain_or_html_body(&self, mime: &MediaTypeBuf) -> Option<bool> {
        match self.is_multipart() {
            false => {
                let subty = mime.subty();
                Some(!self.is_attachment() && mime.ty() == TEXT && (subty == PLAIN || subty == HTML))
            }
            true if self.subparts.is_empty() => Some(false),
            true => None,
        }
    }

    fn get_body_parent_part_mut(&mut self) -> Option<&mut CowMail<'a, H>> {
        let mime = self.get_mime_type();

        if let Some(res) = self.contains_plain_or_html_body(&mime) {
            return match res {
                false => None,
                true => Some(self),
            };
        }

        let mut stack = VecDeque::new();
        stack.push_back(self as *mut CowMail<H>);

        while let Some(current_ptr) = stack.pop_front() {
            let current = unsafe { &mut *current_ptr };

            if current.get_mime_type().subty() == RELATED {
                if let Some(candidate) = current.get_related_candidate() {
                    match current.subparts[candidate].contains_plain_or_html_body(&mime) {
                        Some(true) => return Some(current),
                        Some(false) => return None,
                        None => {
                            stack.clear();
                            stack.push_back(&mut current.subparts[candidate])
                        }
                    }
                    continue;
                }
            }
            for p in current.subparts.iter_mut() {
                match p.contains_plain_or_html_body(&mime) {
                    Some(true) => return Some(current),
                    Some(false) => continue,
                    None => stack.push_back(p),
                }
            }
        }

        None
    }

    pub fn set_text_body<T: ToString>(&mut self, content: T) -> Result<(), CowMailError> {
        if let Some(parent) = self.get_body_parent_part_mut() {
            match parent.get_body_part_mut(BodyType::PLAIN) {
                Some(part) => part.set_content(content, None),
                None => {
                    let mut text_part = Self::new();
                    text_part.set_content(content, None);
                    if parent.get_mime_type().subty() == ALTERNATIVE {
                        parent.subparts.insert(0, text_part);
                    } else {
                        let html_part = parent
                            .get_body_part_mut(BodyType::HTML)
                            .ok_or(CowMailError::Generic("get_body_parent_part_mut returned a parent without a plain or html body, this should never happen!".into()))?;
                        html_part.make_alternative(None)?;
                        html_part.subparts.insert(0, text_part);
                    }
                }
            }

            return Ok(());
        }

        if !self.is_multipart() {
            self.make_mixed(None)?;
        }

        let mut text_part = Self::new();
        text_part.set_content(content, None);
        self.subparts.insert(0, text_part);

        Ok(())
    }

    pub fn set_html_body<T: ToString>(&mut self, content: T) -> Result<(), CowMailError> {
        if let Some(parent) = self.get_body_parent_part_mut() {
            match parent.get_body_part_mut(BodyType::HTML) {
                Some(part) => part.set_content(content, Some(MediaType::new(TEXT, HTML))),
                None => {
                    let mut html_part = Self::new();
                    html_part.set_content(content, Some(MediaType::new(TEXT, HTML)));
                    if parent.get_mime_type().subty() == ALTERNATIVE {
                        parent.subparts.push(html_part);
                    } else {
                        let text_part = parent
                            .get_body_part_mut(BodyType::PLAIN)
                            .ok_or(CowMailError::Generic("get_body_parent_part_mut returned a parent without a plain or html body, this should never happen!".into()))?;
                        text_part.make_alternative(None)?;
                        text_part.subparts.push(html_part);
                    }
                }
            }

            return Ok(());
        }

        if !self.is_multipart() {
            self.make_mixed(None)?;
        }

        let mut html_part = Self::new();
        html_part.set_content(content, Some(MediaType::new(TEXT, HTML)));
        self.subparts.insert(0, html_part);

        Ok(())
    }

    pub fn set_body<T: ToString>(&mut self, text: T, html: T) -> Result<(), CowMailError> {
        self.set_text_body(text)?;
        self.set_html_body(html)
    }

    pub fn serialize_headers(&self) -> Vec<Cow<[u8]>> {
        let mut res: Vec<Cow<[u8]>> = self.headers.iter().flat_map(|h| h.serialize()).collect();
        res.push(Cow::Borrowed(b"\r\n"));
        res
    }

    pub fn serialize_body(&self) -> Vec<Cow<[u8]>> {
        match self.message_type() {
            MessageType::SinglePart => match self.body_bytes {
                Cow::Borrowed(body_bytes) => vec![Cow::Borrowed(body_bytes)],
                Cow::Owned(_) => vec![Cow::Borrowed(&self.body_bytes), Cow::Borrowed(self.end_bytes)],
            },
            MessageType::MultiPart(boundary) => std::iter::once(vec![Cow::Borrowed(&*self.body_bytes)])
                .chain(self.subparts.iter().map(|p| {
                    let mut res = vec![
                        Cow::Borrowed("--".as_bytes()),
                        Cow::Owned(boundary.as_bytes().to_owned()),
                        Cow::Borrowed("\r\n".as_bytes()),
                    ];
                    res.append(&mut p.serialize());
                    res
                }))
                .chain(std::iter::once(vec![
                    Cow::Borrowed("--".as_bytes()),
                    Cow::Owned(boundary.as_bytes().to_owned()),
                    Cow::Borrowed("--".as_bytes()),
                    Cow::Borrowed(self.end_bytes),
                ]))
                .flatten()
                .collect(),
        }
    }

    pub fn serialize(&self) -> Vec<Cow<[u8]>> {
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
        self.headers
            .retain(|h| !h.get_key().to_ascii_lowercase().starts_with("content-"));
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

    if body_idx + 2 < raw_data.len() {
        if &raw_data[(raw_data.len() - 2)..] == b"\r\n" {
            body_end_idx = raw_data.len() - 2;
            end_bytes = b"\r\n";
        }
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

// Like find_from_u8, but additionally filters such that `key` is at the start
// of a line (preceded by `\n`) or at the start of the search space.
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

#[cfg(test)]
mod tests {
    //use std::io::{Read, Write};
    use crate::{names::HTML, names::TEXT, CowHeader, CowMail, Header, HeaderMap, Mail, MediaType};

    #[test]
    fn test_header_mutations() {
        let mut mail = CowMail::<CowHeader>::new();
        assert_eq!(mail.headers.len(), 0);

        let test_headers = vec![
            ("X-Test", "Test"),
            ("X-Another-Test", "Another"),
            ("X-Test", "Test2"),
            ("X-Another-Test", "Another1"),
            ("X-Test", "Test3"),
        ];

        for (key, value) in test_headers.iter() {
            mail.add_header(key, value);
        }

        assert_eq!(mail.headers.len(), test_headers.len());
        for (idx, header) in mail.headers.iter().enumerate() {
            assert_eq!(header.get_key(), test_headers[idx].0);
            assert_eq!(header.get_value(), test_headers[idx].1);
        }

        mail.insert_header("X-Test", "Test1", 1);
        assert_eq!(
            mail.iter_values("X-Test").collect::<Vec<_>>(),
            ["Test", "Test1", "Test2", "Test3"]
        );

        mail.replace_all_headers("X-Test", "Test555");
        assert_eq!(
            mail.iter_values("X-Test").collect::<Vec<_>>(),
            ["Test555", "Test555", "Test555", "Test555"]
        );

        mail.replace_header("X-Test", "Test", 0);
        mail.replace_header("X-Test", "Test1", 1);
        mail.replace_header("X-Test", "Test2", 2);
        mail.replace_header("X-Test", "Test3", 3);
        assert_eq!(
            mail.iter_values("X-Test").collect::<Vec<_>>(),
            ["Test", "Test1", "Test2", "Test3"]
        );

        mail.remove_header("X-Test", 1);
        assert_eq!(
            mail.iter_values("X-Test").collect::<Vec<_>>(),
            ["Test", "Test2", "Test3"]
        );
        mail.remove_all_headers("X-Test");
        assert_eq!(mail.headers.len(), 2);
    }

    #[test]
    fn test_body_mutations() {
        let mut mail =
            CowMail::try_from(concat!("Content-Type: text/plain\n", "\n", "This is a test message").as_bytes())
                .unwrap();
        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype, "text/plain");
        assert_eq!(ctype.charset, "us-ascii");
        assert_eq!(mail.is_multipart(), false);
        assert_eq!(mail.get_content().unwrap(), "This is a test message");

        mail.clear_content();
        assert!(mail.find_header("Content-Type").is_none());
        assert_eq!(mail.get_content().unwrap(), "");
        mail.set_content("This is a new test message ❤\n".to_string(), None);
        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype, "text/plain");
        assert_eq!(ctype.charset, "UTF-8");

        mail.clear_content();
        assert_eq!(mail.get_content().unwrap(), "");
        mail.set_content("This is a new test message\n".to_string(), None);
        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype, "text/plain");
        assert_eq!(ctype.charset, "us-ascii");

        mail.add_header("From", "\"John Doe\" <john.doe@example.com>");

        let mut html = CowMail::new();
        html.set_content(
            "<html><body>This is a new test message ☺</body></html>".to_string(),
            Some(MediaType::new(TEXT, HTML)),
        );

        mail.add_alternative(html).unwrap();
        let alt_boundary = mail.get_boundary().unwrap();

        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype, "multipart/alternative");
        assert_eq!(ctype.params.get("boundary").unwrap(), &alt_boundary);
        assert_eq!(
            mail.find_header("Content-Type").unwrap().get_value(),
            format!("multipart/alternative; boundary=\"{}\"", alt_boundary)
        );

        let mut attachment = CowMail::new();
        attachment.set_content("This is a test text file".to_string(), None);
        let mut disp = CowHeader::new("Content-Disposition", "attachment");
        disp.set_param("filename", "Test☺File.html");
        attachment.headers.push(disp);
        mail.add_attachment(attachment).unwrap();

        let mixed_boundary = mail.get_boundary().unwrap();
        let encoded = mail
            .serialize()
            .iter()
            .flat_map(|p| p.into_iter())
            .cloned()
            .collect::<Vec<u8>>();

        let expected = format!(
            "From: \"John Doe\" <john.doe@example.com>\r\n\
             MIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed;\r\n boundary=\"{mixed_boundary}\"\r\n\
             \r\n\
             --{mixed_boundary}\r\n\
             Content-Type: multipart/alternative;\r\n boundary=\"{alt_boundary}\"\r\n\
             \r\n\
             --{alt_boundary}\r\n\
             Content-Type: text/plain\r\n\
             \r\n\
             This is a new test message\r\n\
             \r\n\
             --{alt_boundary}\r\n\
             Content-Type: text/html; charset=UTF-8\r\n\
             Content-Transfer-Encoding: quoted-printable\r\n\
             \r\n\
             <html><body>This is a new test message =E2=98=BA</body></html>\r\n\
             --{alt_boundary}--\r\n\
             --{mixed_boundary}\r\n\
             Content-Type: text/plain\r\n\
             Content-Disposition: attachment;\r\n filename=\"=?utf-8?b?VGVzdOKYukZpbGUuaHRtbA==?=\"\r\n\
             \r\n\
             This is a test text file\r\n\
             --{mixed_boundary}--\r\n"
        );
        let expected = expected.into_bytes();

        assert_eq!(encoded, expected);

        /*
                let mut f = std::fs::File::open("/tmp/test.eml").unwrap();
                let mut data = Vec::new();
                f.read_to_end(&mut data).unwrap();

                let mail = CowMail::try_from(&data[..]).unwrap();
                let mut serialized = mail
                    .serialize()
                    .iter()
                    .flat_map(|p| p.into_iter())
                    .cloned()
                    .collect::<Vec<u8>>();

                assert_eq!(data, serialized);
                let mut f = std::fs::File::create("/tmp/test1.eml").unwrap();
                f.write_all(&mut serialized).unwrap();
        */
    }
}
