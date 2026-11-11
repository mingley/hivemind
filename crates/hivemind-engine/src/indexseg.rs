//! Memory-mapped immutable lexical index segments (BM25, same semantics as [`LexicalIndex`]).
//!
//! Segments are sealed read-only files with magic `HMLXSEG\x01`. Writers freeze an in-memory
//! [`crate::lexical::LexicalIndex`]; readers mmap the file and search without loading postings
//! into the heap.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use memmap2::{Mmap, MmapOptions};

use crate::error::{Error, Result};
use crate::lexical::{self, LexicalIndex, B, K1};

const MAGIC: &[u8; 8] = b"HMLXSEG\x01";
const HEADER_LEN: usize = 8 + 4 + 8 + 4;

/// Per-term byte range into the postings section (`doc`,`tf` pairs, 8 bytes each).
type PostingSpan = (usize, u32);

/// On-disk layout after the fixed header:
/// - dictionary: `n_terms` × (`u32` utf8_len, utf8 bytes)
/// - postings: `n_terms` × (`u32` n, then `n` × (`u32` doc, `u32` tf))
/// - `doc_len`: `n_docs` × `u32`
#[derive(Debug)]
pub struct LexicalSegment {
    _file: File,
    mmap: Mmap,
    n_docs: u32,
    total_len: u64,
    n_terms: u32,
    doc_len_off: usize,
    term_to_id: HashMap<String, u32>,
    posting_spans: Vec<PostingSpan>,
}

impl LexicalSegment {
    /// Seal `index` to `path` (creates or truncates the file).
    pub fn write_sealed(path: impl AsRef<Path>, index: &LexicalIndex) -> Result<()> {
        let terms = index.terms_ordered();
        let n_terms = terms.len() as u32;
        let n_docs = index.docs() as u32;
        let total_len = index.total_token_length();

        let mut dict_blob = Vec::new();
        for term in &terms {
            let bytes = term.as_bytes();
            dict_blob.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            dict_blob.extend_from_slice(bytes);
        }

        let mut postings_blob = Vec::new();
        for list in index.posting_lists() {
            postings_blob.extend_from_slice(&(list.len() as u32).to_le_bytes());
            for &(doc, tf) in list {
                postings_blob.extend_from_slice(&doc.to_le_bytes());
                postings_blob.extend_from_slice(&tf.to_le_bytes());
            }
        }

        let mut doc_len_blob = Vec::new();
        for &len in index.doc_lengths() {
            doc_len_blob.extend_from_slice(&len.to_le_bytes());
        }

        let mut file = File::create(path)?;
        file.write_all(MAGIC)?;
        file.write_all(&n_docs.to_le_bytes())?;
        file.write_all(&total_len.to_le_bytes())?;
        file.write_all(&n_terms.to_le_bytes())?;
        file.write_all(&dict_blob)?;
        file.write_all(&postings_blob)?;
        file.write_all(&doc_len_blob)?;
        file.sync_all()?;
        Ok(())
    }

    /// Open a sealed segment read-only via memory map.
    ///
    /// The file is copied into an anonymous read-only map so searches do not depend on later file
    /// mutations (and the workspace forbids `unsafe` file-backed `Mmap::map`).
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        let len = file.metadata()?.len() as usize;
        let mut writable = MmapOptions::new()
            .len(len)
            .map_anon()
            .map_err(Error::Io)?;
        file.read_exact(&mut writable).map_err(Error::Io)?;
        let mmap = writable.make_read_only().map_err(Error::Io)?;
        if mmap.len() < HEADER_LEN {
            return Err(Error::Format("lexical segment truncated".into()));
        }
        if mmap[..8] != *MAGIC {
            return Err(Error::Format("bad lexical segment magic".into()));
        }
        let n_docs = u32::from_le_bytes(mmap[8..12].try_into().expect("header"));
        let total_len = u64::from_le_bytes(mmap[12..20].try_into().expect("header"));
        let n_terms = u32::from_le_bytes(mmap[20..24].try_into().expect("header"));

        let mut pos = HEADER_LEN;
        let mut term_to_id = HashMap::with_capacity(n_terms as usize);
        for id in 0..n_terms {
            if pos + 4 > mmap.len() {
                return Err(Error::Format("dictionary overrun".into()));
            }
            let len = u32::from_le_bytes(mmap[pos..pos + 4].try_into().expect("u32")) as usize;
            pos += 4;
            if pos + len > mmap.len() {
                return Err(Error::Format("term bytes overrun".into()));
            }
            let s = std::str::from_utf8(&mmap[pos..pos + len])
                .map_err(|e| Error::Format(format!("invalid utf-8 term: {e}")))?;
            term_to_id.insert(s.to_owned(), id);
            pos += len;
        }

        let mut posting_spans = Vec::with_capacity(n_terms as usize);
        for _ in 0..n_terms {
            if pos + 4 > mmap.len() {
                return Err(Error::Format("postings header overrun".into()));
            }
            let n = u32::from_le_bytes(mmap[pos..pos + 4].try_into().expect("u32"));
            pos += 4;
            let data_off = pos;
            let need = n as usize * 8;
            if pos + need > mmap.len() {
                return Err(Error::Format("postings overrun".into()));
            }
            posting_spans.push((data_off, n));
            pos += need;
        }

        let doc_len_off = pos;
        let doc_need = n_docs as usize * 4;
        if pos + doc_need > mmap.len() {
            return Err(Error::Format("doc_len overrun".into()));
        }
        if pos + doc_need != mmap.len() {
            return Err(Error::Format("trailing bytes in lexical segment".into()));
        }

        Ok(Self {
            _file: file,
            mmap,
            n_docs,
            total_len,
            n_terms,
            doc_len_off,
            term_to_id,
            posting_spans,
        })
    }

    pub fn docs(&self) -> usize {
        self.n_docs as usize
    }

    pub fn terms(&self) -> usize {
        self.n_terms as usize
    }

    fn doc_len_at(&self, doc: u32) -> u32 {
        let off = self.doc_len_off + doc as usize * 4;
        u32::from_le_bytes(self.mmap[off..off + 4].try_into().expect("doc_len"))
    }

    fn iter_postings(&self, term_id: u32) -> impl Iterator<Item = (u32, u32)> + '_ {
        let (data_off, n) = self.posting_spans[term_id as usize];
        (0..n).map(move |i| {
            let off = data_off + i as usize * 8;
            let doc = u32::from_le_bytes(self.mmap[off..off + 4].try_into().expect("doc"));
            let tf = u32::from_le_bytes(self.mmap[off + 4..off + 8].try_into().expect("tf"));
            (doc, tf)
        })
    }

    /// BM25 over the mmap segment; matches [`LexicalIndex::search`].
    pub fn search(&self, query: &str) -> Vec<(u32, f32)> {
        let n = self.n_docs as usize;
        if n == 0 {
            return Vec::new();
        }
        let avg_len = (self.total_len as f32 / n as f32).max(1.0);
        let mut terms = lexical::tokenize(query);
        terms.sort_unstable();
        terms.dedup();
        let term_ids: Vec<u32> = terms
            .iter()
            .filter_map(|term| self.term_to_id.get(term).copied())
            .collect();
        if term_ids.is_empty() {
            return Vec::new();
        }
        let mut scores = vec![0f32; n];
        let mut touched = Vec::new();
        for id in term_ids {
            let list: Vec<(u32, u32)> = self.iter_postings(id).collect();
            let df = list.len() as f32;
            let idf = (1.0 + (n as f32 - df + 0.5) / (df + 0.5)).ln();
            for &(doc, tf) in &list {
                let tf = tf as f32;
                let len = self.doc_len_at(doc) as f32;
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
    use tempfile::NamedTempFile;

    fn sample_index() -> LexicalIndex {
        let mut index = LexicalIndex::default();
        index.add(0, "cargo build fails on linux with a linker error");
        index.add(1, "ld: symbol not found _SSL_CTX_new on macOS arm64");
        index.add(2, "cargo test is flaky when TZ is not UTC");
        index
    }

    #[test]
    fn mmap_search_matches_in_memory() {
        let index = sample_index();
        let queries = ["_SSL_CTX_new not found", "cargo build linux", "flaky TZ"];
        let tmp = NamedTempFile::new().unwrap();
        LexicalSegment::write_sealed(tmp.path(), &index).unwrap();
        let seg = LexicalSegment::open(tmp.path()).unwrap();
        assert_eq!(seg.docs(), index.docs());
        assert_eq!(seg.terms(), index.terms());
        for q in queries {
            let mem = index.search(q);
            let mapped = seg.search(q);
            assert_eq!(mem.len(), mapped.len(), "query {q:?}");
            for (left, right) in mem.iter().zip(&mapped) {
                assert_eq!(left.0, right.0, "query {q:?}");
                assert!(
                    (left.1 - right.1).abs() < 1e-5,
                    "query {q:?}: mem={} mmap={}",
                    left.1,
                    right.1
                );
            }
        }
        let mut mem = index.search("_SSL_CTX_new");
        let mut mapped = seg.search("_SSL_CTX_new");
        mem.sort_by(|a, b| b.1.total_cmp(&a.1));
        mapped.sort_by(|a, b| b.1.total_cmp(&a.1));
        assert_eq!(mem[0].0, mapped[0].0);
    }
}
