//! A mail's HTML made safe to draw (no scripts, styles, event handlers, frames or forms; links and images kept) and
//! turned into readable text for the terminal.

use std::collections::HashSet;

/// The HTML with only what a mail needs to read: text, its structure, links and images.
pub fn sanitize(html: &str) -> String {
    let dropped: HashSet<&str> = ["script", "style", "title", "noscript", "iframe", "object", "embed", "form", "select", "textarea", "button"].into();
    ammonia::Builder::default().clean_content_tags(dropped).add_url_schemes(["cid"]).clean(html).to_string()
}

/// The sanitized HTML as Herdr's TextView draws it: it skips `<br>`, so each line break starts a paragraph instead.
pub fn for_view(clean: &str) -> String {
    if clean.contains("<br>") { sanitize(&clean.replace("<br>", "</p><p>")) } else { clean.to_string() }
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
