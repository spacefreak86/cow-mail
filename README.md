# email-message

A Rust library for parsing, modifying and serializing email messages (RFC 5322 / MIME).

Copy-on-write throughout: a parsed message borrows from the buffer it was parsed from, and only the
fields you actually change become owned. Serializing hands back a `Vec<Cow<[u8]>>` of borrowed and
owned chunks rather than one concatenated buffer, so untouched parts are never copied.

`parse → serialize` is **byte-identical**. The separators a message was written with — line endings,
delimiter-line padding, preamble and epilogue — are kept verbatim, so a message can be read,
modified in one place, and written back without disturbing anything else. This matters for signed
messages and for anything that has to pass a message through unchanged.

## Quickstart

```rust
use email_message::{CowMail, HeaderMap, Mail};

let raw = b"Subject: Hello\r\n\
            Content-Type: text/plain\r\n\
            \r\n\
            Hi there\r\n";

let mut mail = CowMail::try_from(&raw[..]).unwrap();

// Read headers and the decoded body.
assert_eq!(mail.find_value("Subject").unwrap(), "Hello");
assert_eq!(mail.get_content().unwrap(), "Hi there");

// An untouched message serializes back to exactly the bytes it was parsed from.
assert_eq!(mail.serialize().concat(), raw.to_vec());

// Modify it.
mail.add_header("X-Mailer", "email-message");
mail.set_text_body("Hi again").unwrap();

let out: Vec<u8> = mail.serialize().concat();
assert!(out.starts_with(b"Subject: Hello"));
```

Building a multipart message from scratch:

```rust
use email_message::{CowMail, HeaderMap, Mail, MediaType, names};

let mut mail = CowMail::default();
mail.add_header("Subject", "Report");
mail.set_body("plain text version", "<p>html version</p>").unwrap();

let mut attachment = CowMail::new();
attachment.add_header("Content-Disposition", "attachment; filename=\"report.csv\"");
attachment.set_content("a,b\n1,2\n", Some(MediaType::new(names::TEXT, names::CSV)));
mail.add_attachment(attachment);

assert!(mail.is_multipart());
```

## Design

- **`CowMail`** is the message, and every part of a multipart message is a `CowMail` as well. Its
  fields are public: `headers`, `body_bytes` and `subparts` are meant to be edited directly when the
  provided methods do not fit.
- The `*_orig` fields (`bytes_orig`, `header_bytes_orig`, `body_bytes_orig`) keep the originally
  parsed slices even after `body_bytes` has become owned, so the original and the modified message
  can be compared.
- **`HeaderMap`** finds, iterates and edits headers by key; keys match case-insensitively and an
  `index` argument counts occurrences of that key, not positions in the list. It is implemented for
  `CowMail` and for `Vec<H>`.
- **`Mail`** adds the MIME questions on top: content type, disposition, multipart-ness, and decoding
  the body.
- **`Header`** abstracts a single header line; `CowHeader` is the default implementation and stores
  the raw line plus the offsets of its key and value.
- MIME structure is decided by [`mailparse`](https://crates.io/crates/mailparse) — boundary
  matching, `multipart/digest` propagation, recursion limits. This crate keeps the separators
  mailparse discards, which is what makes the round-trip exact.

## Limitations

- Anything written by this crate uses CRLF. A message whose headers were rewritten therefore mixes
  line endings with its untouched parts — that is deliberate: fidelity for what was not touched.
- A message truncated before its closing delimiter gains a `--boundary--` on the way out, because
  its declared structure says there should be one.
- A message that ends with its header block and no blank line gains that blank line, because the
  header/body separator is always written.

## License

GPL-3.0. See [LICENSE](LICENSE).
