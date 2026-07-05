use base64::{Engine, engine::general_purpose::STANDARD};
use rust_mcp_sdk::schema::{CallToolResult, ContentBlock, ImageContent, TextContent};
use serde::Serialize;

pub fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::text_content(vec![TextContent::from(s.into())])
}

pub fn json(value: &impl Serialize) -> CallToolResult {
    text(serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
}

fn mime_type_for_extension(path: &str) -> Option<&'static str> {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    }
}

pub fn file_content(path: &str, bytes: &[u8]) -> CallToolResult {
    match mime_type_for_extension(path) {
        Some(mime_type) => {
            let image =
                ImageContent::new(STANDARD.encode(bytes), mime_type.to_string(), None, None);
            CallToolResult {
                content: vec![ContentBlock::from(image)],
                is_error: None,
                meta: None,
                structured_content: None,
            }
        }
        None => match std::str::from_utf8(bytes) {
            Ok(s) => text(s),
            Err(_) => text(format!("[Unknown format] {}", STANDARD.encode(bytes))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_type_for_extension_recognizes_known_image_types() {
        assert_eq!(mime_type_for_extension("photo.png"), Some("image/png"));
        assert_eq!(mime_type_for_extension("photo.jpg"), Some("image/jpeg"));
        assert_eq!(mime_type_for_extension("photo.jpeg"), Some("image/jpeg"));
        assert_eq!(mime_type_for_extension("doc.pdf"), Some("application/pdf"));
    }

    #[test]
    fn mime_type_for_extension_is_case_insensitive() {
        assert_eq!(mime_type_for_extension("PHOTO.PNG"), Some("image/png"));
    }

    #[test]
    fn mime_type_for_extension_returns_none_for_unknown_extensions() {
        assert_eq!(mime_type_for_extension("notes.txt"), None);
        assert_eq!(mime_type_for_extension("main.rs"), None);
    }

    #[test]
    fn mime_type_for_extension_returns_none_for_a_path_with_no_extension() {
        assert_eq!(mime_type_for_extension("Makefile"), None);
    }

    #[test]
    fn file_content_wraps_image_bytes_as_base64_image_content() {
        let result = file_content("photo.png", b"fake-png-bytes");
        match &result.content[0] {
            ContentBlock::ImageContent(image) => {
                assert_eq!(image.mime_type, "image/png");
                assert_eq!(image.data, STANDARD.encode(b"fake-png-bytes"));
            }
            other => panic!("expected image content, got {other:?}"),
        }
    }

    #[test]
    fn file_content_returns_plain_text_for_utf8_non_image_files() {
        let result = file_content("notes.txt", b"hello world");
        match &result.content[0] {
            ContentBlock::TextContent(text) => assert_eq!(text.text, "hello world"),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    #[test]
    fn file_content_base64_encodes_non_utf8_non_image_files() {
        let bytes = [0xff, 0xfe, 0x00, 0xff];
        let result = file_content("data.bin", &bytes);
        match &result.content[0] {
            ContentBlock::TextContent(text) => {
                assert_eq!(
                    text.text,
                    format!("[Unknown format] {}", STANDARD.encode(bytes))
                );
            }
            other => panic!("expected text content, got {other:?}"),
        }
    }
}
