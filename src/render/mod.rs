//! Turning repository content into safe HTML: syntax highlighting, Markdown
//! rendering and content classification.
//!
//! Everything returned by this module as a `String` is complete, escaped HTML
//! that pages embed with `Unescaped::new_unchecked`. Input text never reaches
//! the output unescaped.

pub mod content;
pub mod highlight;
pub mod markdown;

/// Escape text for an HTML text node or a double-quoted attribute value.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    push_escaped(&mut out, text);
    out
}

/// Append `text` to `out`, escaping `<`, `>`, `&`, `"` and `'`.
pub fn push_escaped(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_characters() {
        assert_eq!(
            escape_html("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
        assert_eq!(escape_html("plain 中文"), "plain 中文");
    }
}
