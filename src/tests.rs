    //use std::io::{Read, Write};
    use crate::{BodyType, CowHeader, CowMail, Header, HeaderMap, Mail, mime};

    fn serialize<'a>(mail: &'a CowMail<'a, CowHeader<'a>>) -> Vec<u8> {
        mail.serialize().iter().flat_map(|p| p.iter()).copied().collect()
    }

    /// Parse `raw` and assert it serializes back to exactly the same bytes.
    fn assert_roundtrip(raw: &str) {
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&serialize(&mail)),
            raw,
            "round-trip is not byte-exact"
        );
    }

    #[test]
    fn test_header_mutations() {
        let mut mail = CowMail::<CowHeader>::new();
        assert_eq!(mail.headers.len(), 0);

        let test_headers = [
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
        assert_eq!(ctype.mimetype(), "text/plain");
        assert_eq!(ctype.charset(), "us-ascii");
        assert!(!mail.is_multipart());
        assert_eq!(mail.get_content().unwrap(), "This is a test message");

        mail.clear_content();
        assert!(mail.find_header("Content-Type").is_none());
        assert_eq!(mail.get_content().unwrap(), "");
        mail.set_content("This is a new test message ❤\n".to_string(), None);
        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype(), "text/plain");
        assert_eq!(ctype.charset(), "UTF-8");

        mail.clear_content();
        assert_eq!(mail.get_content().unwrap(), "");
        mail.set_content("This is a new test message\n".to_string(), None);
        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype(), "text/plain");
        assert_eq!(ctype.charset(), "us-ascii");

        mail.add_header("From", "\"John Doe\" <john.doe@example.com>");

        let mut html = CowMail::new();
        html.set_content(
            "<html><body>This is a new test message ☺</body></html>".to_string(),
            Some(mime::TEXT_HTML),
        );

        mail.add_alternative(html);
        let alt_boundary = mail.get_boundary().unwrap();

        let ctype = mail.content_type();
        assert_eq!(ctype.mimetype(), "multipart/alternative");
        assert_eq!(ctype.get_param("boundary").unwrap(), alt_boundary);
        assert_eq!(
            mail.find_header("Content-Type").unwrap().get_value(),
            format!("multipart/alternative; boundary=\"{}\"", alt_boundary)
        );

        let mut attachment = CowMail::new();
        attachment.set_content("This is a test text file".to_string(), None);
        let mut disp = CowHeader::new("Content-Disposition", "attachment");
        disp.set_param("filename", "Test☺File.html");
        attachment.headers.push(disp);
        mail.add_attachment(attachment);

        let mixed_boundary = mail.get_boundary().unwrap();
        let encoded = mail
            .serialize()
            .iter()
            .flat_map(|p| p.iter())
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

    // Test: single-part text/plain + set_html_body -> alternative[plain, html]
    #[test]
    fn test_single_plain_set_html() {
        let mut mail = CowMail::try_from(
            concat!("Content-Type: text/plain\n", "\n", "Plain text").as_bytes(),
        )
        .unwrap();

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
        assert_eq!(mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(), "Plain text");
        assert_eq!(mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(), "<p>HTML</p>");
    }

    // Test: single-part text/html + set_text_body -> alternative[plain, html]
    #[test]
    fn test_single_html_set_plain() {
        let mut mail = CowMail::try_from(
            concat!("Content-Type: text/html\n", "\n", "<p>HTML</p>").as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
        assert_eq!(mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(), "Plain text");
        assert_eq!(mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(), "<p>HTML</p>");
    }

    // Test: empty + set_text + set_html -> alternative[plain, html]
    #[test]
    fn test_empty_set_text_then_html() {
        let mut mail = CowMail::<CowHeader>::new();

        mail.set_text_body("Plain text").unwrap();
        assert_eq!(mail.content_type().mimetype(), "text/plain");
        assert!(!mail.is_multipart());

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
    }

    // Test: empty + set_html + set_text -> alternative[plain, html]
    #[test]
    fn test_empty_set_html_then_text() {
        let mut mail = CowMail::<CowHeader>::new();

        mail.set_html_body("<p>HTML</p>").unwrap();
        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert!(mail.is_multipart());

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
    }

    // Test: mixed[attachment] + set_text -> mixed[plain, attachment]
    #[test]
    fn test_mixed_attachment_set_text() {
        // Parse a multipart/mixed with just an attachment (no empty subpart)
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attachment content\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 1);

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert!(mail.subparts[1].is_attachment());
    }

    // Test: mixed[attachment] + set_text + set_html -> mixed[alternative[plain,html], attachment]
    #[test]
    fn test_mixed_attachment_set_text_then_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attachment content\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Plain text").unwrap();
        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype(), "text/html");
        assert!(mail.subparts[1].is_attachment());
    }

    // Test: mixed[plain, attachment] + set_html -> mixed[alternative[plain,html], attachment]
    #[test]
    fn test_mixed_plain_attachment_set_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--bound\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attachment\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(mail.subparts.len(), 2);
        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype(), "text/html");
        assert!(mail.subparts[1].is_attachment());
    }

    // Test: related[html, image] + set_text -> alternative[plain, related[html, image]]
    #[test]
    fn test_related_html_set_text() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/related; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/html\n",
                "\n",
                "<p>HTML with image</p>\n",
                "--bound\n",
                "Content-Type: image/png\n",
                "Content-ID: <img1>\n",
                "\n",
                "PNG_DATA\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/related");
        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "multipart/related");
        assert_eq!(mail.subparts[1].subparts.len(), 2);
    }

    // RFC 2045 section 6.1: Content-Transfer-Encoding values are case insensitive
    #[test]
    fn test_uppercase_transfer_encoding_is_decoded() {
        for encoding in ["base64", "Base64", "BASE64"] {
            let raw = format!(
                concat!(
                    "Content-Type: text/plain\n",
                    "Content-Transfer-Encoding: {}\n",
                    "\n",
                    "aGVsbG8gd29ybGQ="
                ),
                encoding
            );
            let mail = CowMail::try_from(raw.as_bytes()).unwrap();
            assert_eq!(mail.get_content().unwrap(), "hello world", "encoding: {encoding}");
            assert_eq!(mail.get_content_raw().unwrap(), b"hello world", "encoding: {encoding}");
        }
    }

    // Test: related[img, plain] without a `start` param + set_html.
    // The body is not the first subpart, which used to hit unreachable!().
    #[test]
    fn test_related_body_not_first_set_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/related; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: image/png\n",
                "Content-Disposition: inline\n",
                "\n",
                "PNG_DATA\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        assert!(mail.get_body_part(BodyType::PLAIN).is_some());
        mail.set_html_body("<p>HTML</p>").unwrap();

        // the related container is wrapped in an alternative holding both bodies
        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "multipart/related");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
        assert_eq!(mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(), "Plain text\n");
        assert_eq!(mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(), "<p>HTML</p>");
    }

    // Test: mixed[related[html,img], attachment] + set_text
    #[test]
    fn test_mixed_related_attachment_set_text() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"mixed\"\n",
                "\n",
                "--mixed\n",
                "Content-Type: multipart/related; boundary=\"related\"\n",
                "\n",
                "--related\n",
                "Content-Type: text/html\n",
                "\n",
                "<p>HTML</p>\n",
                "--related\n",
                "Content-Type: image/png\n",
                "Content-ID: <img1>\n",
                "\n",
                "PNG\n",
                "--related--\n",
                "--mixed\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attachment\n",
                "--mixed--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype(), "multipart/related");
        assert!(mail.subparts[1].is_attachment());
    }

    // Test: alternative[plain] + set_html -> alternative[plain, html]
    #[test]
    fn test_alternative_plain_set_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/alternative; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
    }

    // Test: alternative[html] + set_text -> alternative[plain, html]
    #[test]
    fn test_alternative_html_set_text() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/alternative; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/html\n",
                "\n",
                "<p>HTML</p>\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype(), "text/html");
    }

    // Test: update existing body in alternative[plain, html]
    #[test]
    fn test_update_existing_body() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/alternative; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Original plain\n",
                "--bound\n",
                "Content-Type: text/html\n",
                "\n",
                "<p>Original HTML</p>\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Updated plain").unwrap();
        mail.set_html_body("<p>Updated HTML</p>").unwrap();

        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(
            mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(),
            "Updated plain"
        );
        assert_eq!(
            mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(),
            "<p>Updated HTML</p>"
        );
    }

    // Test: single-part attachment + set_text -> mixed[plain, attachment]
    #[test]
    fn test_single_attachment_set_text() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment; filename=\"test.bin\"\n",
                "\n",
                "binary data"
            )
            .as_bytes(),
        )
        .unwrap();

        assert!(mail.is_attachment());
        assert!(mail.get_body_part(BodyType::PLAIN).is_none());

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[0].get_content().unwrap(), "Plain text");
        assert!(mail.subparts[1].is_attachment());
        assert_eq!(mail.subparts[1].get_content().unwrap(), "binary data");
    }

    // Test: single-part attachment + set_html -> mixed[html, attachment]
    #[test]
    fn test_single_attachment_set_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment; filename=\"test.bin\"\n",
                "\n",
                "binary data"
            )
            .as_bytes(),
        )
        .unwrap();

        assert!(mail.is_attachment());
        assert!(mail.get_body_part(BodyType::HTML).is_none());

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "text/html");
        assert!(mail.subparts[1].is_attachment());
    }

    // Test: single-part attachment + set_text + set_html -> mixed[alternative[plain,html], attachment]
    #[test]
    fn test_single_attachment_set_text_then_html() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment; filename=\"test.bin\"\n",
                "\n",
                "binary data"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_text_body("Plain text").unwrap();
        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype(), "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype(), "text/html");
        assert!(mail.subparts[1].is_attachment());
        assert_eq!(mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(), "Plain text");
        assert_eq!(mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(), "<p>HTML</p>");
    }

    // Test: preserve order in mixed[attach1, plain, attach2] + set_html
    #[test]
    fn test_preserve_order_mixed() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attach1\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--bound\n",
                "Content-Type: application/octet-stream\n",
                "Content-Disposition: attachment\n",
                "\n",
                "attach2\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype(), "multipart/mixed");
        assert_eq!(mail.subparts.len(), 3);
        assert!(mail.subparts[0].is_attachment());
        assert_eq!(mail.subparts[1].content_type().mimetype(), "multipart/alternative");
        assert!(mail.subparts[2].is_attachment());
    }

    // Test: the parser splits preamble, parts, terminators and epilogue the way
    // serialization expects them back.
    #[test]
    fn test_parse_part_boundaries() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "preamble\r\n",
            "--bound\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "--bound\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>HTML</p>\r\n",
            "\r\n",
            "--bound--\r\n",
            "epilogue\r\n"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();

        assert_eq!(mail.raw_body(), b"preamble\r\n");
        assert_eq!(mail.end_bytes, b"\r\nepilogue\r\n");
        assert_eq!(mail.subparts.len(), 2);

        assert_eq!(mail.subparts[0].raw_body(), b"Plain text");
        assert_eq!(mail.subparts[0].end_bytes, b"\r\n");
        // A body ending in a blank line keeps both CRLFs: one for the body, one
        // as the part terminator.
        assert_eq!(mail.subparts[1].raw_body(), b"<p>HTML</p>\r\n");
        assert_eq!(mail.subparts[1].end_bytes, b"\r\n");
    }

    // Test: an empty part keeps its terminator instead of swallowing the header
    // separator.
    #[test]
    fn test_parse_empty_part() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "Subject: empty\r\n",
            "\r\n",
            "--bound--\r\n"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();

        assert_eq!(mail.subparts.len(), 1);
        assert_eq!(mail.subparts[0].bytes_orig, b"Subject: empty\r\n\r\n");
        assert_eq!(mail.subparts[0].raw_body(), b"");
        assert_eq!(mail.subparts[0].end_bytes, b"");
    }

    // Test: a declared boundary that never appears must not lose the body
    // (mailparse drops it in that case).
    #[test]
    fn test_parse_missing_boundary_keeps_body() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "no boundary here\r\n"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();

        assert!(mail.subparts.is_empty());
        assert_eq!(mail.raw_body(), b"no boundary here");
        assert_eq!(mail.end_bytes, b"\r\n");
    }

    // Test: a part of a multipart/digest defaults to message/rfc822
    #[test]
    fn test_parse_digest_default_ctype() {
        let raw = concat!(
            "Content-Type: multipart/digest; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "\r\n",
            "Subject: forwarded\r\n",
            "\r\n",
            "--bound--\r\n"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();

        assert_eq!(mail.subparts.len(), 1);
        assert_eq!(mail.subparts[0].content_type().mimetype(), "message/rfc822");
    }

    // Test: a nested multipart with a preamble and an epilogue round-trips
    // byte-exact.
    #[test]
    fn test_roundtrip_nested_multipart() {
        assert_roundtrip(concat!(
            "From: \"John Doe\" <john.doe@example.com>\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed;\r\n boundary=\"outer\"\r\n",
            "\r\n",
            "This is a MIME message.\r\n",
            "\r\n",
            "--outer\r\n",
            "Content-Type: multipart/alternative; boundary=\"inner\"\r\n",
            "\r\n",
            "--inner\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "--inner\r\n",
            "Content-Type: text/html\r\n",
            "\r\n",
            "<p>HTML</p>\r\n",
            "\r\n",
            "--inner--\r\n",
            "--outer\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"test.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n",
            "\r\n",
            "JVBERi0xLjQK\r\n",
            "--outer--\r\n",
            "epilogue\r\n"
        ));
    }

    // Test: a message written with bare LF keeps its line endings, including on
    // the delimiter lines.
    #[test]
    fn test_roundtrip_lf_line_endings() {
        assert_roundtrip(concat!(
            "Content-Type: multipart/mixed; boundary=\"outer\"\n",
            "\n",
            "preamble\n",
            "--outer\n",
            "Content-Type: multipart/alternative; boundary=\"inner\"\n",
            "\n",
            "--inner\n",
            "Content-Type: text/plain\n",
            "\n",
            "Plain text\n",
            "--inner\n",
            "Content-Type: text/html\n",
            "\n",
            "<p>HTML</p>\n",
            "--inner--\n",
            "--outer--\n"
        ));
    }

    // Test: padding after the boundary on a delimiter line is preserved.
    #[test]
    fn test_roundtrip_delimiter_padding() {
        assert_roundtrip(concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound  \r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "--bound--  \r\n"
        ));
    }

    // Test: the stored delimiter tail is boundary-agnostic, so renaming the
    // boundary keeps the line endings and rewrites every delimiter.
    #[test]
    fn test_set_boundary_keeps_line_endings() {
        let mut mail = CowMail::try_from(
            concat!(
                "Content-Type: multipart/mixed; boundary=\"bound\"\n",
                "\n",
                "--bound\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--bound--\n"
            )
            .as_bytes(),
        )
        .unwrap();

        mail.set_boundary("new").unwrap();

        assert_eq!(
            String::from_utf8_lossy(&serialize(&mail)),
            concat!(
                // The rewritten header, and the blank line after it, are written
                // the way we write headers; the delimiter lines keep the line
                // endings the message had.
                "Content-Type: multipart/mixed; boundary=\"new\"\r\n",
                "\r\n",
                "--new\n",
                "Content-Type: text/plain\n",
                "\n",
                "Plain text\n",
                "--new--\n"
            )
        );
    }

    #[test]
    fn test_roundtrip_single_part() {
        assert_roundtrip("Subject: Test\r\n\r\nThis is a test message\r\n");
        assert_roundtrip("Subject: Test\r\n\r\nThis is a test message");
        assert_roundtrip("Subject: Test\r\n\r\n");
    }

    #[test]
    fn test_roundtrip_edge_parts() {
        // Empty part, and a part whose body ends in a blank line.
        assert_roundtrip(concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "Subject: empty\r\n",
            "\r\n",
            "--bound\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "\r\n",
            "--bound--\r\n"
        ));
        // Closing delimiter at the end of the buffer, no trailing CRLF.
        assert_roundtrip(concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "--bound--"
        ));
        // Declared boundary that never appears: the body is kept as it is.
        assert_roundtrip(concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "no boundary here\r\n"
        ));
    }

    // Test: a message cut off before its closing delimiter keeps every byte it
    // had; serialization completes it with the delimiter that is missing.
    #[test]
    fn test_roundtrip_truncated_message() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&serialize(&mail)),
            format!("{raw}--bound--")
        );
    }

    // Test: an empty body emits no terminator of its own, content supplies one.
    #[test]
    fn test_empty_body_has_no_terminator() {
        let mut mail = CowMail::default();
        mail.add_header("Subject", "Test");
        assert_eq!(String::from_utf8_lossy(&serialize(&mail)), "Subject: Test\r\n\r\n");

        mail.set_text_body("Plain text").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&serialize(&mail)),
            "Subject: Test\r\nContent-Type: text/plain\r\n\r\nPlain text\r\n"
        );
    }

    // Test: a closing delimiter at the very end of the buffer, with no trailing
    // CRLF, used to be missed by the hand-rolled boundary search.
    #[test]
    fn test_parse_closing_boundary_at_eof() {
        let raw = concat!(
            "Content-Type: multipart/mixed; boundary=\"bound\"\r\n",
            "\r\n",
            "--bound\r\n",
            "Content-Type: text/plain\r\n",
            "\r\n",
            "Plain text\r\n",
            "--bound--"
        );
        let mail = CowMail::try_from(raw.as_bytes()).unwrap();

        assert_eq!(mail.subparts.len(), 1);
        assert_eq!(mail.subparts[0].raw_body(), b"Plain text");
        assert_eq!(mail.subparts[0].end_bytes, b"\r\n");
        assert_eq!(mail.end_bytes, b"");
    }

    // The header lines below are rebuilt from the key/value slices mailparse
    // hands back, so each of these pins a case where the line extends past the
    // value: folding, a stray CR, an empty value, or the end of the buffer.
    #[test]
    fn test_roundtrip_folded_header() {
        let raw = concat!(
            "Subject: a\r\n",
            " continued\r\n",
            "To: john@example.com\r\n",
            "\r\n",
            "body\r\n"
        );
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 2);
        assert_eq!(mail.headers[0].get_key_raw(), b"Subject");
        assert_eq!(mail.headers[0].get_value_raw(), b"a\r\n continued");
        assert_eq!(mail.headers[0].get_value(), "a continued");
        assert_eq!(mail.headers[1].get_key_raw(), b"To");
    }

    // A fold whose last line holds nothing but whitespace: the value ends before
    // the line does, for the last header as well as for one in the middle.
    #[test]
    fn test_roundtrip_header_folded_onto_blank_line() {
        let raw = concat!("Subject: x\r\n", " \r\n", "To: y\r\n", " \r\n", "\r\n", "body\r\n");
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 2);
        assert_eq!(mail.headers[0].get_key_raw(), b"Subject");
        assert_eq!(mail.headers[1].get_key_raw(), b"To");
        assert_eq!(mail.raw_body(), b"body");
    }

    // Invalid, but it happens in the wild: mailparse treats the whole line as the
    // key and leaves the value empty. Note that the key keeps the trailing CR.
    #[test]
    fn test_roundtrip_header_without_colon() {
        let raw = concat!("Subject\r\n", "To: y\r\n", "\r\n", "body\r\n");
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 2);
        assert_eq!(mail.headers[0].get_key_raw(), b"Subject\r");
        assert_eq!(mail.headers[0].get_value_raw(), b"");
        assert_eq!(mail.headers[1].get_value_raw(), b"y");
    }

    #[test]
    fn test_roundtrip_empty_header_value() {
        let raw = concat!("X-Empty:\r\n", "Subject: x\r\n", "\r\n", "body\r\n");
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers[0].get_key_raw(), b"X-Empty");
        assert_eq!(mail.headers[0].get_value_raw(), b"");
        assert_eq!(mail.headers[1].get_value_raw(), b"x");
    }

    #[test]
    fn test_roundtrip_no_headers() {
        assert_roundtrip("\r\nbody\r\n");
        assert_roundtrip("\nbody\n");

        let mail = CowMail::try_from("\r\nbody\r\n".as_bytes()).unwrap();
        assert!(mail.headers.is_empty());
        assert_eq!(mail.raw_body(), b"body");
    }

    #[test]
    fn test_roundtrip_lf_headers() {
        let raw = concat!("Subject: a\n", " continued\n", "To: y\n", "\n", "body\n");
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers[0].get_value_raw(), b"a\n continued");
        assert_eq!(mail.headers[1].get_key_raw(), b"To");
    }

    // A message that ends with its header block, with and without the final
    // newline. The blank line separating headers from body is always written, so
    // these are the one case that does not round-trip byte-exact.
    #[test]
    fn test_header_block_without_blank_line() {
        let mail = CowMail::try_from("Subject: x\r\nTo: y\r\n".as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 2);
        assert_eq!(mail.headers[1].get_value_raw(), b"y");
        assert_eq!(mail.raw_body(), b"");
        assert_eq!(
            String::from_utf8_lossy(&serialize(&mail)),
            "Subject: x\r\nTo: y\r\n\r\n"
        );

        let mail = CowMail::try_from("Subject: x".as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 1);
        assert_eq!(mail.headers[0].get_value_raw(), b"x");
        assert_eq!(mail.raw_body(), b"");
        assert_eq!(String::from_utf8_lossy(&serialize(&mail)), "Subject: x\r\n");
    }

    // The header line is rebuilt from pointer offsets, not by searching for the
    // key and value bytes, so a value identical to its key still lands at the
    // right place.
    #[test]
    fn test_roundtrip_header_value_equal_to_key() {
        for raw in [
            "Test: Test\r\n\r\nbody\r\n",
            "Test:Test\r\n\r\nbody\r\n",
            "Test: Test\r\n Test\r\n\r\nbody\r\n",
            "T: T\r\nT: T\r\n\r\nbody\r\n",
        ] {
            assert_roundtrip(raw);
        }

        let mail = CowMail::try_from("Test: Test\r\nTest:Test\r\n\r\nbody\r\n".as_bytes()).unwrap();
        assert_eq!(mail.headers.len(), 2);
        for header in &mail.headers {
            assert_eq!(header.get_key_raw(), b"Test");
            assert_eq!(header.get_value_raw(), b"Test");
        }
        assert_eq!(mail.headers[0].get_raw().as_ref(), b"Test: Test\r\n");
        assert_eq!(mail.headers[1].get_raw().as_ref(), b"Test:Test\r\n");
    }

    // A header line that is nothing but a colon: both the key and the value are
    // empty, and they sit at different offsets.
    #[test]
    fn test_roundtrip_header_colon_only() {
        let raw = ":\r\nTo: y\r\n\r\nbody\r\n";
        assert_roundtrip(raw);

        let mail = CowMail::try_from(raw.as_bytes()).unwrap();
        assert_eq!(mail.headers[0].get_key_raw(), b"");
        assert_eq!(mail.headers[0].get_value_raw(), b"");
        assert_eq!(mail.headers[0].get_raw().as_ref(), b":\r\n");
    }
