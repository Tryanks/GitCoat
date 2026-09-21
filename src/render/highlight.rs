//! Syntax highlighting with syntect, emitting CSS classes
//! (`ClassStyle::Spaced`) that `static/app.css` styles for both themes.

use std::sync::LazyLock;

use syntect::{
    html::{ClassStyle, line_tokens_to_classed_spans},
    parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet},
};

use super::{escape_html, push_escaped};
use crate::limits::HIGHLIGHT_MAX_BYTES;

/// The bundled grammars, built for input lines that keep their `\n`.
static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// What is known about a text that helps pick its grammar.
#[derive(Debug, Clone, Copy, Default)]
pub struct Hint<'a> {
    /// The file name (`Makefile`, `main.rs`).
    pub filename: Option<&'a str>,
    /// An explicit language token, e.g. from a Markdown fence (`rust`, `sh`).
    pub language: Option<&'a str>,
    /// The first line of the text, for shebangs and mode lines.
    pub first_line: Option<&'a str>,
}

impl<'a> Hint<'a> {
    /// A hint from a file name and the text itself.
    pub fn for_file(filename: &'a str, text: &'a str) -> Self {
        Self {
            filename: Some(filename),
            language: None,
            first_line: text.lines().next(),
        }
    }
}

/// Pick the grammar for `hint`: explicit language, then extension, then the
/// whole file name (`Makefile`), then the first line. `None` means plain text.
fn find_syntax(hint: &Hint<'_>) -> Option<&'static SyntaxReference> {
    let set = &*SYNTAX_SET;
    if let Some(language) = hint.language.map(str::trim).filter(|l| !l.is_empty())
        && let Some(syntax) = set.find_syntax_by_token(language)
    {
        return Some(syntax);
    }
    if let Some(name) = hint.filename {
        let extension = name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
        if !extension.is_empty()
            && let Some(syntax) = set.find_syntax_by_extension(extension)
        {
            return Some(syntax);
        }
        if let Some(syntax) = set.find_syntax_by_extension(name) {
            return Some(syntax);
        }
    }
    hint.first_line
        .and_then(|line| set.find_syntax_by_first_line(line))
        .filter(|syntax| syntax.name != "Plain Text")
}

/// Split `text` into lines without their terminators. A trailing newline does
/// not produce an extra empty line; an empty text has no lines.
pub fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n')
        .map(|line| line.strip_suffix('\n').unwrap_or(line))
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// Highlight `text`, returning one self-contained escaped HTML fragment per
/// line (see [`split_lines`]). Unknown grammars, texts above
/// `HIGHLIGHT_MAX_BYTES` and any highlighter failure fall back to plain
/// escaped lines, so the output never contains unescaped input.
pub fn highlight_lines(text: &str, hint: Hint<'_>) -> Vec<String> {
    let lines = split_lines(text);
    if text.len() > HIGHLIGHT_MAX_BYTES {
        return lines.iter().map(|line| escape_html(line)).collect();
    }
    let Some(syntax) = find_syntax(&hint) else {
        return lines.iter().map(|line| escape_html(line)).collect();
    };
    match highlight_with(syntax, &lines) {
        Some(html) => html,
        None => lines.iter().map(|line| escape_html(line)).collect(),
    }
}

/// Run the grammar over every line. Spans left open at a line end are closed
/// there and reopened on the next line, so each fragment is well-formed on its
/// own (each line lives in its own table cell).
fn highlight_with(syntax: &SyntaxReference, lines: &[&str]) -> Option<Vec<String>> {
    let set = &*SYNTAX_SET;
    let mut state = ParseState::new(syntax);
    let mut stack = ScopeStack::new();
    let mut out = Vec::with_capacity(lines.len());
    let mut with_newline = String::new();
    for line in lines {
        with_newline.clear();
        with_newline.push_str(line);
        with_newline.push('\n');
        // Spans carried over from previous lines.
        let mut html = String::new();
        for scope in stack.as_slice() {
            html.push_str("<span class=\"");
            push_escaped(&mut html, &scope.build_string().replace('.', " "));
            html.push_str("\">");
        }
        let ops = state.parse_line(&with_newline, set).ok()?;
        let (spans, _delta) =
            line_tokens_to_classed_spans(&with_newline, &ops, ClassStyle::Spaced, &mut stack)
                .ok()?;
        // The fragment ends with the "\n"; drop it, the table row is the break.
        html.push_str(spans.strip_suffix('\n').unwrap_or(&spans));
        // `stack` now holds exactly the scopes still open, one span each.
        for _ in 0..stack.len() {
            html.push_str("</span>");
        }
        out.push(html);
    }
    Some(out)
}

/// Highlight a fenced code block for Markdown output. `lang` is the fence
/// token; unknown tokens give escaped plain text. The result is a complete
/// `<pre class="code-block"><code class="language-…">…</code></pre>`.
pub fn highlight_block(code: &str, lang: Option<&str>) -> String {
    let lang = lang.map(str::trim).filter(|l| {
        !l.is_empty()
            && l.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'#' | b'.'))
    });
    let hint = Hint {
        filename: None,
        language: lang,
        first_line: None,
    };
    let lines = highlight_lines(code, hint);
    let mut out = String::from("<pre class=\"code-block\"><code");
    if let Some(lang) = lang {
        out.push_str(" class=\"language-");
        push_escaped(&mut out, lang);
        out.push('"');
    }
    out.push('>');
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.push_str("</code></pre>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_lines_like_an_editor() {
        assert_eq!(split_lines(""), Vec::<&str>::new());
        assert_eq!(split_lines("a"), vec!["a"]);
        assert_eq!(split_lines("a\n"), vec!["a"]);
        assert_eq!(split_lines("a\n\n"), vec!["a", ""]);
        assert_eq!(split_lines("a\r\nb"), vec!["a", "b"]);
    }

    #[test]
    fn rust_gets_classes_and_is_escaped() {
        let text = "fn main() {\n    println!(\"<hi>\");\n}\n";
        let lines = highlight_lines(text, Hint::for_file("main.rs", text));
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("class=\"source rust\""), "{}", lines[0]);
        assert!(
            lines[0].contains("storage type function rust"),
            "{}",
            lines[0]
        );
        assert!(lines[1].contains("&lt;hi&gt;"), "{}", lines[1]);
        assert!(!lines[1].contains("<hi>"));
        for line in &lines {
            assert!(!line.contains('\n'));
            let opens = line.matches("<span").count();
            let closes = line.matches("</span>").count();
            assert_eq!(opens, closes, "balanced spans in {line}");
        }
    }

    #[test]
    fn multi_line_constructs_reopen_spans() {
        let text = "/* a\n b */\nlet x = 1;\n";
        let lines = highlight_lines(text, Hint::for_file("x.rs", text));
        assert!(
            lines[1].starts_with("<span class=\"source rust\"><span class=\"comment block rust\">"),
            "{}",
            lines[1]
        );
        assert_eq!(
            lines[1].matches("<span").count(),
            lines[1].matches("</span>").count()
        );
    }

    #[test]
    fn unknown_extension_is_plain_escaped() {
        let text = "<b>&\n";
        let lines = highlight_lines(text, Hint::for_file("file.unknownext", text));
        assert_eq!(lines, vec!["&lt;b&gt;&amp;"]);
    }

    #[test]
    fn whole_file_names_and_shebangs_are_recognised() {
        let text = "all:\n\techo hi\n";
        let lines = highlight_lines(text, Hint::for_file("Makefile", text));
        assert!(lines[0].contains("<span"), "{}", lines[0]);
        let text = "#!/bin/sh\necho hi\n";
        let lines = highlight_lines(text, Hint::for_file("run", text));
        assert!(lines[0].contains("<span"), "{}", lines[0]);
    }

    #[test]
    fn oversized_text_is_only_escaped() {
        let text = format!("// <x>\n{}", "a\n".repeat(HIGHLIGHT_MAX_BYTES));
        let lines = highlight_lines(&text, Hint::for_file("big.rs", &text));
        assert_eq!(lines[0], "// &lt;x&gt;");
    }

    #[test]
    fn blocks_wrap_pre_code_with_language_class() {
        let html = highlight_block("fn main() {}\n", Some("rust"));
        assert!(html.starts_with("<pre class=\"code-block\"><code class=\"language-rust\">"));
        assert!(html.contains("<span class=\"source rust\">"));
        assert!(html.ends_with("</code></pre>\n"));
        let html = highlight_block("<script>", Some("nope\" onload=\"x"));
        assert!(!html.contains("language-"), "{html}");
        assert!(html.contains("&lt;script&gt;"));
        let html = highlight_block("x", None);
        assert_eq!(html, "<pre class=\"code-block\"><code>x</code></pre>\n");
    }
}
