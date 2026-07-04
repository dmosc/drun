use base64::{Engine, engine::general_purpose::STANDARD};
use rust_mcp_sdk::schema::{CallToolResult, ContentBlock, ImageContent, TextContent};
use serde::Serialize;

pub fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::text_content(vec![TextContent::from(s.into())])
}

pub fn json(value: &impl Serialize) -> CallToolResult {
    text(serde_json::to_string(value).unwrap_or_else(|_| "null".into()))
}

pub fn file_content(path: &str, bytes: &[u8]) -> CallToolResult {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let mime = match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "pdf" => Some("application/pdf"),
        _ => None,
    };
    if let Some(mime_type) = mime {
        let image = ImageContent::new(STANDARD.encode(bytes), mime_type.to_string(), None, None);
        CallToolResult {
            content: vec![ContentBlock::from(image)],
            is_error: None,
            meta: None,
            structured_content: None,
        }
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        text(s)
    } else {
        text(format!("[Unknown format] {}", STANDARD.encode(bytes)))
    }
}
