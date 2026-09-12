    //use std::io::{Read, Write};
    use crate::{BodyType, CowHeader, CowMail, Header, HeaderMap, Mail, MediaType, names::HTML, names::TEXT};

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

        mail.add_alternative(html);
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
        mail.add_attachment(attachment);

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

    // Test: single-part text/plain + set_html_body -> alternative[plain, html]
    #[test]
    fn test_single_plain_set_html() {
        let mut mail = CowMail::try_from(
            concat!("Content-Type: text/plain\n", "\n", "Plain text").as_bytes(),
        )
        .unwrap();

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
        assert_eq!(mail.get_body_part(BodyType::PLAIN).unwrap().get_content().unwrap(), "Plain text");
        assert_eq!(mail.get_body_part(BodyType::HTML).unwrap().get_content().unwrap(), "<p>HTML</p>");
    }

    // Test: empty + set_text + set_html -> alternative[plain, html]
    #[test]
    fn test_empty_set_text_then_html() {
        let mut mail = CowMail::<CowHeader>::new();

        mail.set_text_body("Plain text").unwrap();
        assert_eq!(mail.content_type().mimetype, "text/plain");
        assert!(!mail.is_multipart());

        mail.set_html_body("<p>HTML</p>").unwrap();

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
    }

    // Test: empty + set_html + set_text -> alternative[plain, html]
    #[test]
    fn test_empty_set_html_then_text() {
        let mut mail = CowMail::<CowHeader>::new();

        mail.set_html_body("<p>HTML</p>").unwrap();
        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert!(mail.is_multipart());

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 1);

        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/related");
        mail.set_text_body("Plain text").unwrap();

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "multipart/related");
        assert_eq!(mail.subparts[1].subparts.len(), 2);
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
        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "multipart/related");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype, "multipart/related");
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

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/plain");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 2);
        assert_eq!(mail.subparts[0].content_type().mimetype, "multipart/alternative");
        assert_eq!(mail.subparts[0].subparts.len(), 2);
        assert_eq!(mail.subparts[0].subparts[0].content_type().mimetype, "text/plain");
        assert_eq!(mail.subparts[0].subparts[1].content_type().mimetype, "text/html");
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

        assert_eq!(mail.content_type().mimetype, "multipart/mixed");
        assert_eq!(mail.subparts.len(), 3);
        assert!(mail.subparts[0].is_attachment());
        assert_eq!(mail.subparts[1].content_type().mimetype, "multipart/alternative");
        assert!(mail.subparts[2].is_attachment());
    }
