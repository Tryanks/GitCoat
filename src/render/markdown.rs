//! Markdown → safe HTML: rushdown (GFM) for parsing and rendering, syntect for
//! fenced code blocks, ammonia as the single sanitizing gate.
//!
//! Link and image targets are rewritten inside ammonia's attribute filter, so
//! every `href`/`src` that reaches the output has been through one function
//! ([`rewrite_url`]): relative targets resolve against the Markdown file's
//! directory and become `/blob` (links) or `/raw` (images) URLs pinned to the
//! commit; targets that climb above the repository root are dropped; anchors
//! are kept; absolute `http(s)`/`mailto` targets pass through and every link
//! gets `rel="noopener"`. Raw HTML is never emitted by rushdown
//! (`allows_unsafe` is off), and a final relative-URL policy in ammonia
//! refuses anything the rewrite did not produce.
//!
//! rushdown keeps `Rc`s internally, so the whole render is one synchronous
//! function; handlers call it between awaits, never across one.

use std::{borrow::Cow, collections::HashSet};

use ammonia::UrlRelative;
use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use rushdown::{
    as_kind_data,
    ast::{Arena, CodeBlock, Image, NodeRef, WalkStatus},
    matches_kind, new_markdown_to_html_string,
    parser::{self, GfmOptions},
    renderer::{self, NoRendererOptions, RenderNode, html},
};

use super::{highlight::highlight_block, push_escaped};
use crate::{
    app::url::{blob_url, raw_url},
    git::RepoPath,
};

/// What a Markdown document needs to know to produce repository links.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownCtx<'a> {
    /// The ref used in generated links: the pinned commit OID.
    pub ref_name: &'a str,
    /// The directory the Markdown file lives in; relative targets resolve
    /// against it.
    pub base_dir: &'a RepoPath,
}

/// Render `source` to sanitized HTML for a `.markdown-body` container.
pub fn render_markdown(source: &str, ctx: &MarkdownCtx<'_>) -> String {
    let html = markdown_to_html(source, ctx.base_dir);
    sanitize(&html, ctx)
}

// --- rushdown -----------------------------------------------------------------

/// GFM Markdown to (unsanitized) HTML with heading ids, highlighted fences and
/// images that would escape the repository degraded to their alt text.
fn markdown_to_html(source: &str, base_dir: &RepoPath) -> String {
    let base_dir = base_dir.clone();
    let render = new_markdown_to_html_string(
        parser::Options {
            auto_heading_ids: true,
            ..Default::default()
        },
        html::Options::default(),
        parser::gfm(GfmOptions::default()),
        html::renderer_extension(move |r: &mut html::Renderer<'_, String>| {
            r.add_node_renderer(|| CodeBlockRenderer, NoRendererOptions);
            r.add_node_renderer(
                |options: html::Options| ImageRenderer {
                    writer: html::Writer::with_options(options),
                    base_dir,
                },
                NoRendererOptions,
            );
        }),
    );
    let mut out = String::new();
    // Rendering into a String cannot fail; keep whatever was produced if it
    // somehow does.
    let _ = render(&mut out, source);
    out
}

/// Renders fenced and indented code blocks through syntect.
struct CodeBlockRenderer;

impl RenderNode<String> for CodeBlockRenderer {
    fn render_node<'a>(
        &self,
        w: &mut String,
        source: &'a str,
        arena: &'a Arena,
        node_ref: NodeRef,
        entering: bool,
        _context: &mut renderer::Context,
    ) -> rushdown::Result<WalkStatus> {
        if entering {
            let block = as_kind_data!(arena, node_ref, CodeBlock);
            let code: String = block.value().iter(source).collect();
            w.push_str(&highlight_block(&code, block.language_str(source)));
        }
        Ok(WalkStatus::Continue)
    }
}

impl<'r> renderer::NodeRenderer<'r, String> for CodeBlockRenderer {
    fn register_node_renderer_fn(self, nrr: &mut impl renderer::NodeRendererRegistry<'r, String>) {
        nrr.register_node_renderer_fn(
            std::any::TypeId::of::<CodeBlock>(),
            renderer::BoxRenderNode::new(self),
        );
    }
}

/// Renders images like rushdown does, except that an image whose relative
/// target escapes the repository is replaced by its alt text (rendered as
/// ordinary inline content). The `src` written here is the original target;
/// the sanitizer rewrites it.
struct ImageRenderer {
    writer: html::Writer,
    base_dir: RepoPath,
}

impl ImageRenderer {
    fn write_alt(
        &self,
        w: &mut String,
        source: &str,
        arena: &Arena,
        node_ref: NodeRef,
    ) -> rushdown::Result<()> {
        for child in arena[node_ref].children(arena) {
            if matches_kind!(arena[child], Text) {
                let text = as_kind_data!(arena, child, Text);
                self.writer.write(w, text.str(source))?;
            } else {
                self.write_alt(w, source, arena, child)?;
            }
        }
        Ok(())
    }
}

impl RenderNode<String> for ImageRenderer {
    fn render_node<'a>(
        &self,
        w: &mut String,
        source: &'a str,
        arena: &'a Arena,
        node_ref: NodeRef,
        entering: bool,
        _context: &mut renderer::Context,
    ) -> rushdown::Result<WalkStatus> {
        if !entering {
            return Ok(WalkStatus::Continue);
        }
        let image = as_kind_data!(arena, node_ref, Image);
        let destination = image.destination_str(source);
        if !has_scheme(destination) && resolve_relative(&self.base_dir, destination).is_none() {
            // Escapes the repository: fall back to the alt text as inline content.
            return Ok(WalkStatus::Continue);
        }
        w.push_str("<img src=\"");
        push_escaped(w, destination);
        w.push_str("\" alt=\"");
        self.write_alt(w, source, arena, node_ref)?;
        w.push('"');
        if let Some(title) = image.title_str(source) {
            w.push_str(" title=\"");
            self.writer.write(w, &title)?;
            w.push('"');
        }
        w.push('>');
        Ok(WalkStatus::SkipChildren)
    }
}

impl<'r> renderer::NodeRenderer<'r, String> for ImageRenderer {
    fn register_node_renderer_fn(self, nrr: &mut impl renderer::NodeRendererRegistry<'r, String>) {
        nrr.register_node_renderer_fn(
            std::any::TypeId::of::<Image>(),
            renderer::BoxRenderNode::new(self),
        );
    }
}

// --- URL resolution ---------------------------------------------------------------

/// Where a relative Markdown target points.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// A `#fragment`-only target: kept verbatim.
    Anchor,
    /// A path inside the repository, plus an optional fragment.
    Repo {
        path: RepoPath,
        fragment: Option<String>,
    },
}

/// Characters percent-encoded when a fragment is re-emitted.
const FRAGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'\'')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#');

/// Whether `url` starts with a URI scheme (`https:`, `mailto:`, `javascript:`).
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

/// Resolve a relative target against `base_dir`, normalizing `.` and `..`.
/// `None` when the target climbs above the repository root, is
/// protocol-relative, or is not a usable path.
fn resolve_relative(base_dir: &RepoPath, url: &str) -> Option<Target> {
    if url.starts_with("//") {
        return None;
    }
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    };
    let path = rest.split_once('?').map_or(rest, |(path, _)| path);
    if path.is_empty() {
        return fragment.map(|_| Target::Anchor);
    }
    let decoded = percent_decode_str(path).collect::<Vec<u8>>();
    let mut segments: Vec<&[u8]> = if decoded.starts_with(b"/") {
        Vec::new()
    } else {
        base_dir.components().collect()
    };
    for segment in decoded.split(|&b| b == b'/') {
        match segment {
            b"" | b"." => {}
            b".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }
    let path = RepoPath::from_bytes(segments.join(&b'/')).ok()?;
    Some(Target::Repo {
        path,
        fragment: fragment.map(|f| utf8_percent_encode(f, FRAGMENT).to_string()),
    })
}

/// Which kind of repository URL a rewritten target becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    Blob,
    Raw,
}

/// The final value for an `href`/`src`, or `None` to drop the attribute.
///
/// Absolute URLs pass through untouched (the sanitizer's scheme list decides
/// their fate); relative ones are resolved into repository URLs.
fn rewrite_url<'u>(ctx: &MarkdownCtx<'_>, kind: LinkKind, value: &'u str) -> Option<Cow<'u, str>> {
    if has_scheme(value) {
        return Some(Cow::Borrowed(value));
    }
    match resolve_relative(ctx.base_dir, value)? {
        Target::Anchor => Some(Cow::Borrowed(value)),
        Target::Repo { path, fragment } => {
            let path = path.as_str()?;
            let mut url = match kind {
                LinkKind::Blob => blob_url(ctx.ref_name, path),
                LinkKind::Raw => raw_url(ctx.ref_name, path),
            };
            if let Some(fragment) = fragment {
                url.push('#');
                url.push_str(&fragment);
            }
            Some(Cow::Owned(url))
        }
    }
}

// --- ammonia ------------------------------------------------------------------

/// The exact `style` values rushdown emits for aligned table cells.
const CELL_STYLES: [&str; 3] = [
    "text-align: left;",
    "text-align: center;",
    "text-align: right;",
];

/// Whether `token` is a fence language token we are willing to echo in a class.
fn is_language_token(token: &str) -> bool {
    !token.is_empty()
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'#' | b'.'))
}

/// Whether `classes` looks like syntect scope classes (`keyword control rust`),
/// the only classes a `<span>` may carry.
fn is_scope_classes(classes: &str) -> bool {
    !classes.is_empty()
        && classes.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b' ' | b'-' | b'_' | b'+' | b'.' | b'#')
        })
}

/// The relative-URL policy applied after the rewrite: only URLs the rewrite
/// produces may stay.
fn keep_rewritten_url(url: &str) -> Option<Cow<'_, str>> {
    (url.starts_with("/blob?") || url.starts_with("/raw?") || url.starts_with('#'))
        .then_some(Cow::Borrowed(url))
}

/// Sanitize rushdown's output down to the GFM element set, rewriting URLs on
/// the way (see the module docs).
fn sanitize(html: &str, ctx: &MarkdownCtx<'_>) -> String {
    let ref_name = ctx.ref_name.to_owned();
    let base_dir = ctx.base_dir.clone();
    let mut builder = ammonia::Builder::empty();
    builder
        .tags(HashSet::from([
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "p",
            "br",
            "hr",
            "blockquote",
            "ul",
            "ol",
            "li",
            "em",
            "strong",
            "del",
            "s",
            "code",
            "pre",
            "kbd",
            "sup",
            "sub",
            "a",
            "img",
            "table",
            "thead",
            "tbody",
            "tr",
            "th",
            "td",
            "input",
            "details",
            "summary",
            "dl",
            "dt",
            "dd",
            "span",
        ]))
        .tag_attributes(std::collections::HashMap::from([
            ("h1", HashSet::from(["id"])),
            ("h2", HashSet::from(["id"])),
            ("h3", HashSet::from(["id"])),
            ("h4", HashSet::from(["id"])),
            ("h5", HashSet::from(["id"])),
            ("h6", HashSet::from(["id"])),
            ("a", HashSet::from(["href", "title"])),
            (
                "img",
                HashSet::from(["src", "alt", "title", "width", "height"]),
            ),
            ("code", HashSet::from(["class"])),
            ("pre", HashSet::from(["class"])),
            ("span", HashSet::from(["class"])),
            ("ol", HashSet::from(["start"])),
            ("th", HashSet::from(["style"])),
            ("td", HashSet::from(["style"])),
            ("input", HashSet::from(["checked", "disabled"])),
        ]))
        .tag_attribute_values(std::collections::HashMap::from([(
            "input",
            std::collections::HashMap::from([("type", HashSet::from(["checkbox"]))]),
        )]))
        .set_tag_attribute_value("input", "disabled", "")
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .link_rel(Some("noopener"))
        .strip_comments(true)
        .attribute_filter(move |element, attribute, value| {
            let ctx = MarkdownCtx {
                ref_name: &ref_name,
                base_dir: &base_dir,
            };
            match (element, attribute) {
                ("a", "href") => rewrite_url(&ctx, LinkKind::Blob, value),
                ("img", "src") => rewrite_url(&ctx, LinkKind::Raw, value),
                ("code", "class") => value
                    .strip_prefix("language-")
                    .is_some_and(is_language_token)
                    .then_some(Cow::Borrowed(value)),
                ("pre", "class") => (value == "code-block").then_some(Cow::Borrowed(value)),
                ("span", "class") => is_scope_classes(value).then_some(Cow::Borrowed(value)),
                ("th" | "td", "style") => {
                    CELL_STYLES.contains(&value).then_some(Cow::Borrowed(value))
                }
                ("img", "width" | "height") => value
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    .then_some(Cow::Borrowed(value)),
                _ => Some(Cow::Borrowed(value)),
            }
        })
        // Belt and braces: only URLs the rewrite produced may stay relative.
        .url_relative(UrlRelative::Custom(Box::new(keep_rewritten_url)));
    builder.clean(html).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OID: &str = "0123456789abcdef0123456789abcdef01234567";

    fn render(source: &str, base: &str) -> String {
        let base_dir = RepoPath::parse(base).unwrap();
        render_markdown(
            source,
            &MarkdownCtx {
                ref_name: OID,
                base_dir: &base_dir,
            },
        )
    }

    #[test]
    fn headings_get_ids() {
        let html = render("# Hello World\n\n## Features\n\n## Features\n", "");
        assert!(
            html.contains("<h1 id=\"hello-world\">Hello World</h1>"),
            "{html}"
        );
        assert!(html.contains("<h2 id=\"features\">Features</h2>"), "{html}");
        assert!(
            html.contains("<h2 id=\"features-1\">Features</h2>"),
            "{html}"
        );
    }

    #[test]
    fn tables_task_lists_and_strikethrough() {
        let html = render(
            "| a | b |\n| - | :-: |\n| 1 | 2 |\n\n- [x] done\n- [ ] todo\n\n~~gone~~\n",
            "",
        );
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<th>a</th>"), "{html}");
        assert!(
            html.contains("<td style=\"text-align: center;\">2</td>"),
            "{html}"
        );
        assert!(
            html.contains("<input checked=\"\" disabled=\"\" type=\"checkbox\"> done"),
            "{html}"
        );
        assert!(
            html.contains("<input disabled=\"\" type=\"checkbox\"> todo"),
            "{html}"
        );
        assert!(html.contains("<del>gone</del>"), "{html}");
    }

    #[test]
    fn relative_links_and_images_are_rewritten() {
        let html = render(
            "[g](guide.md) [up](../README.md) [root](/src/main.rs) [dot](./x/../y.txt) ![l](img/logo.png)",
            "docs",
        );
        assert!(
            html.contains(&format!(
                "<a href=\"/blob?ref={OID}&amp;path=docs%2Fguide.md\" rel=\"noopener\">g</a>"
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}&amp;path=README.md\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}&amp;path=src%2Fmain.rs\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}&amp;path=docs%2Fy.txt\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "<img src=\"/raw?ref={OID}&amp;path=docs%2Fimg%2Flogo.png\" alt=\"l\">"
            )),
            "{html}"
        );
    }

    #[test]
    fn escaping_targets_are_dropped() {
        let html = render(
            "[up](../outside.md) ![pic](../x.png) [pr](//evil.example/x)",
            "",
        );
        assert!(html.contains("<a rel=\"noopener\">up</a>"), "{html}");
        assert!(!html.contains("outside.md"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("pic"), "alt text survives: {html}");
        assert!(!html.contains("evil.example"), "{html}");
        // From a subdirectory, `..` reaches the root, one more escapes.
        let html = render("[a](../a.md) [b](../../b.md)", "docs");
        assert!(html.contains("path=a.md"), "{html}");
        assert!(!html.contains("b.md"), "{html}");
    }

    #[test]
    fn anchors_fragments_and_encoded_paths() {
        let html = render(
            "[f](#features) [g](guide.md#sec) [sp](my%20file.md) [中](中文/文件.md)",
            "docs",
        );
        assert!(
            html.contains("<a href=\"#features\" rel=\"noopener\">f</a>"),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "href=\"/blob?ref={OID}&amp;path=docs%2Fguide.md#sec\""
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "href=\"/blob?ref={OID}&amp;path=docs%2Fmy+file.md\""
            )),
            "{html}"
        );
        assert!(
            html.contains(&format!(
                "href=\"/blob?ref={OID}&amp;path=docs%2F%E4%B8%AD%E6%96%87%2F%E6%96%87%E4%BB%B6.md\""
            )),
            "{html}"
        );
    }

    #[test]
    fn directories_link_to_blob_which_redirects() {
        let html = render("[d](.) [r](/) [s](src)", "docs");
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}&amp;path=docs\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}\"")),
            "{html}"
        );
        assert!(
            html.contains(&format!("href=\"/blob?ref={OID}&amp;path=docs%2Fsrc\"")),
            "{html}"
        );
    }

    #[test]
    fn dangerous_content_is_stripped() {
        let html = render(
            "<script>alert(1)</script>\n\n[x](javascript:alert(1)) [y](data:text/html,hi) [z](vbscript:x)\n\n<img src=x onerror=alert(1)>\n\n<a href=\"../evil\" onclick=\"x\">raw</a>\n\n<div style=\"color:red\">s</div>\n",
            "",
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("javascript:"), "{html}");
        assert!(!html.contains("data:"), "{html}");
        assert!(!html.contains("vbscript"), "{html}");
        assert!(!html.contains("onerror"), "{html}");
        assert!(!html.contains("onclick"), "{html}");
        assert!(!html.contains("style=\"color"), "{html}");
        assert!(!html.contains("<div"), "{html}");
        assert!(!html.contains("<!--"), "{html}");
        assert!(html.contains("<a rel=\"noopener\">x</a>"), "{html}");
    }

    #[test]
    fn absolute_links_pass_with_rel_noopener() {
        let html = render(
            "[e](https://example.com/a?b=c) <https://auto.example.com> [m](mailto:a@b.c) ![i](https://img.example.com/x.png)",
            "",
        );
        assert!(
            html.contains("<a href=\"https://example.com/a?b=c\" rel=\"noopener\">e</a>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<a href=\"https://auto.example.com\" rel=\"noopener\">https://auto.example.com</a>"
            ),
            "{html}"
        );
        assert!(html.contains("href=\"mailto:a@b.c\""), "{html}");
        assert!(
            html.contains("<img src=\"https://img.example.com/x.png\" alt=\"i\">"),
            "{html}"
        );
    }

    #[test]
    fn fenced_code_is_highlighted_and_escaped() {
        let html = render(
            "```rust\nfn main() { println!(\"<hi>\"); }\n```\n\n```nope\"x\n<b>\n```\n\n    indented <i>\n",
            "",
        );
        assert!(
            html.contains("<pre class=\"code-block\"><code class=\"language-rust\">"),
            "{html}"
        );
        assert!(html.contains("<span class=\"source rust\">"), "{html}");
        assert!(html.contains("&lt;hi&gt;"), "{html}");
        assert!(!html.contains("<hi>"), "{html}");
        assert!(!html.contains("language-nope"), "{html}");
        assert!(html.contains("&lt;b&gt;"), "{html}");
        assert!(html.contains("indented &lt;i&gt;"), "{html}");
    }

    #[test]
    fn code_class_only_for_languages() {
        let html = render("`x`", "");
        assert!(html.contains("<code>x</code>"), "{html}");
    }

    #[test]
    fn resolution_rules() {
        let base = RepoPath::parse("a/b").unwrap();
        let repo = |p: &str| Target::Repo {
            path: RepoPath::parse(p).unwrap(),
            fragment: None,
        };
        assert_eq!(resolve_relative(&base, "c.md"), Some(repo("a/b/c.md")));
        assert_eq!(resolve_relative(&base, "../c.md"), Some(repo("a/c.md")));
        assert_eq!(resolve_relative(&base, "../../c.md"), Some(repo("c.md")));
        assert_eq!(resolve_relative(&base, "../../../c.md"), None);
        assert_eq!(resolve_relative(&base, "/c.md"), Some(repo("c.md")));
        assert_eq!(
            resolve_relative(&base, "./c.md?x=1"),
            Some(repo("a/b/c.md"))
        );
        assert_eq!(resolve_relative(&base, "#x"), Some(Target::Anchor));
        assert_eq!(resolve_relative(&base, "//host/x"), None);
        assert_eq!(resolve_relative(&base, ""), None);
        assert_eq!(resolve_relative(&base, "%00"), None);
        assert_eq!(
            resolve_relative(&base, "c.md#a b"),
            Some(Target::Repo {
                path: RepoPath::parse("a/b/c.md").unwrap(),
                fragment: Some("a%20b".to_owned()),
            })
        );
        assert!(has_scheme("https://x"));
        assert!(has_scheme("mailto:x"));
        assert!(has_scheme("javascript:alert(1)"));
        assert!(!has_scheme("docs/a:b.md"));
        assert!(!has_scheme(":x"));
        assert!(!has_scheme("1a:x"));
    }
}
