//! Minimal `multipart/form-data` builder.
//!
//! The PDK `HttpClient` does not ship a multipart helper. We hand-roll
//! just enough to satisfy the Exchange v2 asset POST shape.

pub struct File<'a> {
    pub filename: &'a str,
    pub content_type: &'a str,
    pub bytes: &'a [u8],
}

pub struct MultipartBuilder {
    boundary: String,
    body: Vec<u8>,
}

impl MultipartBuilder {
    pub fn new(boundary: impl Into<String>) -> Self {
        Self {
            boundary: boundary.into(),
            body: Vec::with_capacity(1024),
        }
    }

    pub fn add_text(&mut self, name: &str, value: &str) {
        self.write_boundary();
        self.body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }

    pub fn add_file(&mut self, name: &str, file: File<'_>) {
        self.write_boundary();
        self.body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{name}\"; filename=\"{}\"\r\n",
                file.filename
            )
            .as_bytes(),
        );
        self.body
            .extend_from_slice(format!("Content-Type: {}\r\n\r\n", file.content_type).as_bytes());
        self.body.extend_from_slice(file.bytes);
        self.body.extend_from_slice(b"\r\n");
    }

    fn write_boundary(&mut self) {
        self.body
            .extend_from_slice(format!("--{}\r\n", self.boundary).as_bytes());
    }

    /// Returns `(content_type_header_value, body_bytes)`.
    pub fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        let content_type = format!("multipart/form-data; boundary={}", self.boundary);
        (content_type, self.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_text_part() {
        let mut mp = MultipartBuilder::new("BOUND");
        mp.add_text("name", "AgentX");
        let (ct, body) = mp.finish();
        let s = String::from_utf8(body).unwrap();
        assert_eq!(ct, "multipart/form-data; boundary=BOUND");
        assert!(s.contains("--BOUND\r\n"));
        assert!(s.contains("Content-Disposition: form-data; name=\"name\"\r\n\r\nAgentX\r\n"));
        assert!(s.ends_with("--BOUND--\r\n"));
    }

    #[test]
    fn builds_file_part() {
        let mut mp = MultipartBuilder::new("B");
        mp.add_file(
            "files.agent-metadata.json",
            File {
                filename: "agent-card.json",
                content_type: "application/json",
                bytes: br#"{"name":"x"}"#,
            },
        );
        let (_, body) = mp.finish();
        let s = String::from_utf8(body).unwrap();
        assert!(s.contains(
            "Content-Disposition: form-data; name=\"files.agent-metadata.json\"; filename=\"agent-card.json\""
        ));
        assert!(s.contains("Content-Type: application/json"));
        assert!(s.contains(r#"{"name":"x"}"#));
    }
}
