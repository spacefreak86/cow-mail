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
