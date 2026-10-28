//! Syntax colours for code blocks and diffs: syntect's grammars with a Vesper palette (orange functions and numbers,
//! teal strings, grey keywords, dim comments). The grammars live only while something paints: each language's
//! compiled patterns take megabytes, so they go with the last painter, and a code block's colours are kept instead.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::str::FromStr;
use std::sync::{Arc, LazyLock, Mutex, PoisonError, Weak};

use ratatui::style::{Color, Modifier, Style};
use syntect::highlighting::{self as sh, FontStyle, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

static THEME: LazyLock<Theme> = LazyLock::new(vesper);
static SYNTAXES: Mutex<Weak<SyntaxSet>> = Mutex::new(Weak::new());
/// Code blocks already painted, by language and text, so a new width or a redraw paints nothing again.
static PAINTED: LazyLock<Mutex<HashMap<u64, Lines>>> = LazyLock::new(Default::default);
const PAINTED_KEPT: usize = 64;

/// Lines of coloured pieces.
pub type Lines = Vec<Vec<(String, Style)>>;

fn rgb(hex: u32) -> sh::Color {
    sh::Color { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 255 }
}

fn vesper() -> Theme {
    let rule = |scope: &str, color: u32, font: Option<FontStyle>| ThemeItem {
        scope: ScopeSelectors::from_str(scope).unwrap_or_default(),
        style: StyleModifier { foreground: Some(rgb(color)), background: None, font_style: font },
    };
    Theme {
        settings: ThemeSettings { foreground: Some(rgb(0xD2D2D2)), ..ThemeSettings::default() },
        scopes: vec![
            rule("comment, punctuation.definition.comment", 0x6E6E74, Some(FontStyle::ITALIC)),
            rule("string, constant.character, punctuation.definition.string", 0x99FFE4, None),
            rule("constant.numeric, constant.language, support.constant", 0xFFC799, None),
            rule("keyword, storage, keyword.operator.word", 0xA0A0A0, None),
            rule("entity.name.function, support.function, meta.function-call entity.name", 0xFFC799, None),
            rule("entity.name.type, entity.name.class, support.type, support.class, storage.type.class", 0xFFFFFF, None),
            rule("entity.name.tag, entity.other.attribute-name", 0xFFC799, None),
            rule("variable.parameter, variable.language", 0xD2D2D2, Some(FontStyle::ITALIC)),
            rule("markup.heading", 0xFFC799, Some(FontStyle::BOLD)),
            rule("markup.inserted", 0x99FFE4, None),
            rule("markup.deleted", 0xFF8080, None),
            rule("invalid", 0xFF8080, None),
        ],
        ..Theme::default()
    }
}

/// The token a grammar is found by: TypeScript borrows JavaScript's and Elixir Ruby's, the default set having neither;
/// None for plain text.
fn token(token: &str) -> Option<String> {
    let token = token.trim().to_lowercase();
    let token = match token.as_str() {
        "" | "text" | "txt" | "plain" => return None,
        "ts" | "tsx" | "typescript" | "mts" | "cts" | "jsx" | "mjs" | "cjs" => "js",
        "ex" | "exs" | "elixir" | "heex" | "eex" => "rb",
        "sh" | "zsh" | "bash" | "shell" | "console" => "sh",
        "yml" => "yaml",
        "jsonc" | "json5" => "json",
        "md" | "markdown" => "md",
        other => other,
    };
    Some(token.to_string())
}

fn syntax<'a>(set: &'a SyntaxSet, token: &str) -> Option<&'a SyntaxReference> {
    set.find_syntax_by_token(token).or_else(|| set.find_syntax_by_extension(token))
}

/// The token naming a path's grammar: its extension, or its whole name (Makefile, Dockerfile).
pub fn token_for_path(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(_, ext)| ext).to_string()
}

fn style_of(style: sh::Style) -> Style {
    let mut out = Style::new().fg(Color::Rgb(style.foreground.r, style.foreground.g, style.foreground.b));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    out
}

/// The grammars, held for as long as this is: paint several texts with one load.
#[derive(Clone)]
pub struct Grammars(Arc<SyntaxSet>);

impl Grammars {
    pub fn load() -> Self {
        let mut held = SYNTAXES.lock().unwrap_or_else(PoisonError::into_inner);
        Self(held.upgrade().unwrap_or_else(|| {
            let set = Arc::new(SyntaxSet::load_defaults_newlines());
            *held = Arc::downgrade(&set);
            set
        }))
    }

    /// A painter for a language token; plain text when the grammars have none for it.
    pub fn painter(&self, lang: &str) -> Painter {
        let mut painter = Painter { set: Some(self.0.clone()), token: token(lang), state: None };
        painter.restart();
        painter
    }
}

/// Colours lines one after the other, keeping the grammar's state across them (a string or comment spanning lines).
pub struct Painter {
    set: Option<Arc<SyntaxSet>>,
    token: Option<String>,
    state: Option<(ParseState, HighlightState)>,
}

impl Painter {
    /// A painter that leaves text plain, holding no grammars.
    pub fn plain() -> Self {
        Self { set: None, token: None, state: None }
    }

    /// Back to the start of a text with the same grammar (a new hunk).
    pub fn restart(&mut self) {
        let highlighter = Highlighter::new(&THEME);
        let found = self.set.as_ref().zip(self.token.as_ref()).and_then(|(set, t)| syntax(set, t));
        self.state = found.map(|s| (ParseState::new(s), HighlightState::new(&highlighter, ScopeStack::new())));
    }

    /// One line (without its newline) as coloured pieces; plain text when there is no grammar or it fails.
    pub fn line(&mut self, text: &str) -> Vec<(String, Style)> {
        let plain = || vec![(text.to_string(), Style::new())];
        let (Some(set), Some((parse, highlight))) = (&self.set, self.state.as_mut()) else { return plain() };
        let with_newline = format!("{text}\n");
        let Ok(ops) = parse.parse_line(&with_newline, set) else { return plain() };
        let highlighter = Highlighter::new(&THEME);
        HighlightIterator::new(highlight, &ops, &with_newline, &highlighter)
            .map(|(style, piece)| (piece.trim_end_matches('\n').to_string(), style_of(style)))
            .filter(|(piece, _)| !piece.is_empty())
            .collect()
    }
}

/// A whole code block's lines, coloured for its fence's language: painted once per language and text, the grammars
/// loaded into `grammars` only when a block needs them.
pub fn code(grammars: &mut Option<Grammars>, lang: &str, text: &str) -> Lines {
    let mut hasher = DefaultHasher::new();
    (lang, text).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(lines) = PAINTED.lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return lines.clone();
    }
    let lines: Lines = match token(lang) {
        Some(_) => {
            let mut painter = grammars.get_or_insert_with(Grammars::load).painter(lang);
            text.trim_end_matches('\n').split('\n').map(|line| painter.line(&line.replace('\t', "    "))).collect()
        }
        None => text.trim_end_matches('\n').split('\n').map(|line| vec![(line.replace('\t', "    "), Style::new())]).collect(),
    };
    let mut painted = PAINTED.lock().unwrap_or_else(PoisonError::into_inner);
    if painted.len() >= PAINTED_KEPT {
        painted.clear();
    }
    painted.insert(key, lines.clone());
    lines
}
