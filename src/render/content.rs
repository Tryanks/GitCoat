//! Deciding how a blob is shown: text, Markdown, image, binary or empty.
//!
//! Detection never trusts the file extension for anything security relevant:
//! images are recognised by magic bytes only, and HTML/SVG/XML are plain text
//! (shown as source, served as `text/plain`).

use crate::limits::IMAGE_PREVIEW_MAX_BYTES;

/// How many leading bytes the text/binary check looks at.
const SNIFF_BYTES: usize = 8 * 1024;

/// The presentation chosen for a blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentKind {
    /// Source or plain text (`lang_hint` is the file name for the highlighter).
    Text { lang_hint: String },
    /// A Markdown document (`.md`, `.markdown`).
    Markdown,
    /// A raster image, previewable inline.
    Image { mime: &'static str },
    /// Anything else.
    Binary,
    /// A zero-length blob.
    Empty,
}

/// The image format recognised from a blob's first bytes, if any.
pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// Whether the leading bytes look like text: no NUL in the first 8 KiB.
pub fn looks_like_text(prefix: &[u8]) -> bool {
    !prefix[..prefix.len().min(SNIFF_BYTES)].contains(&0)
}

/// Whether `filename` names a Markdown document.
pub fn is_markdown_name(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.ends_with(".md") || lower.ends_with(".markdown")
}

/// Classify a blob from its leading bytes (`prefix`, at least the first 8 KiB
/// when available), its full `size` and its `filename`.
pub fn classify(prefix: &[u8], size: u64, filename: &str) -> ContentKind {
    if size == 0 {
        return ContentKind::Empty;
    }
    if let Some(mime) = sniff_image(prefix) {
        return if size <= IMAGE_PREVIEW_MAX_BYTES as u64 {
            ContentKind::Image { mime }
        } else {
            ContentKind::Binary
        };
    }
    if !looks_like_text(prefix) {
        return ContentKind::Binary;
    }
    if is_markdown_name(filename) {
        return ContentKind::Markdown;
    }
    ContentKind::Text {
        lang_hint: filename.to_owned(),
    }
}

/// Decode text bytes, replacing invalid UTF-8; the flag reports whether any
/// replacement happened.
pub fn decode_text(bytes: &[u8]) -> (String, bool) {
    match String::from_utf8_lossy(bytes) {
        std::borrow::Cow::Borrowed(text) => (text.to_owned(), false),
        std::borrow::Cow::Owned(text) => (text, true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_by_magic_bytes_only() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\n...."), Some("image/png"));
        assert_eq!(sniff_image(b"\xff\xd8\xff\xe0"), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a"), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<svg xmlns"), None);
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WAVE"), None);
        assert!(matches!(
            classify(b"<svg></svg>", 11, "icon.png"),
            ContentKind::Text { .. }
        ));
        assert_eq!(
            classify(b"\x89PNG\r\n\x1a\n", 8, "logo.txt"),
            ContentKind::Image { mime: "image/png" }
        );
        assert_eq!(
            classify(
                b"\x89PNG\r\n\x1a\n",
                IMAGE_PREVIEW_MAX_BYTES as u64 + 1,
                "x.png"
            ),
            ContentKind::Binary
        );
    }

    #[test]
    fn text_binary_markdown_empty() {
        assert_eq!(classify(b"", 0, "empty.txt"), ContentKind::Empty);
        assert_eq!(classify(b"\x00\x01", 2, "a.txt"), ContentKind::Binary);
        assert_eq!(classify(b"# hi", 4, "README.md"), ContentKind::Markdown);
        assert_eq!(
            classify(b"# hi", 4, "notes.MARKDOWN"),
            ContentKind::Markdown
        );
        assert_eq!(
            classify(b"<html>", 6, "page.html"),
            ContentKind::Text {
                lang_hint: "page.html".to_owned()
            }
        );
        // A NUL beyond the sniffed window does not flip the decision.
        let mut long = vec![b'a'; SNIFF_BYTES];
        long.push(0);
        assert!(matches!(
            classify(&long, long.len() as u64, "x"),
            ContentKind::Text { .. }
        ));
    }

    #[test]
    fn lossy_decoding_is_flagged() {
        assert_eq!(decode_text(b"ok"), ("ok".to_owned(), false));
        let (text, lossy) = decode_text(b"a\xffb");
        assert!(lossy);
        assert_eq!(text, "a\u{fffd}b");
    }
}
