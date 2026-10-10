#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(missing_debug_implementations)]
#![warn(rustdoc::broken_intra_doc_links)]

/// Body transfer-encoding: choosing one, and the types describing it.
pub mod body;
/// Header lines: the [`Header`] trait, [`CowHeader`](header::CowHeader), and header-value encoding.
pub mod header;

#[cfg(test)]
mod tests;

pub use body::{BodyContent, ContentTransferEncoding};
pub use header::{CowHeader, Header, HeaderMap, parse_header, parse_headers};
use header::build_headers;

/// Re-exported from [`mailparse`], which parses the MIME structure for this crate: these types
/// appear in this crate's public API, so depending on `mailparse` separately is neither necessary
/// nor advisable — the version re-exported here is the one these signatures refer to.
pub use mailparse::{DispositionType, MailParseError, ParsedContentDisposition, ParsedContentType};
use mailparse::{ParsedMail, body::Body, parse_content_disposition, parse_content_type};

/// Re-exported from [`mediatype`]. [`MediaType`] is the content type accepted by
/// [`CowMail::set_content`] and [`CowMail::set_content_raw`], and [`names`] / [`values`] hold the
/// typed constants to build one with, so depending on `mediatype` separately is neither necessary
/// nor advisable.
pub use mediatype::{MediaType, names, values};
use mediatype::{MediaTypeBuf, Name, ReadParams, WriteParams, names::*, values::UTF_8};
use rand::{RngExt, distr::Alphanumeric, rng};
use std::borrow::Cow;
use std::ops::Deref;

/// Everything that can go wrong while parsing or modifying a message.
#[derive(Debug)]
pub enum CowMailError {
    /// mailparse failed to parse the message. Carries its error unchanged.
    MailParseError(MailParseError),
    /// A part declares a `multipart/*` content type without a `boundary` parameter.
    ///
    /// Only reachable from [`parse_mail`], and only for a part mailparse did split into subparts —
    /// which it cannot do without a boundary. In other words: a broken invariant of the bridge to
    /// mailparse, not malformed input.
    MissingBoundary,
    /// A header line mailparse scanned does not point into the part it was scanned from.
    ///
    /// Header positions are recovered by comparing pointers (see the crate's `offset_in`), which
    /// relies on every key and value being an exact subslice of its part. A mailparse that stops
    /// doing that surfaces here instead of as a wrong index.
    HeaderNotASubslice,
    /// A subpart mailparse split out does not point into its parent's bytes.
    ///
    /// The same invariant as [`HeaderNotASubslice`](Self::HeaderNotASubslice), one level up: part
    /// positions are recovered from `ParsedMail::raw_bytes` being an exact subslice of its
    /// parent's.
    SubpartNotASubslice,
    /// The `Content-Type` header a modification needs is absent.
    ///
    /// Returned by [`CowMail::set_boundary`], which has no header to put the parameter on.
    MissingContentTypeHeader,
}

impl std::error::Error for CowMailError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::MailParseError(error) => Some(error),
            _ => None,
        }
    }
}

impl std::fmt::Display for CowMailError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MailParseError(error) => write!(f, "{}", error),
            Self::MissingBoundary => write!(f, "multipart without a boundary parameter"),
            Self::HeaderNotASubslice => write!(f, "header is not a slice of its part"),
            Self::SubpartNotASubslice => write!(f, "subpart is not a slice of its parent"),
            Self::MissingContentTypeHeader => write!(f, "missing Content-Type header"),
        }
    }
}

impl From<MailParseError> for CowMailError {
    fn from(error: MailParseError) -> Self {
        Self::MailParseError(error)
    }
}

/// Which of the two text bodies of a message is meant.
///
/// A message can carry a `text/plain` body, a `text/html` one, or both as the alternatives of a
/// `multipart/alternative`. This selects one of them for [`CowMail::get_body_part`] and friends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyType {
    /// The `text/html` body.
    HTML,
    /// The `text/plain` body.
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

/// Whether a part carries a body of its own or a list of subparts.
///
/// Returned by [`Mail::message_type`]. A part counts as multipart only when its content type is
/// `multipart/*` *and* it declares a `boundary` parameter, since without a boundary there is no way
/// to tell its subparts apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageType {
    /// The part holds a body of its own.
    SinglePart,
    /// The part holds subparts, separated by the boundary carried here.
    MultiPart(String),
}

/// A parsed `Content-Type` header: the mimetype, the charset and the parameters.
///
/// A thin wrapper around [`ParsedContentType`] that it [`Deref`]s to, so `mimetype`, `charset` and
/// `params` are reachable directly. It exists to add [`Clone`] and a default that knows about
/// `multipart/digest` (see [`default_conditional`](Self::default_conditional)).
///
/// Built from a header value with `ContentType::from("text/plain; charset=utf-8")`, or obtained
/// from a part with [`Mail::content_type`]. Note that the latter re-parses the header on every
/// call, so hold on to the result rather than asking repeatedly.
#[derive(Debug, Default)]
pub struct ContentType {
    ctype: ParsedContentType,
}

impl ContentType {
    /// The content type to assume for a part that has no `Content-Type` header.
    ///
    /// `text/plain; charset=us-ascii` normally, but `message/rfc822` for a part of a
    /// `multipart/digest`, per RFC 2046 section 5.1.5. Pass whether the part sits in such a digest.
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

/// The MIME view of a message or one of its parts, on top of its headers.
///
/// Implementors supply storage — the body bytes, the default content type, and the two ways of
/// emptying a part — and get the MIME questions answered on top: content type and disposition,
/// whether the part is a multipart container or an attachment, and the decoded body. Everything
/// header-related comes from [`HeaderMap`], which this extends.
///
/// Implemented for [`CowMail`].
pub trait Mail<'a, T: Header<'a> + 'a>: HeaderMap<'a, T> {
    /// Empty the part completely: no headers, no body, no subparts.
    fn clear(&mut self);

    /// Empty the content of the part, keeping everything else.
    ///
    /// Drops all `Content-*` headers, the body bytes and the subparts; other headers — `Subject`,
    /// `From`, `MIME-Version` — stay. This is what [`CowMail::set_content`] and
    /// [`CowMail::set_content_raw`] call before writing a new body.
    fn clear_content(&mut self);

    /// The content type to assume when the part has no `Content-Type` header.
    ///
    /// Normally `text/plain; charset=us-ascii`, but `message/rfc822` inside a `multipart/digest`.
    /// See [`ContentType::default_conditional`].
    fn default_ctype(&self) -> ContentType;

    /// The body exactly as it sits in the message, still transfer-encoded.
    ///
    /// For a multipart container this is the preamble, not the subparts. Use [`get_content`] or
    /// [`get_content_raw`] to decode a leaf part's body.
    ///
    /// [`get_content`]: Self::get_content
    /// [`get_content_raw`]: Self::get_content_raw
    fn raw_body(&self) -> &[u8];

    /// The body bytes, mutably, as a [`Cow`] so a modification can take ownership.
    ///
    /// Writing through this replaces the body verbatim: no encoding is applied and no
    /// `Content-Transfer-Encoding` header is updated. Use [`CowMail::set_content`] unless that is
    /// what you want.
    fn raw_body_mut(&mut self) -> &mut Cow<'a, [u8]>;

    /// The parsed `Content-Type` of the part, or [`default_ctype`](Self::default_ctype) when the
    /// header is absent.
    ///
    /// The header is re-parsed on every call, which allocates a map and a string per parameter, so
    /// bind the result instead of calling this repeatedly.
    fn content_type(&self) -> ContentType {
        self.find_header("Content-Type")
            .map(|h| ContentType::from(h.get_value_ref().as_ref()))
            .unwrap_or_else(|| self.default_ctype())
    }

    /// Whether the part carries subparts, and the boundary separating them.
    ///
    /// `multipart/*` without a `boundary` parameter counts as
    /// [`SinglePart`](MessageType::SinglePart), as does an unparseable content type.
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

    /// Whether the part is a multipart container, i.e. [`message_type`](Self::message_type) is
    /// [`MultiPart`](MessageType::MultiPart).
    ///
    /// Note that this reflects what the headers *declare*. A part can be multipart while its
    /// subparts are empty, when the declared boundary never appeared in the message.
    fn is_multipart(&self) -> bool {
        match self.message_type() {
            MessageType::SinglePart => false,
            MessageType::MultiPart(_) => true,
        }
    }

    /// The parsed `Content-Disposition` header, or `None` when there is none.
    ///
    /// Match [`ParsedContentDisposition::disposition`] against [`DispositionType`] for the
    /// `inline` / `attachment` distinction, or read `params` for `filename` — see
    /// [`CowMail::get_filename`].
    fn content_disposition(&self) -> Option<ParsedContentDisposition> {
        if let Some(header) = self.find_header("Content-Disposition") {
            let value = header.get_value();
            return Some(parse_content_disposition(&value));
        }
        None
    }

    /// Whether the part declares `Content-Disposition: attachment`.
    ///
    /// A part with no `Content-Disposition` header, or an `inline` one, is not an attachment.
    fn is_attachment(&self) -> bool {
        match self.content_disposition() {
            Some(disp) => matches!(disp.disposition, DispositionType::Attachment),
            None => false,
        }
    }

    /// The body, transfer-decoded and interpreted as text.
    ///
    /// The `Content-Transfer-Encoding` header decides how to decode (case-insensitively, per RFC
    /// 2045 section 6.1); the charset of the `Content-Type` decides how to turn the bytes into a
    /// string. Use [`get_content_raw`](Self::get_content_raw) for anything that is not text.
    ///
    /// # Errors
    ///
    /// [`CowMailError::MailParseError`] if the body is not valid for the encoding it declares, or
    /// cannot be decoded with the declared charset.
    ///
    /// # Examples
    ///
    /// ```
    /// use email_message::{CowMail, Mail};
    ///
    /// let raw = b"Content-Type: text/plain\r\n\
    ///             Content-Transfer-Encoding: base64\r\n\
    ///             \r\n\
    ///             SGVsbG8sIHdvcmxkIQ==\r\n";
    ///
    /// let mail = CowMail::try_from(&raw[..]).unwrap();
    /// assert_eq!(mail.get_content().unwrap(), "Hello, world!");
    /// ```
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

    /// The body, transfer-decoded, as bytes.
    ///
    /// Like [`get_content`](Self::get_content) but without the charset step, so it works for
    /// attachments and other binary content.
    ///
    /// # Errors
    ///
    /// [`CowMailError::MailParseError`] if the body is not valid for the encoding it declares.
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

/// A fresh random multipart boundary: 68 alphanumeric characters.
///
/// Used by [`CowMail::make_mixed`] and friends when no boundary is given. Long enough that it will
/// not occur in the content it separates.
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

/// An email message, or one part of a multipart one.
///
/// The central type of this crate. A message parsed from a buffer borrows from it: the headers keep
/// their raw lines, the body keeps its slice, and only what is modified becomes owned. Each part of
/// a multipart message is a `CowMail` in turn, held in [`subparts`](Self::subparts), so the same
/// methods apply at every level.
///
/// Parse one with [`CowMail::try_from`] or [`parse_mail`], build one from scratch with
/// [`new`](Self::new) / [`default`](Self::default), and write it back out with
/// [`serialize`](Self::serialize) or through [`CowMailReader`]. Headers come from [`HeaderMap`],
/// the MIME side from [`Mail`].
///
/// # Byte-exact round-trip
///
/// `parse → serialize` is byte-identical, which is why this type keeps more than the message's
/// content: [`delimiter_end`](Self::delimiter_end) and [`end_bytes`](Self::end_bytes) carry the
/// separators the message was written with, and the blank line after the headers follows the line
/// ending of the header before it. Everything this crate writes itself uses CRLF, so a message
/// whose headers were rewritten mixes line endings with its untouched parts — deliberately:
/// fidelity for what was not touched.
///
/// # Public fields
///
/// `headers`, `body_bytes` and `subparts` are public on purpose: direct editing is the escape hatch
/// for anything the provided methods do not cover. The consequence is that nothing derived from the
/// headers can be cached, since there is no way to notice a change — see [`Mail::content_type`].
///
/// # Examples
///
/// ```
/// use email_message::{CowMail, Header, HeaderMap, Mail};
///
/// let raw = b"Subject: Hi\r\n\r\nbody\r\n";
/// let mut mail = CowMail::try_from(&raw[..]).unwrap();
///
/// assert_eq!(mail.find_value("Subject").unwrap(), "Hi");
/// assert_eq!(mail.raw_body(), b"body");
///
/// // Untouched, it serializes back to the exact bytes it came from.
/// assert_eq!(mail.serialize().concat(), raw.to_vec());
///
/// // Edit the header list directly.
/// mail.headers.retain(|h| !h.key_eq("Subject"));
/// assert!(mail.find_value("Subject").is_none());
/// ```
#[derive(Debug, Clone)]
pub struct CowMail<'a, H: Header<'a>> {
    /// The part exactly as it was parsed, headers and body, never modified.
    ///
    /// Empty for a part that was built rather than parsed.
    pub bytes_orig: &'a [u8],
    /// The header block as it was parsed, up to and including the blank line that ends it.
    ///
    /// Kept after [`headers`](Self::headers) has been modified, and used to recover the line ending
    /// to write the header/body separator with when the part has no headers left.
    pub header_bytes_orig: &'a [u8],
    /// The headers of this part, in the order they appear in the message.
    ///
    /// Also reachable through [`HeaderMap`], which is implemented both for `CowMail` and for this
    /// `Vec` itself — so `mail.find_value(..)` and `mail.headers.find_value(..)` both work.
    pub headers: Vec<H>,
    /// The body of this part, still transfer-encoded, borrowed until something writes to it.
    ///
    /// For a multipart container this is the preamble before the first delimiter line, not the
    /// subparts.
    pub body_bytes: Cow<'a, [u8]>,
    /// The body as it was parsed.
    ///
    /// Unlike [`body_bytes`](Self::body_bytes) this stays put when the body is replaced, so the
    /// original and the current body can be compared. Not used by this crate itself.
    pub body_bytes_orig: &'a [u8],
    /// The parts of a multipart message, each one a `CowMail` of its own.
    ///
    /// Empty for a single part — and also for a part that declares a boundary which never appeared
    /// in the message, in which case [`body_bytes`](Self::body_bytes) still holds everything and
    /// [`serialize_body`](Self::serialize_body) will not invent a closing delimiter.
    pub subparts: Vec<CowMail<'a, H>>,
    /// What followed `--<boundary>` on the delimiter line introducing this part,
    /// usually `\r\n`. Kept verbatim so that a message written with bare LF, or
    /// with padding after the boundary, serializes back unchanged.
    ///
    /// Only the tail is stored, not the boundary itself, so it stays valid when the container is
    /// given a new boundary with [`set_boundary`](Self::set_boundary) or the part moves elsewhere.
    pub delimiter_end: &'a [u8],
    /// This part's terminator: the newline before the next delimiter line, or — for a container —
    /// the epilogue after the closing delimiter.
    ///
    /// Empty when there is nothing to terminate, which is how [`new`](Self::new),
    /// [`Mail::clear`] and [`Mail::clear_content`] leave it;
    /// [`set_content_raw`](Self::set_content_raw) sets it to `\r\n` after writing a body.
    pub end_bytes: &'a [u8],
    /// The content type to assume while this part has no `Content-Type` header.
    ///
    /// Set while parsing, since only then is it known whether the part sits in a
    /// `multipart/digest`. See [`ContentType::default_conditional`].
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
            end_bytes: b"",
            delimiter_end: b"\r\n",
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
        self.end_bytes = b"\r\n";
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
        // The blank line between headers and body follows the line ending of the
        // header right before it, so that a message written with bare LF keeps its
        // line endings while rewritten headers keep ours. With no headers at all,
        // the parsed header block is the separator.
        let separator = match res.last() {
            Some(header) => line_ending(header),
            None => line_ending(self.header_bytes_orig),
        };
        res.push(Cow::Borrowed(separator));
        res
    }

    pub fn serialize_body(&self) -> Vec<Cow<'_, [u8]>> {
        match self.message_type() {
            MessageType::MultiPart(boundary) if !self.subparts.is_empty() => {
                let delimiter = [b"--", boundary.as_bytes()].concat();
                let closing = [b"--", boundary.as_bytes(), b"--"].concat();

                std::iter::once(vec![Cow::Borrowed(&*self.body_bytes)])
                    .chain(self.subparts.iter().map(|p| {
                        let mut res = vec![Cow::Owned(delimiter.clone()), Cow::Borrowed(p.delimiter_end)];
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
            // Single part — or a part declaring a boundary that never appeared in
            // the message, which keeps the body it was parsed with.
            _ => vec![Cow::Borrowed(&self.body_bytes), Cow::Borrowed(self.end_bytes)],
        }
    }

    pub fn serialize(&self) -> Vec<Cow<'_, [u8]>> {
        let mut res = self.serialize_headers();
        res.append(&mut self.serialize_body());
        res
    }
}

impl<'a, H: Header<'a> + 'a> HeaderMap<'a, H> for CowMail<'a, H> {
    fn headers(&self) -> &[H] {
        &self.headers
    }

    fn headers_mut(&mut self) -> &mut Vec<H> {
        &mut self.headers
    }
}

impl<'a, H: Header<'a> + 'a> Mail<'a, H> for CowMail<'a, H> {
    fn clear(&mut self) {
        self.headers.clear();
        self.body_bytes = Cow::Borrowed(&[]);
        self.subparts.clear();
        self.end_bytes = b"";
    }

    fn clear_content(&mut self) {
        self.headers.retain(|h| !h.key_starts_with("content-"));
        self.body_bytes = Cow::Borrowed(&[]);
        self.subparts.clear();
        self.end_bytes = b"";
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
    type Error = CowMailError;

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

impl std::fmt::Debug for CowMailReader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CowMailReader")
            .field("current", &self.current)
            .finish_non_exhaustive()
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

pub fn parse_mail<'a, H: Header<'a> + 'a>(raw_data: &'a [u8]) -> Result<CowMail<'a, H>, CowMailError> {
    let parsed = mailparse::parse_mail(raw_data)?;
    build_mail(raw_data, &parsed, false)
}

/// The line ending `bytes` ends with, defaulting to CRLF.
fn line_ending(bytes: &[u8]) -> &'static [u8] {
    match bytes {
        [.., b'\r', b'\n'] => b"\r\n",
        [.., b'\n'] => b"\n",
        _ => b"\r\n",
    }
}

/// Byte offset of `inner` within `outer`. Both have to point into the same
/// buffer, which holds for every `ParsedMail::raw_bytes` relative to its
/// parent's. Returns `None` instead of panicking, so that a mailparse that
/// stops handing back subslices surfaces as an error, not as a bad index.
pub(crate) fn offset_in(outer: &[u8], inner: &[u8]) -> Option<usize> {
    let ix = (inner.as_ptr() as usize).checked_sub(outer.as_ptr() as usize)?;
    (ix + inner.len() <= outer.len()).then_some(ix)
}

/// Widen `end` over the newline that follows it, if there is one. Used to undo
/// mailparse's `strip_trailing_crlf`, so that a part's slice covers everything
/// it occupied in the message, and to step over the blank line that ends a
/// header block.
pub(crate) fn extend_over_newline(raw_data: &[u8], end: usize) -> usize {
    if raw_data[end..].starts_with(b"\r\n") {
        end + 2
    } else if raw_data[end..].starts_with(b"\n") {
        end + 1
    } else {
        end
    }
}

/// Build a `CowMail` from a part mailparse has already split out.
///
/// `raw_data` is the part as it appears in the message, including the newline
/// mailparse strips off before a delimiter line; `parsed` has to start at the
/// same offset. mailparse decides where the parts are, we keep the separators
/// it drops.
fn build_mail<'a, H: Header<'a> + 'a>(
    raw_data: &'a [u8],
    parsed: &ParsedMail<'a>,
    in_multipart_digest: bool,
) -> Result<CowMail<'a, H>, CowMailError> {
    let (headers, body_idx) = build_headers::<H>(raw_data, &parsed.headers)?;

    let mut body_end_idx = raw_data.len();
    let mut end_bytes: &[u8] = &[];

    if body_idx + 2 < raw_data.len() && &raw_data[(raw_data.len() - 2)..] == b"\r\n" {
        body_end_idx = raw_data.len() - 2;
        end_bytes = b"\r\n";
    }

    let mut mail = CowMail {
        bytes_orig: raw_data,
        header_bytes_orig: &raw_data[0..body_idx],
        headers,
        body_bytes: Cow::Borrowed(&raw_data[body_idx..body_end_idx]),
        body_bytes_orig: &raw_data[body_idx..body_end_idx],
        subparts: Vec::new(),
        end_bytes,
        // Set by the caller, which is the only one that can see the delimiter
        // line in front of this part.
        delimiter_end: b"\r\n",
        default_ctype: ContentType::default_conditional(in_multipart_digest),
    };

    // An empty subpart list covers both "not a multipart" and "boundary declared
    // but never seen"; either way the body stays what it is above. Note that
    // mailparse drops the body in the latter case, so its body_bytes is of no use
    // to us here.
    let Some(first_part) = parsed.subparts.first() else {
        return Ok(mail);
    };

    let boundary = parsed
        .ctype
        .params
        .get("boundary")
        .ok_or(CowMailError::MissingBoundary)?;

    let offset_of = |part: &ParsedMail<'a>| {
        offset_in(raw_data, part.raw_bytes).ok_or(CowMailError::SubpartNotASubslice)
    };

    // The preamble ends where the delimiter line before the first part begins.
    let first_part_idx = offset_of(first_part)?;
    let scan_end_idx = first_part_idx.saturating_sub(1).max(body_idx);
    let mut delimiter_idx = raw_data[body_idx..scan_end_idx]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map(|ix| body_idx + ix + 1)
        .unwrap_or(body_idx);

    mail.body_bytes = Cow::Borrowed(&raw_data[body_idx..delimiter_idx]);
    mail.body_bytes_orig = &raw_data[body_idx..delimiter_idx];

    let in_multipart_digest = parsed.ctype.mimetype == "multipart/digest";

    for part in &parsed.subparts {
        let part_idx = offset_of(part)?;
        // Widening over the stripped newline is what keeps an empty part, or one
        // whose body ends in a blank line, byte-exact: the last newline is the
        // part's terminator, not part of the delimiter.
        let part_end_idx = extend_over_newline(raw_data, part_idx + part.raw_bytes.len());
        let mut subpart = build_mail(&raw_data[part_idx..part_end_idx], part, in_multipart_digest)?;

        // Whatever the delimiter line carried after `--<boundary>`.
        let delimiter_end_idx = delimiter_idx + 2 + boundary.len();
        if delimiter_end_idx <= part_idx {
            subpart.delimiter_end = &raw_data[delimiter_end_idx..part_idx];
        }
        mail.subparts.push(subpart);
        // The part ends right where the next delimiter line begins.
        delimiter_idx = part_end_idx;
    }

    // Everything after the closing delimiter is the epilogue. Without a closing
    // delimiter there is nothing left over: the trailing newline the split above
    // found belongs to the last part, not to this one.
    let closing_idx = delimiter_idx + 2 + boundary.len();
    mail.end_bytes = match closing_idx + 2 <= raw_data.len() && &raw_data[closing_idx..closing_idx + 2] == b"--" {
        true => &raw_data[closing_idx + 2..],
        false => b"",
    };

    Ok(mail)
}