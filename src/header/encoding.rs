//! RFC 2047 encoding and folding of header values.

use std::fmt::Write;
use email_encoding::headers::writer::EmailWriter;

fn allowed_str(s: &str) -> bool {
    s.bytes().all(allowed_char)
}

const fn allowed_char(c: u8) -> bool {
    c >= 1 && c <= 9 || c == 11 || c == 12 || c >= 14 && c <= 127
}

fn encode(encode_buf: &mut String, writer: &mut EmailWriter) {
    if encode_buf.is_empty() {
        return;
    }

    let prefix = encode_buf.trim_end_matches(' ');
    email_encoding::headers::rfc2047::encode(prefix, writer).expect("String is always writable");

    // TODO: add a better API for doing this in email-encoding
    let spaces = encode_buf.len() - prefix.len();
    for _ in 0..spaces {
        writer.space();
    }

    encode_buf.clear();
}

/// Encode `value` as the value of the header named `key`, ready to be written out.
///
/// Walks the value word by word. A word made entirely of allowed bytes is written as it is, with a
/// line break and indentation inserted when the line would grow too long (RFC 5322 folding).
/// Consecutive words containing anything else are collected and RFC 2047-encoded together, so a
/// value mixing plain and non-ASCII text keeps its plain parts readable.
///
/// `key` itself is not part of the result: only its length matters, for the length of the first
/// line.
///
/// # Examples
///
/// ```
/// use email_message::header::encode_header_value;
///
/// // Plain text is left alone.
/// assert_eq!(encode_header_value("Subject", "a test"), "a test");
///
/// // Only the words that need it are encoded.
/// assert_eq!(encode_header_value("Subject", "Grüße"), "=?utf-8?b?R3LDvMOfZQ==?=");
/// ```
pub fn encode_header_value(key: &str, value: &str) -> String {
    let mut encoded_value = String::new();
    let line_len = key.len() + ": ".len();
    let mut writer = EmailWriter::new(&mut encoded_value, line_len, 0, false);
    let mut encode_buf = String::new();
    for next_word in value.split_inclusive(' ') {
        if allowed_str(next_word) {
            // This word only contains allowed characters
            // the next word is allowed, but we may have accumulated some words to encode
            encode(&mut encode_buf, &mut writer);
            writer.folding().write_str(next_word).expect("String is always writable");
        } else {
            // This word contains unallowed characters
            encode_buf.push_str(next_word);
        }
    }
    encode(&mut encode_buf, &mut writer);
    drop(writer);
    encoded_value
}
