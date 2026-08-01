//! Stateless text-parsing helpers shared across checkpoint file and stream
//! content: extracting plain text from binary document formats, and
//! grep-style pattern search over text already in memory. No sandbox, no
//! subprocess, no network.

use crate::error::RunnerError;
use regex::Regex;
use serde::Serialize;
use std::path::Path;

pub struct TextParserUtilities;

impl TextParserUtilities {
    pub(crate) fn extract(path: &str, bytes: &[u8]) -> anyhow::Result<String> {
        match Self::extension(path) {
            Some("pdf") => pdf_extract::extract_text_from_mem(bytes)
                .map_err(|e| RunnerError::extraction_failed(path, e).into()),
            Some(ext) => Err(RunnerError::unsupported_extraction_format(ext).into()),
            None => Err(RunnerError::unsupported_extraction_format("(no extension)").into()),
        }
    }

    /// Greps `bytes` line by line for `pattern`, a case-sensitive regex.
    /// Requires UTF-8 content; pattern search over binary data is rejected.
    /// Each match reports its 1-based line number and the byte offset its
    /// line starts at, so callers can jump straight to it with a follow-up
    /// offset/limit read.
    pub fn grep(bytes: &[u8], pattern: &str) -> Result<GrepResult, RunnerError> {
        let text = std::str::from_utf8(bytes).map_err(|_| {
            RunnerError::binary_content("pattern search requires UTF-8 text content")
        })?;
        let re = Regex::new(pattern).map_err(|e| RunnerError::invalid_pattern(e.to_string()))?;

        let mut matches = Vec::new();
        let mut byte_offset = 0;
        for (i, raw_line) in text.split_inclusive('\n').enumerate() {
            let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            if re.is_match(line) {
                matches.push(GrepMatch {
                    line_number: i + 1,
                    byte_offset,
                    line: line.to_string(),
                });
            }
            byte_offset += raw_line.len();
        }

        let total_matches = matches.len();
        Ok(GrepResult {
            matches,
            total_matches,
        })
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

#[derive(Debug, Serialize)]
pub struct GrepMatch {
    pub line_number: usize,
    pub byte_offset: usize,
    pub line: String,
}

#[derive(Debug, Serialize)]
pub struct GrepResult {
    pub matches: Vec<GrepMatch>,
    pub total_matches: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_rejects_an_unsupported_extension() {
        let err = TextParserUtilities::extract("notes.docx", b"whatever").unwrap_err();
        assert!(err.to_string().contains("docx"));
    }

    #[test]
    fn extract_rejects_a_path_with_no_extension() {
        let err = TextParserUtilities::extract("README", b"whatever").unwrap_err();
        assert!(err.to_string().contains("no extension"));
    }

    #[test]
    fn extract_surfaces_a_parse_error_for_bytes_that_are_not_a_real_pdf() {
        let err = TextParserUtilities::extract("fake.pdf", b"not a pdf").unwrap_err();
        assert!(err.to_string().contains("fake.pdf"));
    }

    #[test]
    fn extract_reads_text_out_of_a_minimal_real_pdf() {
        let pdf = TextParserUtilities::minimal_pdf_with_text("Hi");
        let text = TextParserUtilities::extract("greeting.pdf", &pdf).unwrap();
        assert!(
            text.contains("Hi"),
            "expected extracted text to contain 'Hi', got {text:?}"
        );
    }

    #[test]
    fn grep_returns_only_matching_lines_with_one_based_line_numbers() {
        let result =
            TextParserUtilities::grep(b"alpha\nbeta\ngamma\nbeta again\n", "beta").unwrap();
        assert_eq!(result.total_matches, 2);
        assert_eq!(result.matches[0].line_number, 2);
        assert_eq!(result.matches[0].line, "beta");
        assert_eq!(result.matches[1].line_number, 4);
        assert_eq!(result.matches[1].line, "beta again");
    }

    #[test]
    fn grep_reports_byte_offset_of_the_start_of_each_matching_line() {
        let result = TextParserUtilities::grep(b"alpha\nbeta\ngamma\n", "gamma").unwrap();
        assert_eq!(result.matches[0].byte_offset, "alpha\nbeta\n".len());
    }

    #[test]
    fn grep_strips_carriage_returns_from_crlf_line_endings() {
        let result = TextParserUtilities::grep(b"alpha\r\nbeta\r\n", "beta").unwrap();
        assert_eq!(result.matches[0].line, "beta");
    }

    #[test]
    fn grep_matches_a_final_line_with_no_trailing_newline() {
        let result = TextParserUtilities::grep(b"alpha\nbeta", "beta").unwrap();
        assert_eq!(result.total_matches, 1);
        assert_eq!(result.matches[0].line_number, 2);
    }

    #[test]
    fn grep_is_case_sensitive_by_default() {
        let result = TextParserUtilities::grep(b"Alpha\n", "alpha").unwrap();
        assert_eq!(result.total_matches, 0);
    }

    #[test]
    fn grep_honors_an_inline_case_insensitive_flag() {
        let result = TextParserUtilities::grep(b"Alpha\n", "(?i)alpha").unwrap();
        assert_eq!(result.total_matches, 1);
    }

    #[test]
    fn grep_returns_zero_matches_for_a_pattern_not_present() {
        let result = TextParserUtilities::grep(b"alpha\nbeta\n", "zzz").unwrap();
        assert_eq!(result.total_matches, 0);
        assert!(result.matches.is_empty());
    }

    #[test]
    fn grep_rejects_an_invalid_regex() {
        let err = TextParserUtilities::grep(b"alpha\n", "(unclosed").unwrap_err();
        assert!(matches!(err, RunnerError::InvalidPattern(_)));
    }

    #[test]
    fn grep_rejects_non_utf8_content() {
        let err = TextParserUtilities::grep(&[0xff, 0xfe, 0xfd], "anything").unwrap_err();
        assert!(matches!(err, RunnerError::BinaryContent(_)));
    }
}
