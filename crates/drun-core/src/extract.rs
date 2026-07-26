//! Plain-text extraction from binary document formats. Runs in-process on
//! bytes already in the checkpoint — no sandbox, no subprocess, no network.

use crate::error::RunnerError;
use std::path::Path;

pub(crate) struct TextExtractor;

impl TextExtractor {
    pub(crate) fn extract(path: &str, bytes: &[u8]) -> anyhow::Result<String> {
        match Self::extension(path) {
            Some("pdf") => pdf_extract::extract_text_from_mem(bytes)
                .map_err(|e| RunnerError::extraction_failed(path, e).into()),
            Some(ext) => Err(RunnerError::unsupported_extraction_format(ext).into()),
            None => Err(RunnerError::unsupported_extraction_format("(no extension)").into()),
        }
    }

    fn extension(path: &str) -> Option<&str> {
        Path::new(path).extension().and_then(|e| e.to_str())
    }

    /// Builds a byte-exact minimal single-page PDF containing `text` for test
    /// purposes only.
    #[cfg(test)]
    pub(crate) fn minimal_pdf_with_text(text: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut offsets = [0usize; 6];
        macro_rules! obj {
            ($num:expr, $body:expr) => {
                offsets[$num] = buf.len();
                buf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", $num, $body).as_bytes());
            };
        }

        buf.extend_from_slice(b"%PDF-1.4\n");
        obj!(1, "<< /Type /Catalog /Pages 2 0 R >>");
        obj!(2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
        obj!(
            3,
            "<< /Type /Page /Parent 2 0 R /Resources << /Font << /F1 4 0 R >> >> \
             /MediaBox [0 0 200 100] /Contents 5 0 R >>"
        );
        obj!(4, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
        let stream = format!("BT /F1 24 Tf 10 50 Td ({text}) Tj ET");
        obj!(
            5,
            format!(
                "<< /Length {} >>\nstream\n{stream}\nendstream",
                stream.len()
            )
        );

        let xref_start = buf.len();
        buf.extend_from_slice(format!("xref\n0 {}\n", offsets.len()).as_bytes());
        buf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets[1..] {
            buf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        buf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_start}\n%%EOF",
                offsets.len()
            )
            .as_bytes(),
        );
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rejects_an_unsupported_extension() {
        let err = TextExtractor::extract("notes.docx", b"whatever").unwrap_err();
        assert!(err.to_string().contains("docx"));
    }

    #[test]
    fn extract_rejects_a_path_with_no_extension() {
        let err = TextExtractor::extract("README", b"whatever").unwrap_err();
        assert!(err.to_string().contains("no extension"));
    }

    #[test]
    fn extract_surfaces_a_parse_error_for_bytes_that_are_not_a_real_pdf() {
        let err = TextExtractor::extract("fake.pdf", b"not a pdf").unwrap_err();
        assert!(err.to_string().contains("fake.pdf"));
    }

    #[test]
    fn extract_reads_text_out_of_a_minimal_real_pdf() {
        let pdf = TextExtractor::minimal_pdf_with_text("Hi");
        let text = TextExtractor::extract("greeting.pdf", &pdf).unwrap();
        assert!(
            text.contains("Hi"),
            "expected extracted text to contain 'Hi', got {text:?}"
        );
    }
}
