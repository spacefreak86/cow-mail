//! Body transfer-encoding.

use std::fmt::{self, Display};
use std::mem;

use email_encoding::body::{Encoding, base64};

/// The `Content-Transfer-Encoding` of a body, as defined in RFC 2045 section 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentTransferEncoding {
    SevenBit,
    EightBit,
    QuotedPrintable,
    Base64,
    Binary,
}

impl Display for ContentTransferEncoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SevenBit => "7bit",
            Self::EightBit => "8bit",
            Self::QuotedPrintable => "quoted-printable",
            Self::Base64 => "base64",
            Self::Binary => "binary",
        })
    }
}

/// Body content to be encoded.
///
/// Pass [`BodyContent::Text`] whenever the content is valid UTF-8: it allows the more
/// efficient `7bit` and `quoted-printable` encodings to be chosen, while binary content
/// always ends up as `base64`.
#[derive(Debug, Clone)]
pub enum BodyContent {
    Text(String),
    Binary(Vec<u8>),
}

impl From<String> for BodyContent {
    fn from(content: String) -> Self {
        Self::Text(content)
    }
}

impl From<&str> for BodyContent {
    fn from(content: &str) -> Self {
        Self::Text(content.to_owned())
    }
}

impl From<Vec<u8>> for BodyContent {
    fn from(content: Vec<u8>) -> Self {
        Self::Binary(content)
    }
}

impl From<&[u8]> for BodyContent {
    fn from(content: &[u8]) -> Self {
        Self::Binary(content.to_vec())
    }
}

impl From<BodyContent> for Vec<u8> {
    fn from(content: BodyContent) -> Self {
        match content {
            BodyContent::Text(content) => content.into_bytes(),
            BodyContent::Binary(content) => content,
        }
    }
}

/// Encode `content`, choosing the most compact of `7bit`, `quoted-printable` and `base64`.
///
/// Line endings of text content are normalized to CRLF.
pub fn encode(content: impl Into<BodyContent>) -> (Vec<u8>, ContentTransferEncoding) {
    let mut content = content.into();

    // Chosen before normalizing line endings, since inserting `\r` changes line lengths.
    let encoding = match &content {
        BodyContent::Text(content) => Encoding::choose(content.as_str(), false),
        BodyContent::Binary(content) => Encoding::choose(content.as_slice(), false),
    };

    if let BodyContent::Text(content) = &mut content {
        crlf_line_endings(content);
    }
    let content = Vec::from(content);

    match encoding {
        Encoding::SevenBit => (content, ContentTransferEncoding::SevenBit),
        Encoding::EightBit => (content, ContentTransferEncoding::EightBit),
        Encoding::QuotedPrintable => (
            quoted_printable::encode(content),
            ContentTransferEncoding::QuotedPrintable,
        ),
        Encoding::Base64 => {
            let mut encoded = String::with_capacity(base64::encoded_len(content.len()));
            base64::encode(&content, &mut encoded).expect("String is always writable");
            (encoded.into_bytes(), ContentTransferEncoding::Base64)
        }
    }
}

/// In place conversion to CRLF line endings.
///
/// Ported from `lettre::message::body` (MIT licensed).
fn crlf_line_endings(content: &mut String) {
    // relies on the indices being in reverse order
    for ix in lone_lf_indices(content) {
        content.insert(ix, '\r');
    }
}

/// Find, in reverse order, the indices at which a `\r` has to be inserted to give
/// `content` CRLF line endings.
fn lone_lf_indices(content: &str) -> Vec<usize> {
    let mut indices = Vec::new();

    let mut found_lf = false;
    for (ix, c) in content.char_indices().rev() {
        if mem::take(&mut found_lf) && c != '\r' {
            // the previous character was a `\n`, but this one is not a `\r`
            indices.push(ix + c.len_utf8());
        }
        found_lf = c == '\n';
    }

    if found_lf {
        // the first character is a `\n`
        indices.push(0);
    }

    indices
}
