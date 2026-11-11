//! Lexical retrieval: a code-aware tokenizer and an in-memory BM25 index.
//!
//! Coding memories are dominated by exact strings: error messages, identifiers,
//! versions, and paths. The tokenizer keeps each compound token whole
//! (`openssl-sys`, `ssl_ctx_new`, `0.9.103`) and also emits its parts
//! (`openssl`, `sys`, `ssl`, `ctx`) and camelCase pieces. Exact and partial
//! queries both match, and exact matches score higher because they hit more
//! tokens.
//!
//! Distinct token types are capped ([`LexicalIndex::max_terms`]). Once the
//! dictionary is full, rare new tokens are dropped so vocabulary growth from
//! code-like text cannot unbounded-grow RSS.

use std::collections::HashMap;

/// BM25 term-frequency saturation.
pub const K1: f32 = 1.2;
/// BM25 document-length normalization.
pub const B: f32 = 0.75;
/// Default dictionary cap: high enough for small and medium stores; the 100k
/// synthetic bench peaked around 207k terms.
pub const DEFAULT_MAX_TERMS: usize = 500_000;
const MAX_TOKEN_CHARS: usize = 64;
/// Characters that join words into one identifier, path, or version.
const GLUE: &[char] = &['_', '-', '.', '/', ':', '@', '+', '#', '$', '\\'];
const STOPWORDS: &[&str] = &[
    "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "have", "in", "is", "it",
    "its", "of", "on", "or", "that", "the", "this", "to", "was", "were", "will", "with",
];

pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for raw in text.split(|c: char| !(c.is_alphanumeric() || GLUE.contains(&c))) {
        let chunk = raw.trim_matches(|c: char| !c.is_alphanumeric());
        if chunk.is_empty() {
            continue;
        }
        push(&mut tokens, chunk);
        let parts: Vec<&str> = chunk
            .split(|c: char| !c.is_alphanumeric())
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() > 1 {
            for part in &parts {
                push(&mut tokens, part);
            }
        }
        for part in &parts {
            let pieces = split_camel(part);
            if pieces.len() > 1 {
                for piece in pieces {
                    push(&mut tokens, piece);
                }
            }
        }
    }
    tokens
}

fn push(tokens: &mut Vec<String>, token: &str) {
    let lower = token.to_lowercase();
    if lower.chars().count() < 2 || STOPWORDS.contains(&lower.as_str()) {
        return;
    }
    tokens.push(lower.chars().take(MAX_TOKEN_CHARS).collect());
}

/// Splits `fooBar` into `foo`, `Bar` and `SSLCtx` into `SSL`, `Ctx`. Digits stay
/// attached (`arm64`, `utf8`).
fn split_camel(word: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = word.char_indices().collect();
    let mut cuts = vec![0];
    for i in 1..chars.len() {
        let prev = chars[i - 1].1;
        let cur = chars[i].1;
        let next_is_lower = chars.get(i + 1).is_some_and(|(_, c)| c.is_lowercase());
        let lower_to_upper = (prev.is_lowercase() || prev.is_ascii_digit()) && cur.is_uppercase();
        let acronym_end = prev.is_uppercase() && cur.is_uppercase() && next_is_lower;
        if lower_to_upper || acronym_end {
            cuts.push(chars[i].0);
        }
    }
    cuts.push(word.len());
    cuts.windows(2).map(|w| &word[w[0]..w[1]]).collect()
}

/// Postings per term, document lengths, and corpus statistics. Documents are
/// dense, increasing IDs added once and never edited, because claims are
/// immutable. Excluding inactive documents is the caller's job.
#[derive(Debug)]
pub struct LexicalIndex {
    dict: HashMap<String, u32>,
    /// Term ID to `(doc, term frequency)` pairs, in increasing doc order.
    postings: Vec<Vec<(u32, u32)>>,
    doc_len: Vec<u32>,
    total_len: u64,
    max_terms: usize,
    /// Times a new token was skipped because the dictionary was at capacity.
    dropped_new_terms: u64,
}

impl Default for LexicalIndex {
    fn default() -> Self {
        Self::with_max_terms(DEFAULT_MAX_TERMS)
    }
}

impl LexicalIndex {
    pub fn with_max_terms(max_terms: usize) -> Self {
        Self {
            dict: HashMap::new(),
            postings: Vec::new(),
            doc_len: Vec::new(),
            total_len: 0,
            max_terms: max_terms.max(1),
            dropped_new_terms: 0,
        }
    }

    pub fn max_terms(&self) -> usize {
        self.max_terms
    }

    pub fn dropped_new_terms(&self) -> u64 {
        self.dropped_new_terms
    }

    pub fn add(&mut self, doc: u32, text: &str) {
        debug_assert_eq!(
            doc as usize,
            self.doc_len.len(),
            "documents are dense and in order"
        );
        let tokens = tokenize(text);
        let mut frequencies: HashMap<&str, u32> = HashMap::new();
        for token in &tokens {
            *frequencies.entry(token).or_default() += 1;
        }
        for (term, tf) in frequencies {
            let id = match self.dict.get(term) {
                Some(&id) => id,
                None if self.dict.len() >= self.max_terms => {
                    self.dropped_new_terms += 1;
                    continue;
                }
                None => {
                    let id = self.postings.len() as u32;
                    self.dict.insert(term.to_owned(), id);
                    self.postings.push(Vec::new());
                    id
                }
            };
            self.postings[id as usize].push((doc, tf));
        }
        let len = tokens.len() as u32;
        self.doc_len.push(len);
        self.total_len += u64::from(len);
    }

    pub fn docs(&self) -> usize {
        self.doc_len.len()
    }

    pub fn terms(&self) -> usize {
        self.dict.len()
    }

    /// BM25 score of every document matching at least one query term, in
    /// increasing document order. Scores accumulate in a dense array, which is
    /// much faster than a hash map when common terms touch many documents.
    pub fn search(&self, query: &str) -> Vec<(u32, f32)> {
        let n = self.doc_len.len();
        if n == 0 {
            return Vec::new();
        }
        let avg_len = (self.total_len as f32 / n as f32).max(1.0);
        let mut terms = tokenize(query);
        terms.sort_unstable();
        terms.dedup();
        let postings: Vec<&Vec<(u32, u32)>> = terms
            .iter()
            .filter_map(|term| self.dict.get(term))
            .map(|id| &self.postings[*id as usize])
            .collect();
        if postings.is_empty() {
            return Vec::new();
        }
        let mut scores = vec![0f32; n];
        let mut touched = Vec::new();
        for list in postings {
            let df = list.len() as f32;
            let idf = (1.0 + (n as f32 - df + 0.5) / (df + 0.5)).ln();
            for &(doc, tf) in list {
                let tf = tf as f32;
                let len = self.doc_len[doc as usize] as f32;
                let score = idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * len / avg_len));
                let slot = &mut scores[doc as usize];
                if *slot == 0.0 {
                    touched.push(doc);
                }
                *slot += score;
            }
        }
        touched.sort_unstable();
        touched
            .into_iter()
            .map(|doc| (doc, scores[doc as usize]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_compounds_and_emits_parts() {
        let tokens = tokenize(r#"ld: symbol(s) not found for architecture arm64 (_SSL_CTX_new)"#);
        for expected in [
            "ld",
            "symbol",
            "not",
            "found",
            "architecture",
            "arm64",
            "ssl_ctx_new",
            "ssl",
            "ctx",
            "new",
        ] {
            assert!(
                tokens.contains(&expected.to_owned()),
                "missing {expected} in {tokens:?}"
            );
        }
        assert!(!tokens.contains(&"for".to_owned()));
        assert!(!tokens.contains(&"s".to_owned()));
    }

    #[test]
    fn splits_paths_versions_and_camel_case() {
        let tokens = tokenize("openssl-sys 0.9.103 src/main.rs parseHTTPResponse std::io::Error");
        for expected in [
            "openssl-sys",
            "openssl",
            "sys",
            "0.9.103",
            "103",
            "src/main.rs",
            "main",
            "rs",
            "parsehttpresponse",
            "parse",
            "http",
            "response",
            "std::io::error",
            "io",
            "error",
        ] {
            assert!(
                tokens.contains(&expected.to_owned()),
                "missing {expected} in {tokens:?}"
            );
        }
    }

    #[test]
    fn handles_unicode_and_empty_input() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("  --  ").is_empty());
        let tokens = tokenize("Fehler: Datei nicht gefunden über café");
        assert!(tokens.contains(&"über".to_owned()));
        assert!(tokens.contains(&"café".to_owned()));
    }

    #[test]
    fn bm25_prefers_exact_rare_terms() {
        let mut index = LexicalIndex::default();
        index.add(0, "cargo build fails on linux with a linker error");
        index.add(1, "ld: symbol not found _SSL_CTX_new on macOS arm64");
        index.add(2, "cargo test is flaky when TZ is not UTC");
        let mut hits = index.search("_SSL_CTX_new not found");
        hits.sort_by(|a, b| b.1.total_cmp(&a.1));
        assert_eq!(hits[0].0, 1);
        assert!(index.search("kubernetes").is_empty());
    }

    #[test]
    fn caps_distinct_token_types_and_drops_rare_new_terms() {
        let mut index = LexicalIndex::with_max_terms(3);
        index.add(0, "alpha beta gamma");
        assert_eq!(index.terms(), 3);
        assert_eq!(index.dropped_new_terms(), 0);
        index.add(1, "alpha delta epsilon");
        assert_eq!(index.terms(), 3, "new tokens past the cap are not admitted");
        assert!(index.dropped_new_terms() >= 2);
        let hits = index.search("alpha");
        assert_eq!(hits.len(), 2, "known terms still post for later docs");
        assert!(
            index.search("delta").is_empty(),
            "dropped new tokens are not searchable"
        );
        assert!(index.search("gamma").iter().any(|(doc, _)| *doc == 0));
    }
}
