    //use std::io::{Read, Write};
    use crate::{CowHeader, CowMail, Header, HeaderMap, Mail, MediaType, names::HTML, names::TEXT};

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
