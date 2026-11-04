//! Sentences as vectors that sit close when they mean the same, for finding past sessions by meaning rather than by
//! their words: model2vec's potion-base-8M, a static model (a vector per token, averaged), downloaded once into
//! ~/.cache/cockpit/models and kept in memory as bytes, a quarter of its size.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use cockpit_core::files;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

const MODEL: &str = "potion-base-8M";
/// The Hugging Face commit the files come from, and each file's sha256 at it.
const REVISION: &str = "bf8b056651a2c21b8d2565580b8569da283cab23";
const FILES: [(&str, &str); 3] = [
    ("config.json", "2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553"),
    ("tokenizer.json", "e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50"),
    ("model.safetensors", "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2"),
];
/// Tokens of a text the vector reads; a session's chunks are well under it.
const MAX_TOKENS: usize = 512;

pub struct Model {
    tokenizer: Tokenizer,
    /// One row per token: its values scaled into bytes, and the scale that brings them back.
    rows: Vec<i8>,
    scales: Vec<f32>,
    pub dim: usize,
    unknown: Option<u32>,
}

fn dir() -> PathBuf {
    files::cache().join("models").join(MODEL)
}

/// Whether the model is used at all: COCKPIT_EMBED=off leaves it out (the hub's tests).
fn is_on() -> bool {
    std::env::var("COCKPIT_EMBED").map_or(true, |v| v != "off")
}

/// The model's files, downloaded when missing or not the pinned ones; false when they could not be.
pub fn fetch() -> bool {
    if !is_on() {
        return false;
    }
    let dir = dir();
    let _ = std::fs::create_dir_all(&dir);
    FILES.iter().all(|(name, sha)| {
        let path = dir.join(name);
        is_pinned(&path, sha) || download(&format!("https://huggingface.co/minishlab/{MODEL}/resolve/{REVISION}/{name}"), &path, sha)
    })
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn is_pinned(path: &Path, sha: &str) -> bool {
    std::fs::read(path).is_ok_and(|bytes| sha256(&bytes) == sha)
}

/// Saves the file only when its sha256 is the pinned one.
fn download(url: &str, path: &Path, sha: &str) -> bool {
    let Ok(response) = ureq::get(url).call() else { return false };
    let Ok(bytes) = response.into_body().with_config().limit(200 * 1024 * 1024).read_to_vec() else { return false };
    if sha256(&bytes) != sha {
        return false;
    }
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, path).is_ok()
}

/// The model, read once its files are there and the pinned ones; None without them.
pub fn model() -> Option<&'static Model> {
    static MODEL: OnceLock<Option<Model>> = OnceLock::new();
    let load = || {
        let dir = dir();
        (is_on() && FILES.iter().all(|(name, sha)| is_pinned(&dir.join(name), sha))).then(|| Model::load(&dir)).flatten()
    };
    MODEL.get_or_init(load).as_ref()
}

impl Model {
    pub fn load(dir: &Path) -> Option<Model> {
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).ok()?;
        let bytes = std::fs::read(dir.join("model.safetensors")).ok()?;
        let tensors = safetensors::SafeTensors::deserialize(&bytes).ok()?;
        let embeddings = tensors.tensor("embeddings").ok()?;
        let [vocab, dim] = embeddings.shape() else { return None };
        if embeddings.dtype() != safetensors::Dtype::F32 {
            return None;
        }
        let values: Vec<f32> = embeddings.data().as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        let (mut rows, mut scales) = (Vec::with_capacity(vocab * dim), Vec::with_capacity(*vocab));
        for row in values.chunks_exact(*dim) {
            let scale = row.iter().fold(0f32, |m, v| m.max(v.abs())) / 127.0;
            scales.push(scale);
            rows.extend(row.iter().map(|v| if scale > 0.0 { (v / scale).round() as i8 } else { 0 }));
        }
        let unknown = tokenizer.token_to_id("[UNK]");
        Some(Model { tokenizer, rows, scales, dim: *dim, unknown })
    }

    /// The text's vector, of length one; None for a text with no known token.
    pub fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let encoding = self.tokenizer.encode(text, false).ok()?;
        let mut sum = vec![0f32; self.dim];
        let mut n = 0;
        for &id in encoding.get_ids().iter().filter(|id| Some(**id) != self.unknown).take(MAX_TOKENS) {
            let (start, scale) = (id as usize * self.dim, self.scales.get(id as usize).copied().unwrap_or_default());
            for (s, v) in sum.iter_mut().zip(&self.rows[start..start + self.dim]) {
                *s += *v as f32 * scale;
            }
            n += 1;
        }
        if n == 0 {
            return None;
        }
        let norm = sum.iter().map(|v| v * v).sum::<f32>().sqrt();
        (norm > 0.0).then(|| sum.iter().map(|v| v / norm).collect())
    }
}

/// What a question asks about, without the words that only say it is looking for a session: "the session where we fixed
/// quotas" asks about fixed quotas.
pub fn question(text: &str) -> String {
    const ASKING: [&str; 22] = [
        "the",
        "a",
        "an",
        "session",
        "sessions",
        "conversation",
        "chat",
        "where",
        "when",
        "which",
        "that",
        "we",
        "i",
        "you",
        "about",
        "did",
        "talked",
        "discussed",
        "one",
        "find",
        "resume",
        "with",
    ];
    let kept: Vec<&str> = text.split_whitespace().filter(|w| !ASKING.contains(&w.to_lowercase().trim_matches(|c: char| !c.is_alphanumeric()))).collect();
    if kept.is_empty() { text.to_string() } else { kept.join(" ") }
}

/// A vector as bytes to store: its values scaled into i8, the scale first as four bytes.
pub fn pack(vector: &[f32]) -> Vec<u8> {
    let scale = vector.iter().fold(0f32, |m, v| m.max(v.abs())) / 127.0;
    let mut out = scale.to_le_bytes().to_vec();
    out.extend(vector.iter().map(|v| if scale > 0.0 { (v / scale).round() as i8 as u8 } else { 0 }));
    out
}

/// How close a stored vector is to a query's, from -1 to 1.
pub fn similarity(stored: &[u8], query: &[f32]) -> f32 {
    if stored.len() != query.len() + 4 {
        return 0.0;
    }
    let scale = f32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    stored[4..].iter().zip(query).map(|(b, q)| *b as i8 as f32 * scale * q).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vector_packs_into_bytes_and_keeps_its_closeness() {
        let a = [0.6, 0.8, 0.0];
        let packed = pack(&a);
        assert_eq!(packed.len(), 7);
        assert!((similarity(&packed, &a) - 1.0).abs() < 0.01);
        assert!(similarity(&packed, &[0.0, 0.0, 1.0]).abs() < 0.01);
        assert_eq!(similarity(&packed[..5], &a), 0.0, "another dimension compares as nothing");
    }

    #[test]
    fn a_question_keeps_what_it_asks_about() {
        assert_eq!(question("the session where we fixed quotas"), "fixed quotas");
        assert_eq!(question("Resume the one about dark mode"), "dark mode");
        assert_eq!(question("the session"), "the session", "nothing left keeps it whole");
    }

    #[test]
    fn a_file_counts_only_with_its_pinned_hash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "abc").unwrap();
        assert!(is_pinned(&path, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        assert!(!is_pinned(&path, FILES[0].1));
        assert!(!is_pinned(&dir.path().join("missing.json"), FILES[0].1));
    }

    /// With the real model, when it was downloaded on this machine: meaning outranks shared words.
    #[test]
    fn sentences_that_mean_the_same_sit_closer() {
        let Some(model) = Model::load(&dir()) else { return };
        let query = model.embed(&question("the session where we fixed quotas")).unwrap();
        let close = pack(&model.embed("API daily quotas: the quota check now counts export previews").unwrap());
        let far = pack(&model.embed("the session drew the meetings of the week as a picture").unwrap());
        assert!(similarity(&close, &query) > similarity(&far, &query), "{} vs {}", similarity(&close, &query), similarity(&far, &query));
    }
}
