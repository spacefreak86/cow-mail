# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
cargo build              # Build the library
cargo build --release    # Build with optimizations (LTO enabled)
cargo test               # Run all tests
cargo test test_name     # Run a specific test
cargo doc --open         # Generate and view documentation
```

## Architecture

This is a Rust library for parsing, modifying, and serializing email messages (RFC 5322/MIME). It uses copy-on-write semantics to minimize allocations when working with email data.

### Core Types

- **`CowMail<H>`** (`lib.rs`): Main email message struct. Holds headers, body bytes, and nested subparts for multipart messages. Uses `Cow<[u8]>` for zero-copy parsing with owned modifications.

- **`CowMailReader`** (`lib.rs`): Streaming reader implementing `std::io::Read`. Allows efficient serialization without full memory allocation.

- **`CowHeader`** (`header.rs`): Single email header with copy-on-write storage. Tracks byte indices for key/value positions within raw data.

- **`Header` trait** (`header.rs`): Interface for header implementations including parsing, serialization, and RFC 2045 parameter handling.

- **`HeaderMap` trait** (`headers.rs`): Collection operations for headers - find, iterate, add, replace, remove.

- **`Mail` trait** (`lib.rs`): High-level operations for email bodies - content decoding, content-type detection, multipart handling.

- **`CowMailError`** (`lib.rs`): Error type for mail operations (generic errors, parse errors, missing headers).

- **`BodyType`** (`lib.rs`): Enum for body type selection (HTML, PLAIN).

- **`MessageType`** (`lib.rs`): Enum distinguishing SinglePart from MultiPart messages (with boundary).

- **`ContentType`** (`lib.rs`): Wrapper around `ParsedContentType` with conditional defaults for multipart/digest.

### Key Design Patterns

**Copy-on-Write**: Parsed messages borrow from the original byte slice. Only modified fields become owned. The `serialize()` method returns `Vec<Cow<[u8]>>` to preserve this optimization.

**Multipart Handling**: `CowMail` recursively contains `subparts: Vec<CowMail>`. Methods like `make_mixed()`, `make_alternative()`, and `make_related()` transform single-part messages into multipart containers. The original content becomes the first subpart.

**Body Part Navigation**: `get_body_part(BodyType)` traverses the MIME tree to find text/plain or text/html parts, correctly handling `multipart/alternative`, `multipart/related` with `start` parameter, and attachments. The internal `BodyStatus` enum tracks whether a part is a body, not a body, or needs traversal.

### Content Encoding

Body content is automatically encoded/decoded using the `lettre` and `mailparse` crates. `set_content()` automatically selects transfer encoding (quoted-printable for UTF-8 text) and sets appropriate headers.

### High-Level Body Manipulation

- `set_text_body()` / `set_html_body()` / `set_body()`: Intelligently insert or update body parts, creating `multipart/alternative` structures as needed
- These methods use `get_body_parent_part_mut()` to find the correct insertion point in nested MIME structures
