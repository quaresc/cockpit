//! A mail's HTML made safe to draw (no scripts, styles, event handlers, frames or forms; links and images kept) and
//! turned into readable text for the terminal.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

/// The elements a mail hides from the reader (its preview text, spacers), by their inline style.
static HIDDEN: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    ["span", "div", "p", "td", "table"]
        .iter()
        .filter_map(|tag| {
            let style =
                r#"style\s*=\s*["'][^"']*(?:display\s*:\s*none|visibility\s*:\s*hidden|max-height\s*:\s*0|mso-hide\s*:\s*all|font-size\s*:\s*0)[^"']*["']"#;
            Regex::new(&format!(r"(?is)<{tag}\b[^>]*{style}[^>]*>.*?</{tag}>")).ok()
        })
        .collect()
});

/// The tags a mail lays its page out with: tables, cells and boxes, each a block of text of its own once flattened.
static LAYOUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</?(?:table|tbody|thead|tfoot|tr|td|th|div|section|article|header|footer|center|main)\b[^>]*>").unwrap());

/// A paragraph holding nothing to read.
static EMPTY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<p>(?:\s|&nbsp;|&zwnj;|\x{a0}|\x{200c}|\x{34f})*</p>").unwrap());

/// A link with nothing to click on: its image dropped, or a spacer.
static BARE_LINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<a\b[^>]*>\s*</a>").unwrap());

/// The HTML with only what a mail needs to read: text, its structure, links and images, without what it hides.
pub fn sanitize(html: &str) -> String {
    let shown = HIDDEN.iter().fold(html.to_string(), |html, hidden| hidden.replace_all(&html, "").into_owned());
    let dropped: HashSet<&str> = ["script", "style", "title", "noscript", "iframe", "object", "embed", "form", "select", "textarea", "button"].into();
    ammonia::Builder::default().clean_content_tags(dropped).add_url_schemes(["cid"]).clean(&shown).to_string()
}

/// The HTML as a browser page shows it, its layout kept: inline styles and the table attributes mails lay out with stay,
/// scripts, frames, forms and handlers still go.
pub fn sanitize_page(html: &str) -> String {
    let dropped: HashSet<&str> = ["script", "style", "title", "noscript", "iframe", "object", "embed", "form", "select", "textarea", "button"].into();
    let layout: HashSet<&str> = ["style", "align", "valign", "bgcolor", "width", "height", "border", "cellpadding", "cellspacing", "color", "dir"].into();
    ammonia::Builder::default()
        .clean_content_tags(dropped)
        .add_generic_attributes(layout)
        .add_tags(["font", "center"])
        .add_url_schemes(["cid"])
        .clean(html)
        .to_string()
}

/// The sanitized HTML as Herdr's TextView draws it, which knows paragraphs but not a mail's layout: each table cell, box
/// and line break starts a paragraph, the empty ones go.
pub fn for_view(clean: &str) -> String {
    let lines = clean.replace("<br>", "</p><p>");
    let flat = LAYOUT.replace_all(&lines, "</p><p>");
    let tidy = sanitize(&format!("<p>{flat}</p>"));
    let linked = BARE_LINK.replace_all(&tidy, "");
    EMPTY.replace_all(&linked, "").into_owned()
}

/// The HTML as text in lines of at most `width` columns, its links listed at the end.
pub fn to_text(html: &str, width: usize) -> String {
    let clean = sanitize(html);
    html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
        .link_footnotes(true)
        .string_from_read(clean.as_bytes(), width.max(20))
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// A message's text: its own plain text when it has some, else its HTML's.
pub fn text_of(text: &str, html: &str, width: usize) -> String {
    if text.trim().is_empty() && !html.trim().is_empty() { to_text(html, width) } else { text.trim_end().replace("\r\n", "\n") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_styles_handlers_frames_and_forms_go_links_and_images_stay() {
        let html = r#"<html><head><title>T</title><style>p{color:red}</style></head><body>
            <p onclick="steal()" style="color:red">Hello <b>Maya</b></p><script>alert(1)</script>
            <iframe src="https://evil.example.com"></iframe><form action="https://evil.example.com"><input name="pw"></form>
            <a href="https://acme.example/pr/4" onmouseover="x()">the PR</a><img src="https://acme.example/logo.png" onerror="x()" alt="logo">
            <a href="javascript:alert(1)">bad</a></body></html>"#;
        let clean = sanitize(html);
        for gone in ["script", "alert", "<style", "color:red", "onclick", "onmouseover", "onerror", "iframe", "<form", "<input", "javascript:", ">T<"] {
            assert!(!clean.contains(gone), "{gone} left in {clean}");
        }
        assert!(clean.contains(r#"href="https://acme.example/pr/4""#), "{clean}");
        assert!(clean.contains(r#"src="https://acme.example/logo.png""#), "{clean}");
        assert!(clean.contains("<b>Maya</b>"));
        assert_eq!(for_view(&sanitize("<p>Thanks,<br>Maya</p>")), "<p>Thanks,</p><p>Maya</p>");
    }

    #[test]
    fn a_page_keeps_the_layout_but_not_the_scripts() {
        let page = sanitize_page(
            r#"<table width="600" style="background:#fff"><tr><td align="center" onclick="x()"><img src="https://a/logo.png"><script>alert(1)</script></td></tr></table>"#,
        );
        assert!(page.contains(r#"width="600""#) && page.contains("background:#fff") && page.contains(r#"align="center""#), "{page}");
        assert!(page.contains("https://a/logo.png") && !page.contains("onclick") && !page.contains("alert"), "{page}");
    }

    #[test]
    fn a_laid_out_mail_reads_as_paragraphs_without_its_hidden_preview() {
        let html = r#"<div style="display:none;max-height:0">Preview text&zwnj;&nbsp;&zwnj;</div>
            <table><tr><td>J</td><td>Jonathan mentioned you in <a href="https://notion.so/p">the ADR</a></td></tr>
            <tr><td><div>Go to page</div><div><a href="https://notion.so/s">Settings</a></div><a href="https://notion.so/x"> </a></td></tr></table>"#;
        let view = for_view(&sanitize(html));
        assert!(!view.contains("Preview text"), "{view}");
        assert_eq!(
            view,
            r#"<p>J</p><p>Jonathan mentioned you in <a href="https://notion.so/p" rel="noopener noreferrer">the ADR</a></p><p>Go to page</p><p><a href="https://notion.so/s" rel="noopener noreferrer">Settings</a></p>"#
        );
    }

    #[test]
    fn html_reads_as_text_with_its_links() {
        let text = to_text("<h1>Release</h1><p>Shipped <a href=\"https://acme.example/r/2\">2.0</a> today.</p><ul><li>one</li><li>two</li></ul>", 60);
        assert!(text.contains("Release"), "{text}");
        assert!(text.contains("Shipped 2.0"), "{text}");
        assert!(text.contains("https://acme.example/r/2"), "{text}");
        assert!(text.contains("one") && text.contains("two"));
        assert!(!text.contains('<'));
        assert_eq!(text_of("Plain\r\nbody\n", "<p>html</p>", 60), "Plain\nbody");
        assert_eq!(text_of("  ", "<p>html</p>", 60), "html");
    }
}
