//! Syntax colours for code blocks and diffs: tree-sitter grammars and their highlight queries in a Vesper palette (orange
//! functions and numbers, teal strings, grey keywords, dim comments). A language's queries are compiled only when a text
//! in it is painted and go with their `Grammars`; a code block's colours are kept instead.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{LazyLock, Mutex, Once, PoisonError};

use ratatui::style::{Color, Modifier, Style};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Code blocks already painted, by language and text, so a new width or a redraw paints nothing again.
static PAINTED: LazyLock<Mutex<HashMap<u64, Lines>>> = LazyLock::new(Default::default);
const PAINTED_KEPT: usize = 64;

/// Lines of coloured pieces.
pub type Lines = Vec<Vec<(String, Style)>>;

/// The capture names coloured, with their colour and look; a capture takes the longest of these its name starts with.
const NAMES: &[(&str, Option<u32>, Modifier)] = &[
    ("comment", Some(0x6E6E74), Modifier::ITALIC),
    ("string", Some(0x99FFE4), Modifier::empty()),
    ("character", Some(0x99FFE4), Modifier::empty()),
    ("escape", Some(0x99FFE4), Modifier::empty()),
    ("string.special.key", Some(0xFFFFFF), Modifier::empty()),
    ("string.special.symbol", Some(0xFFC799), Modifier::empty()),
    ("number", Some(0xFFC799), Modifier::empty()),
    ("float", Some(0xFFC799), Modifier::empty()),
    ("boolean", Some(0xFFC799), Modifier::empty()),
    ("constant", Some(0xFFC799), Modifier::empty()),
    ("keyword", Some(0xA0A0A0), Modifier::empty()),
    ("include", Some(0xA0A0A0), Modifier::empty()),
    ("conditional", Some(0xA0A0A0), Modifier::empty()),
    ("repeat", Some(0xA0A0A0), Modifier::empty()),
    ("exception", Some(0xA0A0A0), Modifier::empty()),
    ("storageclass", Some(0xA0A0A0), Modifier::empty()),
    ("type.qualifier", Some(0xA0A0A0), Modifier::empty()),
    ("function", Some(0xFFC799), Modifier::empty()),
    ("method", Some(0xFFC799), Modifier::empty()),
    ("type", Some(0xFFFFFF), Modifier::empty()),
    ("constructor", Some(0xFFFFFF), Modifier::empty()),
    ("module", Some(0xFFFFFF), Modifier::empty()),
    ("namespace", Some(0xFFFFFF), Modifier::empty()),
    ("tag", Some(0xFFC799), Modifier::empty()),
    ("attribute", Some(0xFFC799), Modifier::empty()),
    ("variable.parameter", Some(0xD2D2D2), Modifier::ITALIC),
    ("variable.builtin", Some(0xD2D2D2), Modifier::ITALIC),
    ("parameter", Some(0xD2D2D2), Modifier::ITALIC),
    ("punctuation.special", Some(0xA0A0A0), Modifier::empty()),
    ("text.title", Some(0xFFC799), Modifier::BOLD),
    ("markup.heading", Some(0xFFC799), Modifier::BOLD),
    ("text.literal", Some(0x99FFE4), Modifier::empty()),
    ("markup.raw", Some(0x99FFE4), Modifier::empty()),
    ("text.uri", Some(0xA0A0FF), Modifier::UNDERLINED),
    ("markup.link.url", Some(0xA0A0FF), Modifier::UNDERLINED),
    ("text.reference", Some(0xFFC799), Modifier::empty()),
    ("text.emphasis", None, Modifier::ITALIC),
    ("markup.italic", None, Modifier::ITALIC),
    ("text.strong", None, Modifier::BOLD),
    ("markup.bold", None, Modifier::BOLD),
];

fn style(index: usize) -> Style {
    let Some((_, color, look)) = NAMES.get(index) else { return Style::new() };
    let style = Style::new().add_modifier(*look);
    color.map_or(style, |hex| style.fg(Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)))
}

/// A language a grammar is bundled for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Lang {
    Bash,
    C,
    Cpp,
    Css,
    Elixir,
    Go,
    Html,
    Java,
    JavaScript,
    Json,
    Kotlin,
    Markdown,
    MarkdownInline,
    Python,
    Ruby,
    Rust,
    Sql,
    Swift,
    Toml,
    Tsx,
    TypeScript,
    Yaml,
}

impl Lang {
    /// The language a fence's info, a file's extension or name, or an injection names; None for plain text.
    fn find(token: &str) -> Option<Self> {
        Some(match token.trim().to_lowercase().as_str() {
            "sh" | "bash" | "zsh" | "shell" | "console" | "bashrc" | "zshrc" | "profile" => Self::Bash,
            "c" | "h" => Self::C,
            "cpp" | "c++" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Self::Cpp,
            "css" | "scss" => Self::Css,
            "ex" | "exs" | "elixir" => Self::Elixir,
            "go" | "golang" => Self::Go,
            "html" | "htm" | "xhtml" => Self::Html,
            "java" => Self::Java,
            "js" | "javascript" | "jsx" | "mjs" | "cjs" => Self::JavaScript,
            "json" | "jsonc" | "json5" => Self::Json,
            "kt" | "kts" | "kotlin" => Self::Kotlin,
            "md" | "markdown" => Self::Markdown,
            "markdown_inline" => Self::MarkdownInline,
            "py" | "python" | "pyi" => Self::Python,
            "rb" | "ruby" | "rake" | "gemspec" | "gemfile" | "rakefile" => Self::Ruby,
            "rs" | "rust" => Self::Rust,
            "sql" | "psql" | "pgsql" | "postgres" | "postgresql" | "mysql" | "sqlite" => Self::Sql,
            "swift" => Self::Swift,
            "toml" => Self::Toml,
            "tsx" => Self::Tsx,
            "ts" | "typescript" | "mts" | "cts" => Self::TypeScript,
            "yaml" | "yml" => Self::Yaml,
            _ => return None,
        })
    }

    /// The languages it embeds, loaded with it: an injection only finds a grammar loaded before the paint starts.
    fn embeds(self) -> &'static [Lang] {
        match self {
            Self::Html => &[Self::JavaScript, Self::Css],
            Self::Markdown => &[Self::MarkdownInline],
            _ => &[],
        }
    }

    /// The grammar with its queries compiled; TypeScript's and C++'s extend JavaScript's and C's.
    fn configuration(self) -> Option<HighlightConfiguration> {
        use_mimalloc();
        use tree_sitter_javascript as js;
        use tree_sitter_typescript as ts;
        let joined = |parts: &[&str]| parts.join("\n");
        let (language, name, highlights, injections, locals): (tree_sitter::Language, &str, String, &str, String) = match self {
            Self::Bash => (tree_sitter_bash::LANGUAGE.into(), "bash", tree_sitter_bash::HIGHLIGHT_QUERY.into(), "", String::new()),
            Self::C => (tree_sitter_c::LANGUAGE.into(), "c", tree_sitter_c::HIGHLIGHT_QUERY.into(), "", String::new()),
            Self::Cpp => (tree_sitter_cpp::LANGUAGE.into(), "cpp", joined(&[tree_sitter_c::HIGHLIGHT_QUERY, tree_sitter_cpp::HIGHLIGHT_QUERY]), "", String::new()),
            Self::Css => (tree_sitter_css::LANGUAGE.into(), "css", tree_sitter_css::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Elixir => (tree_sitter_elixir::LANGUAGE.into(), "elixir", tree_sitter_elixir::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Go => (tree_sitter_go::LANGUAGE.into(), "go", tree_sitter_go::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Html => (tree_sitter_html::LANGUAGE.into(), "html", tree_sitter_html::HIGHLIGHTS_QUERY.into(), tree_sitter_html::INJECTIONS_QUERY, String::new()),
            Self::Java => (tree_sitter_java::LANGUAGE.into(), "java", tree_sitter_java::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::JavaScript => (js::LANGUAGE.into(), "javascript", joined(&[js::HIGHLIGHT_QUERY, js::JSX_HIGHLIGHT_QUERY]), js::INJECTIONS_QUERY, js::LOCALS_QUERY.into()),
            Self::Json => (tree_sitter_json::LANGUAGE.into(), "json", tree_sitter_json::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Kotlin => (tree_sitter_kotlin_sg::LANGUAGE.into(), "kotlin", tree_sitter_kotlin_sg::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Markdown => (tree_sitter_md::LANGUAGE.into(), "markdown", tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.into(), tree_sitter_md::INJECTION_QUERY_BLOCK, String::new()),
            Self::MarkdownInline => (tree_sitter_md::INLINE_LANGUAGE.into(), "markdown_inline", tree_sitter_md::HIGHLIGHT_QUERY_INLINE.into(), "", String::new()),
            Self::Python => (tree_sitter_python::LANGUAGE.into(), "python", tree_sitter_python::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Ruby => (tree_sitter_ruby::LANGUAGE.into(), "ruby", tree_sitter_ruby::HIGHLIGHTS_QUERY.into(), "", tree_sitter_ruby::LOCALS_QUERY.into()),
            Self::Rust => (tree_sitter_rust::LANGUAGE.into(), "rust", tree_sitter_rust::HIGHLIGHTS_QUERY.into(), tree_sitter_rust::INJECTIONS_QUERY, String::new()),
            Self::Sql => (tree_sitter_sequel::LANGUAGE.into(), "sql", tree_sitter_sequel::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Swift => (tree_sitter_swift::LANGUAGE.into(), "swift", tree_sitter_swift::HIGHLIGHTS_QUERY.into(), "", tree_sitter_swift::LOCALS_QUERY.into()),
            Self::Toml => (tree_sitter_toml_ng::LANGUAGE.into(), "toml", tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(), "", String::new()),
            Self::Tsx => (
                ts::LANGUAGE_TSX.into(),
                "tsx",
                joined(&[js::HIGHLIGHT_QUERY, js::JSX_HIGHLIGHT_QUERY, ts::HIGHLIGHTS_QUERY]),
                js::INJECTIONS_QUERY,
                joined(&[js::LOCALS_QUERY, ts::LOCALS_QUERY]),
            ),
            Self::TypeScript => (
                ts::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                joined(&[js::HIGHLIGHT_QUERY, ts::HIGHLIGHTS_QUERY]),
                js::INJECTIONS_QUERY,
                joined(&[js::LOCALS_QUERY, ts::LOCALS_QUERY]),
            ),
            Self::Yaml => (tree_sitter_yaml::LANGUAGE.into(), "yaml", tree_sitter_yaml::HIGHLIGHTS_QUERY.into(), "", String::new()),
        };
        let mut config = HighlightConfiguration::new(language, name, &highlights, injections, &locals).ok()?;
        config.configure(&NAMES.iter().map(|(name, _, _)| *name).collect::<Vec<_>>());
        Some(config)
    }
}

/// The token naming a path's grammar: its extension, or its whole name (Gemfile).
pub fn token_for_path(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(_, ext)| ext).to_string()
}

/// Lines as one piece each, uncoloured.
fn plain(lines: &[String]) -> Lines {
    lines.iter().map(|l| if l.is_empty() { vec![] } else { vec![(l.clone(), Style::new())] }).collect()
}

/// The grammars loaded so far, each the first time a text in its language is painted; dropped, they free their queries.
#[derive(Default)]
pub struct Grammars {
    loaded: HashMap<Lang, Option<HighlightConfiguration>>,
    highlighter: Option<Highlighter>,
}

/// Has tree-sitter allocate with mimalloc, whose freed pages go back to the system, rather than the system allocator,
/// which keeps a grammar's compiled queries resident after they are dropped; done once, before tree-sitter's first use.
fn use_mimalloc() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let allocator = tree_sitter::Allocator {
            malloc: libmimalloc_sys::mi_malloc,
            calloc: libmimalloc_sys::mi_calloc,
            realloc: libmimalloc_sys::mi_realloc,
            free: libmimalloc_sys::mi_free,
        };
        // SAFETY: one allocator family, set once before any tree-sitter object exists (every use goes through here).
        unsafe { tree_sitter::set_allocator(Some(allocator)) };
    });
}

impl Grammars {
    /// The lines coloured as one text in the language `token` names (a fence's info or a file's extension), so a
    /// string or comment spanning lines keeps its colour; plain when there is no grammar for it or it fails.
    pub fn paint(&mut self, token: &str, lines: &[String]) -> Lines {
        let Some(lang) = Lang::find(token) else { return plain(lines) };
        use_mimalloc();
        for lang in std::iter::once(lang).chain(lang.embeds().iter().copied()) {
            self.loaded.entry(lang).or_insert_with(|| lang.configuration());
        }
        let text = lines.join("\n");
        let highlighter = self.highlighter.get_or_insert_with(Highlighter::new);
        let loaded = &self.loaded;
        let Some(Some(config)) = loaded.get(&lang) else { return plain(lines) };
        let injected = |name: &str| Lang::find(name).and_then(|l| loaded.get(&l)).and_then(Option::as_ref);
        let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, injected) else { return plain(lines) };
        let mut out: Lines = vec![Vec::new()];
        let mut styles = vec![Style::new()];
        for event in events {
            match event {
                Ok(HighlightEvent::HighlightStart(h)) => styles.push(styles.last().copied().unwrap_or_default().patch(style(h.0))),
                Ok(HighlightEvent::HighlightEnd) => {
                    styles.pop();
                }
                Ok(HighlightEvent::Source { start, end }) => {
                    let Some(piece) = text.get(start..end) else { return plain(lines) };
                    let style = styles.last().copied().unwrap_or_default();
                    for (n, part) in piece.split('\n').enumerate() {
                        if n > 0 {
                            out.push(Vec::new());
                        }
                        if let (false, Some(line)) = (part.is_empty(), out.last_mut()) {
                            line.push((part.to_string(), style));
                        }
                    }
                }
                Err(_) => return plain(lines),
            }
        }
        if out.len() != lines.len().max(1) {
            return plain(lines);
        }
        out.truncate(lines.len());
        out
    }
}

/// A whole code block's lines, coloured for its fence's language: painted once per language and text, the grammar
/// loaded into `grammars` only when a block needs it.
pub fn code(grammars: &mut Grammars, lang: &str, text: &str) -> Lines {
    let mut hasher = DefaultHasher::new();
    (lang, text).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(lines) = PAINTED.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return lines.clone();
    }
    let lines: Vec<String> = text.trim_end_matches('\n').split('\n').map(|line| line.replace('\t', "    ")).collect();
    let lines = grammars.paint(lang, &lines);
    let mut painted = PAINTED.lock().unwrap_or_else(PoisonError::into_inner);
    if painted.len() >= PAINTED_KEPT {
        painted.clear();
    }
    painted.insert(key, lines.clone());
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const EVERY: [Lang; 22] = [
        Lang::Bash,
        Lang::C,
        Lang::Cpp,
        Lang::Css,
        Lang::Elixir,
        Lang::Go,
        Lang::Html,
        Lang::Java,
        Lang::JavaScript,
        Lang::Json,
        Lang::Kotlin,
        Lang::Markdown,
        Lang::MarkdownInline,
        Lang::Python,
        Lang::Ruby,
        Lang::Rust,
        Lang::Sql,
        Lang::Swift,
        Lang::Toml,
        Lang::Tsx,
        Lang::TypeScript,
        Lang::Yaml,
    ];

    fn rgb(hex: u32) -> Option<Color> {
        Some(Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8))
    }

    /// The colour of the first piece that is exactly `word` once trimmed.
    fn colour_of(lines: &Lines, word: &str) -> Option<Color> {
        lines.iter().flatten().find(|(t, _)| t.trim() == word).and_then(|(_, s)| s.fg)
    }

    fn paint(token: &str, text: &str) -> Lines {
        Grammars::default().paint(token, &text.split('\n').map(str::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn every_grammar_compiles_its_queries() {
        for lang in EVERY {
            assert!(lang.configuration().is_some(), "{lang:?}");
        }
    }

    #[test]
    fn typescript_in_vesper() {
        let lines = paint("ts", "// quota\nexport function quota(plan: Plan): number {\n  return { pro: 200 }[plan.name] ?? \"none\";\n}");
        assert_eq!(colour_of(&lines, "// quota"), rgb(0x6E6E74));
        assert_eq!(colour_of(&lines, "function"), rgb(0xA0A0A0));
        assert_eq!(colour_of(&lines, "quota"), rgb(0xFFC799));
        assert_eq!(colour_of(&lines, "Plan"), rgb(0xFFFFFF));
        assert_eq!(colour_of(&lines, "200"), rgb(0xFFC799));
        assert_eq!(colour_of(&lines, "\"none\""), rgb(0x99FFE4));
    }

    #[test]
    fn tsx_tags_and_elixir_atoms() {
        let tsx = paint("tsx", "const b = <span className=\"x\">{n}</span>;");
        assert_eq!(colour_of(&tsx, "span"), rgb(0xFFC799));
        let ex = paint("elixir", "defmodule Quota do\n  def limit(:free), do: 10\nend");
        assert_eq!(colour_of(&ex, "Quota"), rgb(0xFFFFFF));
        assert_eq!(colour_of(&ex, ":free"), rgb(0xFFC799));
        assert_eq!(colour_of(&ex, "defmodule"), rgb(0xA0A0A0));
    }

    #[test]
    fn a_comment_spanning_lines_keeps_its_colour() {
        let lines = paint("rust", "/* one\ntwo */ let x = 1;");
        assert_eq!(colour_of(&lines, "two */"), rgb(0x6E6E74));
    }

    #[test]
    fn unknown_languages_stay_plain() {
        assert_eq!(paint("brainfuck", "+++\n\n."), vec![vec![("+++".to_string(), Style::new())], vec![], vec![(".".to_string(), Style::new())]]);
    }

    proptest! {
        /// Whatever the code and language, painting keeps every line's text.
        #[test]
        fn painting_keeps_the_text(text in "\\PC{0,120}|[ -~\n]{0,200}", lang in prop::sample::select(vec!["ts", "tsx", "rust", "elixir", "md", "html", "sql", "swift", "kotlin", "cpp", "yaml", "none"])) {
            thread_local!(static GRAMMARS: std::cell::RefCell<Grammars> = Default::default());
            let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
            let painted = GRAMMARS.with_borrow_mut(|g| g.paint(lang, &lines));
            prop_assert_eq!(painted.len(), lines.len());
            for (line, pieces) in lines.iter().zip(&painted) {
                prop_assert_eq!(pieces.iter().map(|p| p.0.as_str()).collect::<String>(), line.clone());
            }
        }
    }
}
