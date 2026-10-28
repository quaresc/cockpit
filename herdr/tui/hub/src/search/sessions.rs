//! Past Claude sessions, searchable by their words and by what they meant: a tantivy index of the transcripts in Claude's
//! projects folder, one document per passage with its vector (see embed), kept in ~/.cache/cockpit/sessions-index. A
//! process of its own (the hub's command run again with HUB_TASK set) indexes the transcripts whose size or time
//! changed, when the hub starts, on a watch of the projects folder and every few minutes, so the memory indexing takes
//! goes back to the system each time; the hub only reads.

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use chrono::{DateTime, Local, NaiveDate};
use cockpit_core::files;
use notify::{RecursiveMode, Watcher};
use serde_json::{Value, json};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value as _};
use tantivy::snippet::SnippetGenerator;
use tantivy::{DocAddress, IndexReader, ReloadPolicy, Searcher, TantivyDocument, Term, doc};

use super::embed;
use super::transcript::{self, Session};

/// Set on the hub's command run again to index the sessions once, then exit.
pub const HUB_TASK: &str = "COCKPIT_HUB_INDEX_SESSIONS";
/// Sessions shown after cockpit's own entries.
const BEST: usize = 6;
/// The writer's memory, tantivy's least for one thread: a batch is committed and the writer dropped well before.
const WRITER_HEAP: usize = 15_000_000;
/// Transcripts per commit, so a first index answers searches long before it ends.
const BATCH: usize = 20;
/// A full scan for what the watch missed.
const SCAN: Duration = Duration::from_secs(300);
/// A busy session writes its transcript all the time: changes wait for this much quiet, and an update this long after
/// the last one at least.
const QUIET: Duration = Duration::from_secs(5);
const SPACING: Duration = Duration::from_secs(30);
/// Bumped when the schema or what a document holds changes: the old index is dropped and built again.
const VERSION: &str = "2";
const TITLE_BOOST: f32 = 3.0;
/// A passage: about this many characters of a session, cut at a line.
const PASSAGE: usize = 1500;
/// Passages each way of searching brings to the ranking, and how it weighs a rank (reciprocal rank fusion).
const CANDIDATES: usize = 40;
const FUSION: f32 = 60.0;
const MEANING_WEIGHT: f32 = 0.8;
/// A passage reads as close in meaning from this similarity on.
const CLOSE: f32 = 0.3;
/// The words a result shows of the passage that matched.
const SNIPPET: usize = 110;

static INDEX: OnceLock<Sessions> = OnceLock::new();

/// The past sessions matching the query, as palette entries; none until the index exists, what it has while it builds.
pub fn search(query: &str) -> Vec<Value> {
    INDEX.get().map(|index| index.search(query, Local::now().date_naive())).unwrap_or_default()
}

/// Claude's projects folder, under CLAUDE_CONFIG_DIR when set.
pub fn projects_dir() -> PathBuf {
    std::env::var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|_| files::home().join(".claude")).join("projects")
}

fn index_dir() -> PathBuf {
    files::cache().join("sessions-index")
}

/// The indexing process's work: brings the index up to date with the transcripts.
pub fn index_once() {
    embed::fetch();
    if let Ok(index) = Sessions::open(&index_dir()) {
        let _ = index.update(&projects_dir());
    }
}

/// Keeps the index current for as long as the hub runs, reading it as soon as it exists.
pub fn keep() {
    let projects = projects_dir();
    let (tx, changes) = channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = tx.send(());
        }
    })
    .ok();
    let mut watching = false;
    loop {
        // Watching before indexing, so a change made while it runs still brings the next run.
        if !watching && let Some(w) = watcher.as_mut() {
            watching = w.watch(&projects, RecursiveMode::Recursive).is_ok();
        }
        let started = Instant::now();
        run_indexer();
        match changes.recv_timeout(SCAN) {
            Ok(()) => {
                let first = Instant::now();
                while first.elapsed() < SPACING && changes.recv_timeout(QUIET).is_ok() {}
                std::thread::sleep(SPACING.saturating_sub(started.elapsed()));
                while changes.try_recv().is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(SCAN),
        }
    }
}

/// Runs the hub's own command line again as the indexing process and waits for it, opening the index for searches as
/// soon as its first batch lands; the reader follows each commit by itself.
fn run_indexer() {
    let Ok(exe) = std::env::current_exe() else { return };
    let child = Command::new(exe).args(std::env::args_os().skip(1)).env(HUB_TASK, "1").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return };
    loop {
        if INDEX.get().is_none()
            && let Some(index) = Sessions::open_existing(&index_dir())
        {
            let _ = INDEX.set(index);
        }
        if !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    if INDEX.get().is_none()
        && let Some(index) = Sessions::open_existing(&index_dir())
    {
        let _ = INDEX.set(index);
    }
}

struct Fields {
    path: Field,
    id: Field,
    cwd: Field,
    title: Field,
    text: Field,
    first: Field,
    last: Field,
    vector: Field,
}

/// The reader's generation they were read at, and each passage's vector.
type Vectors = (u64, Vec<(DocAddress, Vec<u8>)>);

pub struct Sessions {
    index: tantivy::Index,
    reader: IndexReader,
    fields: Fields,
    manifest: PathBuf,
    /// The passages' vectors, read once per commit the reader sees.
    vectors: Mutex<Vectors>,
}

/// Each transcript indexed, by path: its size and modification time in nanoseconds when it was read.
type Manifest = BTreeMap<String, (u64, u128)>;

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let words = TextOptions::default().set_indexing_options(TextFieldIndexing::default().set_tokenizer("en_stem").set_index_option(IndexRecordOption::WithFreqsAndPositions));
    let fields = Fields {
        path: builder.add_text_field("path", STRING | STORED),
        id: builder.add_text_field("id", STORED),
        cwd: builder.add_text_field("cwd", STORED),
        title: builder.add_text_field("title", words.clone() | STORED),
        text: builder.add_text_field("text", words | STORED),
        first: builder.add_text_field("first", STORED),
        last: builder.add_text_field("last", STORED),
        vector: builder.add_bytes_field("vector", STORED),
    };
    (builder.build(), fields)
}

impl Sessions {
    /// The index in this folder, made anew when missing, from another version or unreadable.
    pub fn open(dir: &Path) -> tantivy::Result<Self> {
        let current = dir.join(format!("v{VERSION}"));
        if let Ok(entries) = std::fs::read_dir(dir) {
            for old in entries.flatten().filter(|e| e.path() != current) {
                let _ = std::fs::remove_dir_all(old.path());
            }
        }
        let (schema, fields) = schema();
        let opened = std::fs::create_dir_all(&current).map_err(|e| tantivy::TantivyError::SystemError(e.to_string())).and_then(|_| {
            let directory = MmapDirectory::open(&current)?;
            tantivy::Index::open_or_create(directory, schema.clone())
        });
        let index = match opened {
            Ok(index) => index,
            Err(_) => {
                let _ = std::fs::remove_dir_all(&current);
                std::fs::create_dir_all(&current).map_err(|e| tantivy::TantivyError::SystemError(e.to_string()))?;
                tantivy::Index::create_in_dir(&current, schema)?
            }
        };
        let reader = index.reader_builder().reload_policy(ReloadPolicy::Manual).try_into()?;
        Ok(Self { index, reader, fields, manifest: current.join("manifest.json"), vectors: Mutex::default() })
    }

    /// The index another process keeps, for reading: None until it holds a commit of this version.
    fn open_existing(dir: &Path) -> Option<Self> {
        let current = dir.join(format!("v{VERSION}"));
        if !current.join("meta.json").exists() {
            return None;
        }
        let (schema, fields) = schema();
        let index = tantivy::Index::open_in_dir(&current).ok().filter(|i| i.schema() == schema)?;
        let reader = index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into().ok()?;
        Some(Self { index, reader, fields, manifest: current.join("manifest.json"), vectors: Mutex::default() })
    }

    fn read_manifest(&self) -> Manifest {
        std::fs::read_to_string(&self.manifest).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    /// Indexes the transcripts added or changed since the last update and drops the removed ones, a batch per commit;
    /// how many it indexed or dropped.
    pub fn update(&self, projects: &Path) -> tantivy::Result<usize> {
        let mut manifest = self.read_manifest();
        let found = transcripts(projects);
        let gone: Vec<String> = manifest.keys().filter(|p| !found.contains_key(*p)).cloned().collect();
        let changed: Vec<(&String, &(u64, u128))> = found.iter().filter(|(path, stamp)| manifest.get(*path) != Some(stamp)).collect();
        let work: Vec<(&String, Option<&(u64, u128)>)> = gone.iter().map(|p| (p, None)).chain(changed.iter().map(|(p, s)| (*p, Some(*s)))).collect();
        for batch in work.chunks(BATCH) {
            let mut writer = self.index.writer_with_num_threads::<TantivyDocument>(1, WRITER_HEAP)?;
            for &(path, stamp) in batch {
                writer.delete_term(Term::from_field_text(self.fields.path, path));
                match stamp {
                    Some(stamp) => {
                        if let Some(session) = transcript::read(Path::new(path)) {
                            for passage in self.documents(path, &session) {
                                writer.add_document(passage)?;
                            }
                        }
                        manifest.insert(path.clone(), *stamp);
                    }
                    None => {
                        manifest.remove(path);
                    }
                }
            }
            writer.commit()?;
            writer.wait_merging_threads()?;
            let _ = files::write(&self.manifest, &json!(manifest));
            self.reader.reload()?;
        }
        Ok(work.len())
    }

    /// A document per passage of the session, each with the session's fields and, with the model there, its vector.
    fn documents(&self, path: &str, session: &Session) -> Vec<TantivyDocument> {
        let f = &self.fields;
        let model = embed::model();
        passages(&session.text)
            .into_iter()
            .map(|passage| {
                let vector = model.and_then(|m| m.embed(&format!("{}\n{passage}", session.title))).map(|v| embed::pack(&v)).unwrap_or_default();
                doc!(f.path => path, f.id => session.id.as_str(), f.cwd => session.cwd.as_str(), f.title => session.title.as_str(),
                    f.text => passage, f.first => session.first.as_str(), f.last => session.last.as_str(), f.vector => vector)
            })
            .collect()
    }

    /// The best sessions for the query: passages ranked by their words (BM25 over the title, boosted, and the text;
    /// quoted phrases as phrases) and by their meaning, the two ranks fused; each session once, with the passage that
    /// matched it.
    pub fn search(&self, query: &str, today: NaiveDate) -> Vec<Value> {
        if query.trim().is_empty() {
            return Vec::new();
        }
        let searcher = self.reader.searcher();
        let mut parser = QueryParser::for_index(&self.index, vec![self.fields.title, self.fields.text]);
        parser.set_field_boost(self.fields.title, TITLE_BOOST);
        let (parsed, _) = parser.parse_query_lenient(query);
        let by_words: Vec<DocAddress> =
            searcher.search(&parsed, &TopDocs::with_limit(CANDIDATES).order_by_score()).unwrap_or_default().into_iter().map(|(_, a)| a).collect();
        // A quoted phrase asks for those words: meaning stays out of it.
        let meaning = if query.contains('"') { None } else { embed::model().and_then(|m| m.embed(&embed::question(query))) };
        let by_meaning = meaning.as_ref().map(|q| self.closest(&searcher, q)).unwrap_or_default();
        let mut fused: HashMap<DocAddress, f32> = HashMap::new();
        // The words weigh a little more: a passage saying what was typed beats one only near it in meaning.
        for (ranked, weight) in [(&by_words, 1.0), (&by_meaning, MEANING_WEIGHT)] {
            for (rank, address) in ranked.iter().enumerate() {
                *fused.entry(*address).or_default() += weight / (FUSION + rank as f32);
            }
        }
        let mut ranked: Vec<(DocAddress, f32)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let snippets = SnippetGenerator::create(&searcher, &*parsed, self.fields.text).ok().map(|mut g| {
            g.set_max_num_chars(SNIPPET);
            g
        });
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for (address, _) in ranked {
            let Ok(found) = searcher.doc::<TantivyDocument>(address) else { continue };
            let id = found.get_first(self.fields.id).and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if !seen.insert(id) {
                continue;
            }
            let worded = by_words.contains(&address);
            out.push(self.entry(&found, today, &self.snippet(&found, worded.then_some(snippets.as_ref()).flatten(), meaning.as_deref())));
            if out.len() == BEST {
                break;
            }
        }
        out
    }

    /// The passages closest in meaning to the query, closest first, from CLOSE on.
    fn closest(&self, searcher: &Searcher, query: &[f32]) -> Vec<DocAddress> {
        let Ok(mut cache) = self.vectors.lock() else { return Vec::new() };
        let generation = searcher.generation().generation_id();
        if cache.0 != generation || cache.1.is_empty() {
            let mut all = Vec::new();
            for (segment, reader) in searcher.segment_readers().iter().enumerate() {
                for doc in reader.doc_ids_alive() {
                    let address = DocAddress::new(segment as u32, doc);
                    let stored = searcher.doc::<TantivyDocument>(address).ok();
                    let bytes = stored.as_ref().and_then(|d| d.get_first(self.fields.vector)).and_then(|v| v.as_bytes()).map(<[u8]>::to_vec);
                    if let Some(bytes) = bytes.filter(|b| !b.is_empty()) {
                        all.push((address, bytes));
                    }
                }
            }
            *cache = (generation, all);
        }
        let mut scored: Vec<(f32, DocAddress)> = cache.1.iter().map(|(a, v)| (embed::similarity(v, query), *a)).filter(|(s, _)| *s >= CLOSE).collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored.into_iter().take(CANDIDATES).map(|(_, a)| a).collect()
    }

    /// Why the passage matched: the words around the query's, else its line closest in meaning, cut to a line.
    fn snippet(&self, found: &TantivyDocument, words: Option<&SnippetGenerator>, meaning: Option<&[f32]>) -> String {
        let text = found.get_first(self.fields.text).and_then(|v| v.as_str()).unwrap_or_default();
        // A fragment starting inside the text starts inside a word too: that word goes.
        let by_words = words.map(|g| g.snippet_from_doc(found)).filter(|s| !s.highlighted().is_empty()).map(|s| {
            let fragment = s.fragment();
            let cut = s.fragment_range().start > 0 && !text[..s.fragment_range().start].ends_with(char::is_whitespace);
            if cut { fragment.split_once(char::is_whitespace).map_or(fragment, |(_, rest)| rest).to_string() } else { fragment.to_string() }
        });
        let by_meaning = || {
            let (model, query) = (embed::model()?, meaning?);
            text.lines()
                .filter(|l| l.split_whitespace().count() >= 4)
                .filter_map(|l| Some((model.embed(l).map(|v| v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>())?, l)))
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, l)| l.to_string())
        };
        let line = by_words.or_else(by_meaning).unwrap_or_else(|| text.lines().next().unwrap_or_default().to_string());
        let words = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if words.chars().count() <= SNIPPET { words } else { format!("{}…", words.chars().take(SNIPPET).collect::<String>().trim_end()) }
    }

    fn entry(&self, found: &TantivyDocument, today: NaiveDate, snippet: &str) -> Value {
        let get = |field: Field| found.get_first(field).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        let (id, cwd, title, last) = (get(self.fields.id), get(self.fields.cwd), get(self.fields.title), get(self.fields.last));
        let project = Path::new(&cwd).file_name().map(|n| n.to_string_lossy().into_owned());
        let when = DateTime::parse_from_rfc3339(&last).ok().map(|t| t.with_timezone(&Local).date_naive());
        let quoted = (!snippet.is_empty()).then(|| format!("“{snippet}”"));
        let help: Vec<String> =
            project.into_iter().chain(when.map(|d| d.format("%a %d %b").to_string())).chain(when.map(|d| age(d, today))).chain(quoted).collect();
        json!({"app": "session", "kind": "session", "title": title, "help": help.join(" · "), "target": {"app": "session", "key": id, "cwd": cwd, "title": title}})
    }
}

/// The session's text cut into passages of about PASSAGE characters, at lines.
fn passages(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in text.lines() {
        let current = out.last_mut().expect("one passage at least");
        if !current.is_empty() && current.len() + line.len() > PASSAGE {
            out.push(String::new());
        }
        let current = out.last_mut().expect("one passage at least");
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    out.retain(|p| !p.trim().is_empty());
    out
}

/// The session transcripts, `<projects>/<project>/<session>.jsonl`, with their stamps; subagents' are left out.
fn transcripts(projects: &Path) -> BTreeMap<String, (u64, u128)> {
    let mut found = BTreeMap::new();
    for project in std::fs::read_dir(projects).into_iter().flatten().flatten() {
        for file in std::fs::read_dir(project.path()).into_iter().flatten().flatten() {
            let path = file.path();
            let is_session = path.extension().is_some_and(|e| e == "jsonl") && !file.file_name().to_string_lossy().starts_with("agent-");
            let Ok(meta) = file.metadata() else { continue };
            if is_session && meta.is_file() {
                let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or_default();
                found.insert(path.to_string_lossy().into_owned(), (meta.len(), modified));
            }
        }
    }
    found
}

/// How long ago a day was, in the unit that reads best.
pub fn age(day: NaiveDate, today: NaiveDate) -> String {
    match (today - day).num_days() {
        ..=0 => "today".into(),
        1 => "yesterday".into(),
        n @ 2..14 => format!("{n} days ago"),
        n @ 14..60 => format!("{} weeks ago", n / 7),
        n @ 60..730 => format!("{} months ago", n / 30),
        n => format!("{} years ago", n / 365),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 28).unwrap()
    }

    /// The fixture box's projects folder, copied, with an index beside it.
    fn indexed() -> (tempfile::TempDir, PathBuf, Sessions) {
        let dir = tempfile::tempdir().unwrap();
        let projects = dir.path().join("projects");
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/box/home/.claude/projects");
        copy(&fixtures, &projects);
        let index = Sessions::open(&dir.path().join("index")).unwrap();
        index.update(&projects).unwrap();
        (dir, projects, index)
    }

    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap().flatten() {
            if entry.path().is_dir() {
                copy(&entry.path(), &to.join(entry.file_name()));
            } else {
                std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
            }
        }
    }

    fn keys(found: &[Value]) -> Vec<&str> {
        found.iter().map(|e| e["target"]["key"].as_str().unwrap()).collect()
    }

    #[test]
    fn entries_carry_the_title_folder_and_age() {
        let (_dir, _, index) = indexed();
        let found = index.search("callback", today());
        assert_eq!(keys(&found), ["s-login"], "subagents, tool output and sidechains are not indexed");
        assert_eq!(found[0]["title"], "Google sign-in for ENG-401");
        assert!(found[0]["help"].as_str().unwrap().starts_with("api · Tue 27 Oct · yesterday · “"), "{}", found[0]["help"]);
        assert!(found[0]["help"].as_str().unwrap().contains("callback"), "the passage that matched shows");
        assert_eq!(found[0]["target"], json!({"app": "session", "key": "s-login", "cwd": "@BOX@/repos/api", "title": "Google sign-in for ENG-401"}));
        assert!(index.search("dark mode", today())[0]["help"].as_str().unwrap().starts_with("web · Wed 02 Sep · 8 weeks ago · “"));
    }

    #[test]
    fn the_title_weighs_more_than_the_text() {
        let (_dir, _, index) = indexed();
        // Both sessions say sign-in; s-dark only in its text, s-login in its title too.
        assert_eq!(keys(&index.search("sign-in", today())), ["s-login", "s-dark"]);
        assert_eq!(keys(&index.search("tokens", today())), ["s-dark"]);
        assert!(index.search("zebrafish", today()).is_empty());
    }

    #[test]
    fn phrases_match_as_phrases() {
        let (_dir, _, index) = indexed();
        assert_eq!(keys(&index.search("\"compares the state\"", today())), ["s-login"]);
        assert!(index.search("\"state compares the\"", today()).is_empty());
        assert_eq!(keys(&index.search("comparing states", today())), ["s-login"], "words are stemmed");
    }

    #[test]
    fn only_changed_transcripts_are_indexed_again() {
        let (_dir, projects, index) = indexed();
        assert_eq!(index.update(&projects).unwrap(), 0);
        let dark = projects.join("-box-repos-web/s-dark.jsonl");
        let mut text = std::fs::read_to_string(&dark).unwrap();
        text += r#"{"type":"user","timestamp":"2026-10-28T08:00:00Z","message":{"role":"user","content":"Also the walrus banner"}}"#;
        std::fs::write(&dark, text).unwrap();
        assert_eq!(index.update(&projects).unwrap(), 1);
        let found = index.search("walrus", today());
        assert_eq!(keys(&found), ["s-dark"]);
        assert!(found[0]["help"].as_str().unwrap().starts_with("web · Wed 28 Oct · today · “"));
        assert!(found[0]["help"].as_str().unwrap().contains("walrus"));
        assert_eq!(index.search("dark mode", today()).len(), 1, "the old version is gone");
    }

    #[test]
    fn a_removed_transcript_leaves_the_index() {
        let (_dir, projects, index) = indexed();
        std::fs::remove_file(projects.join("-box-repos-api/s-login.jsonl")).unwrap();
        assert_eq!(index.update(&projects).unwrap(), 1);
        assert!(index.search("callback", today()).is_empty());
    }

    #[test]
    fn the_index_survives_a_restart_and_a_new_version_starts_over() {
        let (dir, projects, index) = indexed();
        drop(index);
        let index = Sessions::open(&dir.path().join("index")).unwrap();
        assert_eq!(index.update(&projects).unwrap(), 0);
        assert_eq!(keys(&index.search("callback", today())), ["s-login"]);
        drop(index);
        std::fs::create_dir_all(dir.path().join("index/v0")).unwrap();
        Sessions::open(&dir.path().join("index")).unwrap();
        assert!(!dir.path().join("index/v0").exists());
    }

    #[test]
    fn odd_queries_find_nothing_rather_than_fail() {
        let (_dir, _, index) = indexed();
        for query in ["", "   ", "\"", "title:", "((", "AND", "-"] {
            let _ = index.search(query, today());
        }
    }

    #[test]
    fn ages() {
        let day = |d: u32, m: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let ages: Vec<String> = [day(28, 10), day(27, 10), day(20, 10), day(1, 10), day(1, 6)].into_iter().map(|d| age(d, today())).collect();
        assert_eq!(ages, ["today", "yesterday", "8 days ago", "3 weeks ago", "4 months ago"]);
        assert_eq!(age(NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(), today()), "3 years ago");
    }
}
