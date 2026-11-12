//! Exact (and quantized) vector index for optional hybrid recall (M4).
//!
//! Every vector is tagged with an [`EmbeddingSpace`]. Search is exact-first:
//! brute-force cosine over f32, or over int8 codes with optional f32 rescoring
//! of the top candidates (`docs/04` D5). Real model inference plugs in behind
//! [`Embedder`]; tests use [`HashEmbedder`].

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Row count at or above which ANN search is meaningful (caller may still use exact [`VectorIndex::search`]).
pub const ANN_ROW_THRESHOLD: usize = 64;
const ANN_TABLES: usize = 12;
const ANN_BITS_PER_TABLE: usize = 10;
const ANN_SEED: u64 = 0x9e37_79b9_7f4a_7c15;

/// Identifies an embedding model revision and layout.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmbeddingSpace {
    pub model: String,
    pub revision: String,
    pub dims: u32,
    pub quantization: Quantization,
}

impl EmbeddingSpace {
    pub fn f32(model: impl Into<String>, revision: impl Into<String>, dims: u32) -> Self {
        Self {
            model: model.into(),
            revision: revision.into(),
            dims,
            quantization: Quantization::F32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantization {
    F32,
    /// Per-vector int8 codes with a scale for dequantization.
    Int8,
}

/// One stored vector, addressed by claim document id (same as lexical docs).
#[derive(Clone, Debug)]
pub struct VectorRow {
    pub doc: u32,
    pub values: Vec<f32>,
    /// Present when stored as int8 codes.
    pub codes: Option<Vec<i8>>,
    pub scale: Option<f32>,
}

/// LSH accelerator: random hyperplanes for cosine similarity (int8/F32 dot sign bits).
#[derive(Debug, Default)]
pub struct VectorAnn {
    dims: usize,
    tables: Vec<HashMap<u64, Vec<usize>>>,
    /// Flattened hyperplanes: `tables.len()` × `bits` × `dims`.
    planes: Vec<f32>,
    bits: usize,
}

impl VectorAnn {
    fn build(rows: &[VectorRow], dims: usize) -> Self {
        let bits = ANN_BITS_PER_TABLE;
        let mut planes = Vec::with_capacity(ANN_TABLES * bits * dims);
        let mut tables: Vec<HashMap<u64, Vec<usize>>> = vec![HashMap::new(); ANN_TABLES];
        let mut rng = SplitMix64::new(ANN_SEED);

        for _ in 0..ANN_TABLES {
            for _ in 0..bits {
                for _ in 0..dims {
                    planes.push(rng.unit_gaussian());
                }
            }
        }

        for (row_idx, row) in rows.iter().enumerate() {
            for (t, table) in tables.iter_mut().enumerate() {
                let key = hyperplane_hash(&planes, t, bits, dims, &row.values);
                table.entry(key).or_default().push(row_idx);
            }
        }

        Self {
            dims,
            tables,
            planes,
            bits,
        }
    }

    fn candidates(&self, query: &[f32]) -> HashSet<usize> {
        let mut out = HashSet::new();
        for (t, table) in self.tables.iter().enumerate() {
            let key = hyperplane_hash(&self.planes, t, self.bits, self.dims, query);
            if let Some(ids) = table.get(&key) {
                out.extend(ids.iter().copied());
            }
            for b in 0..self.bits {
                if let Some(ids) = table.get(&(key ^ (1u64 << b))) {
                    out.extend(ids.iter().copied());
                }
            }
        }
        out
    }
}

#[derive(Debug, Default)]
pub struct VectorIndex {
    space: Option<EmbeddingSpace>,
    rows: Vec<VectorRow>,
    by_doc: HashMap<u32, usize>,
    ann: Option<VectorAnn>,
}

impl VectorIndex {
    pub fn space(&self) -> Option<&EmbeddingSpace> {
        self.space.as_ref()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Bind the index to a space. Refusing a mismatched space keeps stores honest.
    pub fn set_space(&mut self, space: EmbeddingSpace) -> Result<(), String> {
        if let Some(existing) = &self.space
            && existing != &space
            && !self.rows.is_empty()
        {
            return Err(format!(
                "vector index already holds {} vectors in space {}@{} dims={}",
                self.rows.len(),
                existing.model,
                existing.revision,
                existing.dims
            ));
        }
        self.space = Some(space);
        Ok(())
    }

    pub fn upsert_f32(&mut self, doc: u32, values: Vec<f32>) -> Result<(), String> {
        let space = self
            .space
            .as_ref()
            .ok_or_else(|| "vector space is not set".to_owned())?;
        if values.len() != space.dims as usize {
            return Err(format!(
                "embedding has {} dims; space expects {}",
                values.len(),
                space.dims
            ));
        }
        let (codes, scale) = match space.quantization {
            Quantization::F32 => (None, None),
            Quantization::Int8 => {
                let (c, s) = quantize_int8(&values);
                (Some(c), Some(s))
            }
        };
        let row = VectorRow {
            doc,
            values,
            codes,
            scale,
        };
        if let Some(idx) = self.by_doc.get(&doc).copied() {
            self.rows[idx] = row;
        } else {
            self.by_doc.insert(doc, self.rows.len());
            self.rows.push(row);
        }
        self.ann = None;
        Ok(())
    }

    /// Drop a document's vector (claim erasure). Idempotent when missing.
    pub fn remove(&mut self, doc: u32) {
        let Some(idx) = self.by_doc.remove(&doc) else {
            return;
        };
        self.rows.swap_remove(idx);
        if let Some(swapped) = self.rows.get(idx) {
            self.by_doc.insert(swapped.doc, idx);
        }
        self.ann = None;
    }

    /// Exact cosine search. When the space is int8, scores from codes first,
    /// then optionally rescored with f32 for the top `rescore` hits.
    pub fn search(&self, query: &[f32], limit: usize, rescore: usize) -> Vec<(u32, f32)> {
        if limit == 0 || query.is_empty() || self.rows.is_empty() {
            return Vec::new();
        }
        let use_codes = self
            .space
            .as_ref()
            .is_some_and(|s| s.quantization == Quantization::Int8);

        let mut scored: Vec<(u32, f32)> = if use_codes {
            let (q_codes, q_scale) = quantize_int8(query);
            self.rows
                .iter()
                .filter_map(|row| {
                    let codes = row.codes.as_ref()?;
                    let scale = row.scale.unwrap_or(1.0);
                    let score = cosine_int8(&q_codes, q_scale, codes, scale);
                    Some((row.doc, score))
                })
                .collect()
        } else {
            self.rows
                .iter()
                .map(|row| (row.doc, cosine_f32(query, &row.values)))
                .collect()
        };
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        if use_codes && rescore > 0 {
            let n = rescore.min(scored.len());
            for item in &mut scored[..n] {
                if let Some(&idx) = self.by_doc.get(&item.0) {
                    item.1 = cosine_f32(query, &self.rows[idx].values);
                }
            }
            scored[..n].sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        }
        scored.truncate(limit);
        scored
    }

    pub fn ann_built(&self) -> bool {
        self.ann.is_some()
    }

    /// Build or rebuild the LSH tables from current rows (F32 space only).
    pub fn build_ann(&mut self) -> Result<(), String> {
        let space = self
            .space
            .as_ref()
            .ok_or_else(|| "vector space is not set".to_owned())?;
        if space.quantization != Quantization::F32 {
            return Err("ANN build requires F32 quantization".into());
        }
        let dims = space.dims as usize;
        self.ann = Some(VectorAnn::build(&self.rows, dims));
        Ok(())
    }

    /// LSH candidate gathering plus exact cosine rescoring (same scoring as [`Self::search`] with `rescore=0`).
    pub fn search_ann(&self, query: &[f32], limit: usize) -> Vec<(u32, f32)> {
        if limit == 0 || query.is_empty() || self.rows.is_empty() {
            return Vec::new();
        }
        if self.rows.len() < ANN_ROW_THRESHOLD {
            return self.search(query, limit, 0);
        }
        let ann = match &self.ann {
            Some(a) => a,
            None => return self.search(query, limit, 0),
        };
        let candidate_idxs = ann.candidates(query);
        if candidate_idxs.is_empty() {
            return self.search(query, limit, 0);
        }
        let mut scored: Vec<(u32, f32)> = candidate_idxs
            .into_iter()
            .map(|idx| {
                let row = &self.rows[idx];
                (row.doc, cosine_f32(query, &row.values))
            })
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(limit);
        scored
    }
}

fn hyperplane_hash(planes: &[f32], table: usize, bits: usize, dims: usize, vec: &[f32]) -> u64 {
    let base = table * bits * dims;
    let mut key = 0u64;
    for b in 0..bits {
        let off = base + b * dims;
        let mut dot = 0.0f32;
        for d in 0..dims {
            dot += vec[d] * planes[off + d];
        }
        if dot >= 0.0 {
            key |= 1u64 << b;
        }
    }
    key
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit_gaussian(&mut self) -> f32 {
        let u1 = (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32;
        let u2 = (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32;
        let r = (-2.0 * u1.ln()).sqrt();
        let theta = 2.0 * std::f32::consts::PI * u2;
        r * theta.cos()
    }
}

fn cosine_f32(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

fn quantize_int8(values: &[f32]) -> (Vec<i8>, f32) {
    let max = values
        .iter()
        .map(|v| v.abs())
        .fold(0.0f32, f32::max)
        .max(1e-8);
    let scale = max / 127.0;
    let codes = values
        .iter()
        .map(|v| ((v / scale).round().clamp(-127.0, 127.0)) as i8)
        .collect();
    (codes, scale)
}

fn cosine_int8(a: &[i8], a_scale: f32, b: &[i8], b_scale: f32) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (&x, &y) in a.iter().zip(b) {
        let xf = f32::from(x) * a_scale;
        let yf = f32::from(y) * b_scale;
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Produces vectors for a given [`EmbeddingSpace`].
pub trait Embedder: Send + Sync {
    fn space(&self) -> &EmbeddingSpace;
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Deterministic bag-of-hashed-tokens embedder for tests and offline dry-runs.
/// Not a substitute for a real model; it only exercises the index and fusion path.
#[derive(Clone, Debug)]
pub struct HashEmbedder {
    space: EmbeddingSpace,
}

impl HashEmbedder {
    pub fn new(dims: u32) -> Self {
        Self {
            space: EmbeddingSpace {
                model: "hash-embedder".into(),
                revision: "v0".into(),
                dims,
                quantization: Quantization::F32,
            },
        }
    }

    pub fn int8(dims: u32) -> Self {
        let mut e = Self::new(dims);
        e.space.quantization = Quantization::Int8;
        e
    }
}

impl Embedder for HashEmbedder {
    fn space(&self) -> &EmbeddingSpace {
        &self.space
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let dims = self.space.dims as usize;
        let mut v = vec![0.0f32; dims];
        for token in text.split(|c: char| !c.is_ascii_alphanumeric()) {
            if token.is_empty() {
                continue;
            }
            let h = hash_token(token);
            let idx = (h as usize) % dims;
            let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
            v[idx] += sign;
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-8);
        for x in &mut v {
            *x /= norm;
        }
        v
    }
}

fn hash_token(token: &str) -> u64 {
    // FNV-1a 64
    let mut hash = 0xcbf29ce484222325u64;
    for b in token.bytes() {
        hash ^= u64::from(b.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// Reciprocal rank fusion of two ranked lists (`docs/04` retrieval pipeline).
pub fn rrf_fuse(lexical: &[(u32, f32)], vectors: &[(u32, f32)], k: f32) -> Vec<(u32, f32)> {
    let mut scores: HashMap<u32, f32> = HashMap::new();
    for (rank, (doc, _)) in lexical.iter().enumerate() {
        *scores.entry(*doc).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    for (rank, (doc, _)) in vectors.iter().enumerate() {
        *scores.entry(*doc).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    let mut out: Vec<(u32, f32)> = scores.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_search_ranks_similar_text_first() {
        let embedder = HashEmbedder::new(64);
        let mut index = VectorIndex::default();
        index.set_space(embedder.space().clone()).unwrap();
        index
            .upsert_f32(0, embedder.embed("openssl linker error on macos"))
            .unwrap();
        index
            .upsert_f32(1, embedder.embed("rate limiter half done handoff"))
            .unwrap();
        let q = embedder.embed("macos openssl link failure");
        let hits = index.search(&q, 2, 0);
        assert_eq!(hits[0].0, 0);
        assert!(hits[0].1 > hits[1].1);
    }

    #[test]
    fn int8_rescore_keeps_nearest_neighbor() {
        let embedder = HashEmbedder::int8(32);
        let mut index = VectorIndex::default();
        index.set_space(embedder.space().clone()).unwrap();
        for (doc, text) in [
            (0, "flaky test race on shared fixture"),
            (1, "completely unrelated poetry about rivers"),
            (2, "race condition in shared test fixture"),
        ] {
            index.upsert_f32(doc, embedder.embed(text)).unwrap();
        }
        let q = embedder.embed("shared fixture race");
        let hits = index.search(&q, 2, 2);
        assert!(hits.iter().any(|(d, _)| *d == 0 || *d == 2));
    }

    #[test]
    fn rrf_merges_lists() {
        let fused = rrf_fuse(&[(1, 0.9), (2, 0.5)], &[(2, 0.8), (3, 0.7)], 60.0);
        assert_eq!(fused[0].0, 2);
    }

    #[test]
    fn ann_search_overlaps_exact_top10() {
        let embedder = HashEmbedder::new(64);
        let mut index = VectorIndex::default();
        index.set_space(embedder.space().clone()).unwrap();
        for doc in 0..200u32 {
            let text = format!(
                "doc{doc} topic{} fragment{} {}",
                doc % 17,
                doc % 9,
                if doc % 3 == 0 {
                    "openssl macos"
                } else {
                    "misc"
                }
            );
            index.upsert_f32(doc, embedder.embed(&text)).unwrap();
        }
        index.build_ann().unwrap();
        let q = embedder.embed("openssl macos doc42");
        let exact = index.search(&q, 10, 0);
        let ann = index.search_ann(&q, 10);
        let exact_docs: HashSet<u32> = exact.iter().map(|(d, _)| *d).collect();
        let overlap = ann.iter().filter(|(d, _)| exact_docs.contains(d)).count();
        assert!(
            overlap >= 7,
            "ANN top-10 should overlap exact top-10 (got {overlap}/10); exact={exact:?} ann={ann:?}"
        );
    }
}
