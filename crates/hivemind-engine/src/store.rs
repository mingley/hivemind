//! The store: one directory, one owning process, a log as the source of truth,
//! and in-memory projections rebuilt by replay.
//!
//! Every write is validated against the current state, appended to the log,
//! flushed, and only then applied to the projections and acknowledged. The
//! single log lock serializes writers, so validation and apply always see the
//! same state. Readers take a read lock on the projections and never wait for a
//! flush.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hivemind_core::applicability::{self, Context};
use hivemind_core::hygiene;
use hivemind_core::{
    Actor, ActorKind, BlobHash, ClaimData, ClaimId, ClaimKind, EVENT_VERSION, EdgeKind, Event,
    EventBody, EventId, EvidenceRef, Origin, Outcome, RetractReason, Scope, Status, Timestamp,
    check_promotion,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::blob::BlobStore;
use crate::codec;
use crate::error::{Error, Result};
use crate::fsutil;
use crate::indexseg::LexicalSegment;
use crate::lexical::DEFAULT_MAX_TERMS;
use crate::log::{DEFAULT_SEGMENT_BYTES, Durability, Log, Recovery};
use crate::recall::{self, Card, ClaimDetail, Flag, Query, Recall};
use crate::snapshot::{self, SnapshotInfo};
use crate::state::{ArtifactRow, LeaseRow, State};
use crate::vector::{Embedder, EmbeddingSpace};

pub const STORE_FORMAT: u32 = 1;
const MANIFEST: &str = "MANIFEST";
const LOCK: &str = "LOCK";
/// A claim counts as similar when its BM25 score against the new statement is
/// at least this fraction of the new claim's score against itself.
const SIMILARITY_THRESHOLD: f32 = 0.4;
const MAX_SIMILAR: usize = 3;
const MAX_NOTE_CHARS: usize = 2_000;
const MAX_KEY_CHARS: usize = 200;
const MAX_PRINCIPAL_CHARS: usize = 200;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub durability: Durability,
    pub segment_bytes: u64,
    /// Cap on distinct BM25 token types. New rare tokens are dropped once the
    /// dictionary reaches this size. Defaults to [`DEFAULT_MAX_TERMS`].
    pub max_terms: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            durability: Durability::Fsync,
            segment_bytes: DEFAULT_SEGMENT_BYTES,
            max_terms: DEFAULT_MAX_TERMS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: u32,
    pub store_id: String,
    pub created_at: Timestamp,
}

#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub event: EventId,
    /// Log offset of the write. Readers can pass it as a cursor or as a
    /// read-your-writes token.
    pub offset: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim: Option<ClaimId>,
    /// True when an idempotency key matched an earlier write, so nothing new was written.
    pub replayed: bool,
    /// Active claims in the same scope that look similar. Corroborate,
    /// supersede, or link them rather than keep near-duplicates.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub similar: Vec<Card>,
}

/// A request to record a new claim.
#[derive(Clone, Debug)]
pub struct Remember {
    pub data: ClaimData,
    /// Active claims this one replaces, in the same event.
    pub supersedes: Vec<ClaimId>,
    pub idempotency_key: Option<String>,
}

/// Public view of a lease projection.
#[derive(Clone, Debug, Serialize)]
pub struct LeaseView {
    pub work_item: String,
    pub token: u64,
    pub holder: String,
    pub holder_kind: ActorKind,
    pub acquired_at: Timestamp,
    pub expires_at: Timestamp,
    pub released: bool,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl LeaseView {
    fn from_row(row: &LeaseRow, now: Timestamp) -> Self {
        Self {
            work_item: row.work_item.clone(),
            token: row.token,
            holder: row.holder.to_string(),
            holder_kind: row.holder_kind,
            acquired_at: row.acquired_at,
            expires_at: row.expires_at,
            released: row.released,
            active: row.is_active(now),
            note: row.note.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LeaseReceipt {
    pub event: EventId,
    pub offset: u64,
    pub lease: LeaseView,
}

/// Result of [`Store::backup_segments`].
#[derive(Clone, Debug, Serialize)]
pub struct BackupReport {
    pub store_id: String,
    pub segments: usize,
    pub segment_bases: Vec<u64>,
    pub next_offset: u64,
    pub dest: PathBuf,
}

impl Remember {
    pub fn new(data: ClaimData) -> Self {
        Self {
            data,
            supersedes: Vec::new(),
            idempotency_key: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Stats {
    pub store_id: String,
    pub format: u32,
    pub events: u64,
    pub claims: usize,
    pub by_status: BTreeMap<String, usize>,
    pub by_kind: BTreeMap<String, usize>,
    pub edges: usize,
    pub artifacts: usize,
    pub entities: usize,
    pub terms: usize,
    /// Distinct strings in the scope/principal/entity interner.
    pub interned_strings: usize,
    pub log_segments: usize,
    pub log_bytes: u64,
    pub skipped_events: u64,
    /// Repairs recorded in the log: `(offset, truncated bytes)`.
    pub recoveries: Vec<(u64, u64)>,
    /// What recovery found when this process opened the store.
    pub recovery: Recovery,
}

#[derive(Clone, Debug, Serialize)]
pub struct VerifyReport {
    pub events: u64,
    pub log_segments: usize,
    pub blobs: usize,
    pub problems: Vec<String>,
}

/// Filters for [`Store::changes_since`]. Empty fields match everything.
#[derive(Clone, Debug, Default)]
pub struct ChangesFilter {
    pub scopes: Option<Vec<Scope>>,
    pub claims: Option<Vec<ClaimId>>,
    pub kinds: Option<Vec<ClaimKind>>,
}

impl ChangesFilter {
    fn matches(&self, store: &Store, event: &Event) -> bool {
        if self.scopes.is_none() && self.claims.is_none() && self.kinds.is_none() {
            return true;
        }
        let claim_id = event.body.claim();
        let row = claim_id.and_then(|id| store.state.read().row(&id).cloned());
        if let Some(scopes) = &self.scopes {
            let Some(row) = &row else {
                return false;
            };
            if !scopes.iter().any(|s| s == &row.scope) {
                return false;
            }
        }
        if let Some(claims) = &self.claims {
            let Some(id) = claim_id else {
                return false;
            };
            if !claims.contains(&id) {
                return false;
            }
        }
        if let Some(kinds) = &self.kinds {
            let Some(row) = &row else {
                return false;
            };
            if !kinds.iter().any(|k| k == &row.kind) {
                return false;
            }
        }
        true
    }
}

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub struct Store {
    dir: PathBuf,
    manifest: Manifest,
    log: Mutex<Log>,
    state: RwLock<State>,
    blobs: BlobStore,
    recovery: Recovery,
    /// Log offset covered by the snapshot applied at open (`0` if none).
    snapshot_offset: u64,
    embedder: Mutex<Option<Arc<dyn Embedder>>>,
    durability: Durability,
    _lock: File,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("dir", &self.dir)
            .field("store_id", &self.manifest.store_id)
            .field("has_embedder", &self.embedder.lock().is_some())
            .finish()
    }
}

impl Store {
    /// Opens the store in `dir`, creating it if needed, and replays the log.
    /// Fails with [`Error::Locked`] if another process owns the store.
    pub fn open(dir: impl AsRef<Path>, options: Options) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join(LOCK))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(Error::Locked(dir)),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        let sync = options.durability == Durability::Fsync;
        let existing = read_manifest(&dir)?;
        let mut state = State::new(options.max_terms);
        let mut snapshot_offset = 0u64;
        if let Some(snap) = snapshot::load_latest(&dir.join("snapshots"))? {
            for (offset, bytes) in &snap.events {
                let event = codec::decode(*offset, bytes)?;
                state.apply(*offset, &event);
            }
            snapshot_offset = snap.next_offset;
        }
        let (log, recovery) = Log::open_from(
            &dir.join("log"),
            options.durability,
            options.segment_bytes,
            snapshot_offset,
            |offset, bytes| {
                let event = codec::decode(offset, bytes)?;
                state.apply(offset, &event);
                Ok(())
            },
        )?;
        if snapshot_offset > log.next_offset() {
            return Err(Error::Format(format!(
                "{} snapshot covers offset {snapshot_offset} but the log ends at {}",
                dir.display(),
                log.next_offset()
            )));
        }
        let blobs = BlobStore::open(dir.join("blobs"), options.durability)?;
        if sync {
            // Make `log/` and `blobs/` themselves durable before anything in them
            // is acknowledged.
            fsutil::sync_dir(&dir)?;
        }
        // The MANIFEST is written last, so its presence proves the log was
        // created durably. A MANIFEST without a log means the log was lost.
        let manifest = match existing {
            Some(manifest) if recovery.created => {
                return Err(Error::Format(format!(
                    "{} has a MANIFEST for store {} but its log is missing; refusing to start an empty log",
                    dir.display(),
                    manifest.store_id
                )));
            }
            Some(manifest) => manifest,
            None if log.next_offset() > 0 => {
                return Err(Error::Format(format!(
                    "{} has a log with events but no MANIFEST",
                    dir.display()
                )));
            }
            None => create_manifest(&dir, sync)?,
        };
        let store = Self {
            dir,
            manifest,
            log: Mutex::new(log),
            state: RwLock::new(state),
            blobs,
            recovery,
            snapshot_offset,
            embedder: Mutex::new(None),
            durability: options.durability,
            _lock: lock,
        };
        if store.recovery.truncated_bytes > 0 {
            store.record_recovery()?;
        }
        Ok(store)
    }

    /// Appends a `recovered` event so a repair made at open is never silent.
    fn record_recovery(&self) -> Result<()> {
        let actor = Actor {
            principal: "hivemind".into(),
            kind: ActorKind::Service,
            agent: Some("recovery".into()),
            session: None,
        };
        let truncated_bytes = self.recovery.truncated_bytes;
        self.commit_event(&actor, None, |_| {
            Ok(EventBody::Recovered { truncated_bytes })
        })?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Log offset covered by the snapshot loaded at open (`0` if none).
    pub fn snapshot_offset(&self) -> u64 {
        self.snapshot_offset
    }

    /// Freeze the current in-memory lexical index to an immutable mmap segment
    /// under `indexes/lexical-<next_offset>.seg` (for RSS experiments / sealed scopes).
    pub fn seal_lexical(&self) -> Result<PathBuf> {
        let state = self.state.read();
        let offset = state.next_offset;
        let dir = self.dir.join("indexes");
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("lexical-{offset:020}.seg"));
        LexicalSegment::write_sealed(&path, &state.lexical)?;
        Ok(path)
    }

    /// Build the LSH ANN accelerator over current vectors (no-op if empty).
    pub fn build_ann(&self) -> Result<()> {
        let mut state = self.state.write();
        if state.vectors.is_empty() {
            return Ok(());
        }
        state.vectors.build_ann().map_err(Error::Invalid)
    }

    /// Write a projection checkpoint covering every acknowledged event so far.
    /// Subsequent opens apply the pack and skip re-applying those log offsets.
    pub fn checkpoint(&self) -> Result<SnapshotInfo> {
        let log = self.log.lock();
        let view = log.view();
        let next_offset = view.end();
        let mut events = Vec::with_capacity(next_offset as usize);
        view.scan(0, |offset, bytes| {
            if offset >= next_offset {
                return Ok(false);
            }
            events.push((offset, bytes.to_vec()));
            Ok(true)
        })?;
        snapshot::write(
            &self.dir.join("snapshots"),
            next_offset,
            &events,
            self.durability,
        )
    }

    /// What recovery found when the store was opened.
    pub fn recovery(&self) -> &Recovery {
        &self.recovery
    }

    /// Offset the next write will get; the number of acknowledged events.
    pub fn next_offset(&self) -> u64 {
        self.state.read().next_offset
    }

    // ----- writes -------------------------------------------------------

    /// Records a new claim, optionally superseding active claims in the same event.
    pub fn remember(&self, actor: &Actor, request: Remember) -> Result<Receipt> {
        let Remember {
            mut data,
            supersedes,
            idempotency_key,
        } = request;
        sanitize(&mut data);
        if data.review_by.is_none() {
            data.review_by = default_review_by(data.kind, Timestamp::now());
        }
        data.validate().map_err(|e| Error::Invalid(e.to_string()))?;
        if let Some(kind) = data.strings().into_iter().find_map(hygiene::find_secret) {
            return Err(Error::Secret(kind));
        }
        let claim = ClaimId::generate();
        let statement = data.statement.clone();
        let receipt = self
            .commit(actor, idempotency_key, |state| {
                let mut seen = BTreeSet::new();
                for old in &supersedes {
                    if !seen.insert(*old) {
                        return Err(Error::Invalid(format!(
                            "{old} is listed twice in supersedes"
                        )));
                    }
                    let row = state
                        .row(old)
                        .ok_or_else(|| Error::NotFound(format!("claim {old}")))?;
                    if row.status != Status::Active {
                        return Err(Error::Conflict(format!(
                            "{old} is {}; only active claims can be superseded",
                            row.status
                        )));
                    }
                    // C6: supersede only when applicability contexts match; otherwise
                    // keep a sibling and link with contrasts_with.
                    if let Some(diff) =
                        context_contrast_keys(&row.applies_when, &data.applies_when)
                    {
                        return Err(Error::Conflict(format!(
                            "{old} applies under different conditions ({}); write a sibling and link with contrasts_with instead of superseding",
                            diff.join(", ")
                        )));
                    }
                }
                for input in &data.provenance.derived_from {
                    if state.row(input).is_none() {
                        return Err(Error::NotFound(format!("claim {input} in derived_from")));
                    }
                }
                check_evidence(state, &data.provenance.evidence)?;
                Ok(Some(EventBody::ClaimAsserted {
                    claim,
                    supersedes: supersedes.clone(),
                    data: Box::new(data),
                }))
            })?
            .expect("claim writes always append an event");
        if receipt.replayed {
            return Ok(receipt);
        }
        let similar = self.similar(claim, &supersedes, &statement);
        // Drop the read guard before indexing: a temporary held across
        // `index_doc_vector` would deadlock with its write lock.
        let doc = self.state.read().row(&claim).map(|r| r.doc);
        if let Some(doc) = doc {
            self.index_doc_vector(doc, &statement);
        }
        Ok(Receipt { similar, ..receipt })
    }

    /// Attach a pure-Rust [`Embedder`]. Reindexes existing claims into the
    /// in-memory vector space. Spaces must not be mixed within one store.
    pub fn set_embedder(&self, embedder: Arc<dyn Embedder>) -> Result<()> {
        {
            let mut state = self.state.write();
            state
                .vectors
                .set_space(embedder.space().clone())
                .map_err(Error::Invalid)?;
        }
        *self.embedder.lock() = Some(embedder);
        self.reindex_vectors()?;
        Ok(())
    }

    pub fn embedding_space(&self) -> Option<EmbeddingSpace> {
        if let Some(space) = self.embedder.lock().as_ref().map(|e| e.space().clone()) {
            return Some(space);
        }
        self.state.read().vectors.space().cloned()
    }

    pub fn vector_count(&self) -> usize {
        self.state.read().vectors.len()
    }

    /// Copy durable log segments (and MANIFEST) to `dest` for cold backup / shipping.
    /// Returns the number of segment files copied. The active segment is included;
    /// callers that want only sealed segments can filter by comparing sizes later.
    pub fn backup_segments(&self, dest: &Path) -> Result<BackupReport> {
        fs::create_dir_all(dest)?;
        let log_src = self.dir.join("log");
        let log_dst = dest.join("log");
        fs::create_dir_all(&log_dst)?;
        let view = self.log.lock().view();
        let mut segments = Vec::new();
        for &base in view.segment_bases() {
            let name = format!("{base:020}.hlog");
            let from = log_src.join(&name);
            let to = log_dst.join(&name);
            fs::copy(&from, &to)?;
            segments.push(base);
        }
        let manifest_src = self.dir.join(MANIFEST);
        if manifest_src.exists() {
            fs::copy(&manifest_src, dest.join(MANIFEST))?;
        }
        Ok(BackupReport {
            store_id: self.manifest.store_id.clone(),
            segments: segments.len(),
            segment_bases: segments,
            next_offset: view.end(),
            dest: dest.to_path_buf(),
        })
    }

    /// Rebuild all vectors from claim statement caches using the current embedder.
    pub fn reindex_vectors(&self) -> Result<()> {
        let embedder = self.embedder.lock().clone();
        let Some(embedder) = embedder else {
            return Ok(());
        };
        let rows: Vec<(u32, String)> = self
            .state
            .read()
            .claims()
            .iter()
            .map(|r| (r.doc, r.statement_cache.clone()))
            .collect();
        let mut state = self.state.write();
        state
            .vectors
            .set_space(embedder.space().clone())
            .map_err(Error::Invalid)?;
        for (doc, text) in rows {
            let values = embedder.embed(&text);
            state
                .vectors
                .upsert_f32(doc, values)
                .map_err(Error::Invalid)?;
        }
        Ok(())
    }

    fn index_doc_vector(&self, doc: u32, text: &str) {
        let embedder = self.embedder.lock().clone();
        let Some(embedder) = embedder else {
            return;
        };
        let values = embedder.embed(text);
        let mut state = self.state.write();
        let _ = state.vectors.set_space(embedder.space().clone());
        let _ = state.vectors.upsert_f32(doc, values);
    }

    /// Privacy erasure: append a tombstone, purge the claim payload from the
    /// log and projections, and delete blobs that nothing else references.
    /// Forgotten claims are never returned by recall (including `--history`).
    /// Idempotent when the claim is already forgotten: re-runs the physical
    /// purge in case a prior attempt was interrupted.
    pub fn forget(
        &self,
        actor: &Actor,
        claim: ClaimId,
        reason: &str,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        let reason = clean_note(reason)?;
        if reason.trim().is_empty() {
            return Err(Error::Invalid(
                "give a reason for forgetting the claim (privacy / compliance audit)".into(),
            ));
        }

        let (offset, kind, already) = {
            let state = self.state.read();
            let row = state
                .row(&claim)
                .ok_or_else(|| Error::NotFound(format!("claim {claim}")))?;
            (row.offset, row.kind, row.status == Status::Forgotten)
        };

        // Collect exclusive blob hashes while the assert event still has them
        // (or from the redacted stub / tombstone on a re-run).
        let mut candidate_blobs = BTreeSet::new();
        if let Ok(data) = self.load_claim_data(offset) {
            for evidence in &data.provenance.evidence {
                if let EvidenceRef::Artifact(hash) = evidence {
                    candidate_blobs.insert(*hash);
                }
            }
        }
        {
            let state = self.state.read();
            if let Some(row) = state.row(&claim)
                && let Some(forgetting) = &row.forgetting
            {
                candidate_blobs.extend(forgetting.purged_blobs.iter().copied());
            }
            // Edges touching this claim may also cite artifacts.
            for edge in state.edges_of(&claim) {
                for evidence in &edge.evidence {
                    if let EvidenceRef::Artifact(hash) = evidence {
                        candidate_blobs.insert(*hash);
                    }
                }
            }
        }

        let purged_blobs: Vec<BlobHash> = {
            let state = self.state.read();
            candidate_blobs
                .into_iter()
                .filter(|hash| blob_exclusive_to(&state, *hash, claim))
                .collect()
        };

        let receipt = if already {
            // Re-purge without a second tombstone.
            Receipt {
                event: EventId::generate(),
                offset: self.next_offset(),
                claim: Some(claim),
                replayed: true,
                similar: Vec::new(),
            }
        } else {
            self.commit_event(actor, idempotency_key, |_| {
                Ok(EventBody::ClaimForgotten {
                    claim,
                    reason: reason.clone(),
                    purged_blobs: purged_blobs.clone(),
                })
            })?
        };

        // Physical purge of the ClaimAsserted payload (history rewrite).
        self.redact_claim_asserted(offset, kind)?;

        // Best-effort blob delete for exclusive artifacts.
        for hash in &purged_blobs {
            let _ = self.blobs.delete(hash);
        }

        // Snapshots may still hold pre-redaction bytes; drop them so reopen
        // replays the redacted log.
        let _ = snapshot::invalidate_all(&self.dir.join("snapshots"));

        Ok(receipt)
    }

    /// Rewrite the ClaimAsserted at `offset` to a forgotten stub (padded in place).
    fn redact_claim_asserted(&self, offset: u64, kind: ClaimKind) -> Result<()> {
        let view = self.log.lock().view();
        let rows = view.read(offset, 1)?;
        let Some((got, bytes)) = rows.into_iter().next() else {
            return Err(Error::corruption(
                self.dir.join("log"),
                offset,
                format!("no event at offset {offset}"),
            ));
        };
        if got != offset {
            return Err(Error::corruption(
                self.dir.join("log"),
                offset,
                format!("expected offset {offset}, read {got}"),
            ));
        }
        let mut event = codec::decode(offset, &bytes)?;
        match &mut event.body {
            EventBody::ClaimAsserted { data, .. } => {
                // Keep supersedes ids (graph integrity); strip every payload field.
                **data = ClaimData::forgotten_stub(kind);
            }
            EventBody::ClaimForgotten { .. } => {
                // Already a tombstone slot somehow; nothing to redact.
                return Ok(());
            }
            other => {
                return Err(Error::corruption(
                    self.dir.join("log"),
                    offset,
                    format!(
                        "offset {offset} is {}; expected claim_asserted for erasure",
                        other.type_name()
                    ),
                ));
            }
        }
        let redacted = codec::encode(&event)?;
        self.log.lock().rewrite_event(offset, &redacted)?;
        Ok(())
    }

    /// Marks a claim as wrong (`wrong`), recorded wrongly (`misrecorded`), or a
    /// duplicate. Optionally points at the claim that corrects it.
    pub fn retract(
        &self,
        actor: &Actor,
        claim: ClaimId,
        reason: RetractReason,
        note: &str,
        corrected_by: Option<ClaimId>,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        let note = clean_note(note)?;
        self.commit_event(actor, idempotency_key, |state| {
            let row = state
                .row(&claim)
                .ok_or_else(|| Error::NotFound(format!("claim {claim}")))?;
            if row.status == Status::Retracted {
                return Err(Error::Conflict(format!("{claim} is already retracted")));
            }
            if let Some(fix) = corrected_by {
                if fix == claim {
                    return Err(Error::Invalid("a claim cannot correct itself".into()));
                }
                if state.row(&fix).is_none() {
                    return Err(Error::NotFound(format!("claim {fix} in corrected_by")));
                }
            }
            Ok(EventBody::ClaimRetracted {
                claim,
                reason,
                note,
                corrected_by,
            })
        })
    }

    /// Records that an active claim stopped being true at `at` (default: now),
    /// with no replacement.
    pub fn invalidate(
        &self,
        actor: &Actor,
        claim: ClaimId,
        at: Option<Timestamp>,
        reason: &str,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        let reason = clean_note(reason)?;
        if reason.trim().is_empty() {
            return Err(Error::Invalid(
                "give a reason for invalidating the claim".into(),
            ));
        }
        if let Some(at) = at
            && !at.is_representable()
        {
            return Err(Error::Invalid(
                "`at` must fall between 1970 and the year 9999".into(),
            ));
        }
        self.commit_event(actor, idempotency_key, |state| {
            let row = state
                .row(&claim)
                .ok_or_else(|| Error::NotFound(format!("claim {claim}")))?;
            if row.status != Status::Active {
                return Err(Error::Conflict(format!(
                    "{claim} is {}; only active claims can be invalidated",
                    row.status
                )));
            }
            Ok(EventBody::ClaimInvalidated {
                claim,
                at: at.unwrap_or_else(Timestamp::now),
                reason,
            })
        })
    }

    /// Widens a claim's scope when trust policy allows it.
    ///
    /// Refuses untrusted / agent-only / tainted-without-human-review claims
    /// ([`check_promotion`]). On success, asserts a sibling claim at `to` that
    /// supersedes the original — no new [`EventBody`] variant, so erasure can
    /// add `ClaimForgotten` without fighting this change.
    pub fn promote(
        &self,
        actor: &Actor,
        claim: ClaimId,
        to: Scope,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        to.validate()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let (offset, from_scope, trust) = {
            let state = self.state.read();
            let row = state
                .row(&claim)
                .ok_or_else(|| Error::NotFound(format!("claim {claim}")))?;
            if row.status != Status::Active {
                return Err(Error::Conflict(format!(
                    "{claim} is {}; only active claims can be promoted",
                    row.status
                )));
            }
            let belief = state.belief(row);
            (row.offset, row.scope.clone(), belief.trust)
        };
        check_promotion(trust, &from_scope, &to).map_err(|e| Error::Invalid(e.to_string()))?;
        let mut data = self.load_claim_data(offset)?;
        data.scope = to;
        self.remember(
            actor,
            Remember {
                data,
                supersedes: vec![claim],
                idempotency_key,
            },
        )
    }

    /// Adds a typed relation between two claims.
    #[allow(clippy::too_many_arguments)]
    pub fn link(
        &self,
        actor: &Actor,
        from: ClaimId,
        to: ClaimId,
        kind: EdgeKind,
        note: Option<&str>,
        evidence: Vec<EvidenceRef>,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        if from == to {
            return Err(Error::Invalid("a claim cannot be linked to itself".into()));
        }
        let note = note.map(clean_note).transpose()?;
        let evidence = clean_evidence(evidence)?;
        self.commit_event(actor, idempotency_key, |state| {
            for id in [from, to] {
                if state.row(&id).is_none() {
                    return Err(Error::NotFound(format!("claim {id}")));
                }
            }
            check_evidence(state, &evidence)?;
            Ok(EventBody::EdgeAdded {
                from,
                to,
                kind,
                note,
                evidence,
            })
        })
    }

    /// Reports how a claim worked out. Helpful feedback with evidence is a
    /// validated use: it reinforces the claim and counts as independent support.
    pub fn feedback(
        &self,
        actor: &Actor,
        claim: ClaimId,
        outcome: Outcome,
        note: Option<&str>,
        evidence: Vec<EvidenceRef>,
        idempotency_key: Option<String>,
    ) -> Result<Receipt> {
        let note = note.map(clean_note).transpose()?;
        let evidence = clean_evidence(evidence)?;
        self.commit_event(actor, idempotency_key, |state| {
            if state.row(&claim).is_none() {
                return Err(Error::NotFound(format!("claim {claim}")));
            }
            check_evidence(state, &evidence)?;
            Ok(EventBody::FeedbackRecorded {
                claim,
                outcome,
                note,
                evidence,
            })
        })
    }

    /// Stores an artifact durably, then records it in the log. Storing the same
    /// bytes again returns the same hash without a new event.
    pub fn put_artifact(
        &self,
        actor: &Actor,
        bytes: &[u8],
        media_type: Option<String>,
        name: Option<String>,
        origin: Origin,
    ) -> Result<(BlobHash, Option<Receipt>)> {
        if let Some(kind) = hygiene::find_secret(&String::from_utf8_lossy(bytes)) {
            return Err(Error::Secret(kind));
        }
        let (hash, _) = self.blobs.put(bytes)?;
        let size = bytes.len() as u64;
        let receipt = self.commit(actor, None, |state| {
            if state.artifacts.contains_key(&hash) {
                return Ok(None);
            }
            Ok(Some(EventBody::ArtifactStored {
                hash,
                size,
                media_type,
                name,
                origin,
            }))
        })?;
        Ok((hash, receipt))
    }

    fn commit_event(
        &self,
        actor: &Actor,
        idempotency_key: Option<String>,
        build: impl FnOnce(&State) -> Result<EventBody>,
    ) -> Result<Receipt> {
        let receipt = self.commit(actor, idempotency_key, |state| build(state).map(Some))?;
        Ok(receipt.expect("event builders always produce an event"))
    }

    /// Validates, appends, flushes, then applies one event. Returns `None` when
    /// `build` decides there is nothing to write.
    fn commit(
        &self,
        actor: &Actor,
        idempotency_key: Option<String>,
        build: impl FnOnce(&State) -> Result<Option<EventBody>>,
    ) -> Result<Option<Receipt>> {
        check_actor(actor)?;
        if let Some(key) = &idempotency_key
            && (key.is_empty() || key.chars().count() > MAX_KEY_CHARS)
        {
            return Err(Error::Invalid(format!(
                "idempotency keys must be 1-{MAX_KEY_CHARS} characters"
            )));
        }
        let mut log = self.log.lock();
        if let Some(key) = &idempotency_key
            && let Some(ack) = self.state.read().ack(&actor.principal, key)
        {
            return Ok(Some(Receipt {
                event: ack.event,
                offset: ack.offset,
                claim: ack.claim,
                replayed: true,
                similar: Vec::new(),
            }));
        }
        let body = build(&self.state.read())?;
        let Some(body) = body else {
            return Ok(None);
        };
        let event = Event {
            v: EVENT_VERSION,
            id: EventId::generate(),
            recorded_at: Timestamp::now(),
            actor: actor.clone(),
            idempotency_key,
            body,
        };
        let bytes = codec::encode(&event)?;
        // Never acknowledge an event that replay couldn't decode: that would
        // leave a store that refuses to reopen.
        codec::decode(log.next_offset(), &bytes)
            .map_err(|e| Error::Invalid(format!("write would not survive replay: {e}")))?;
        let offset = log.append(&[bytes])?;
        let claim = match &event.body {
            EventBody::ClaimAsserted { claim, .. } => Some(*claim),
            other => other.claim(),
        };
        self.state.write().apply(offset, &event);
        Ok(Some(Receipt {
            event: event.id,
            offset,
            claim,
            replayed: false,
            similar: Vec::new(),
        }))
    }

    fn similar(&self, claim: ClaimId, exclude: &[ClaimId], statement: &str) -> Vec<Card> {
        let state = self.state.read();
        let Some(row) = state.row(&claim) else {
            return Vec::new();
        };
        let hits = state.lexical.search(statement);
        let own = hits
            .iter()
            .find(|(doc, _)| *doc == row.doc)
            .map_or(0.0, |(_, score)| *score);
        if own <= 0.0 {
            return Vec::new();
        }
        let mut found: Vec<(u32, f32)> = hits
            .into_iter()
            .filter(|(doc, score)| {
                let other = &state.claims[*doc as usize];
                *doc != row.doc
                    && score / own >= SIMILARITY_THRESHOLD
                    && other.status == Status::Active
                    && other.scope == row.scope
                    && !exclude.contains(&other.id)
            })
            .collect();
        found.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        let now = Timestamp::now();
        found
            .into_iter()
            .take(MAX_SIMILAR)
            .map(|(doc, score)| {
                let other = &state.claims[doc as usize];
                // Compare conditions: a similar claim under other conditions shows as a mismatch.
                let applies = applicability::evaluate(&other.applies_when, &row.applies_when);
                let mut card = recall::card(&state, other, score / own, applies, now);
                if let Some(keys) = context_contrast_keys(&other.applies_when, &row.applies_when) {
                    card.flags.push(Flag::ContextContrast);
                    card.differs_in = keys;
                }
                card
            })
            .collect()
    }

    /// Loads the full [`ClaimData`] for a claim from its log offset.
    pub(crate) fn load_claim_data(&self, offset: u64) -> Result<ClaimData> {
        let view = self.log.lock().view();
        let rows = view.read(offset, 1)?;
        let Some((got, bytes)) = rows.into_iter().next() else {
            return Err(Error::corruption(
                self.dir.join("log"),
                offset,
                format!("no event at offset {offset}"),
            ));
        };
        if got != offset {
            return Err(Error::corruption(
                self.dir.join("log"),
                offset,
                format!("expected offset {offset}, read {got}"),
            ));
        }
        let event = codec::decode(offset, &bytes)?;
        match event.body {
            EventBody::ClaimAsserted { data, .. } => Ok(*data),
            _ => Err(Error::corruption(
                self.dir.join("log"),
                offset,
                format!("offset {offset} is not a ClaimAsserted event"),
            )),
        }
    }

    // ----- reads --------------------------------------------------------

    pub fn recall(&self, query: &Query) -> Recall {
        let mut query = query.clone();
        if query.query_vector.is_none()
            && !query.text.trim().is_empty()
            && let Some(embedder) = self.embedder.lock().clone()
        {
            query.query_vector = Some(embedder.embed(&query.text));
        }
        recall::recall(&self.state.read(), &query)
    }

    /// The full (L1) view of a claim. With a context, applicability is evaluated too.
    /// Reloads the claim body from the log by the keydir offset.
    /// Forgotten claims return `None` (payload purged; use stats / log tombstones).
    pub fn get(&self, id: &ClaimId, context: Option<&Context>) -> Option<ClaimDetail> {
        self.get_with_workspace(id, context, None)
    }

    /// Like [`get`], and when `workspace` is set, verifies code citations on disk.
    pub fn get_with_workspace(
        &self,
        id: &ClaimId,
        context: Option<&Context>,
        workspace: Option<&std::path::Path>,
    ) -> Option<ClaimDetail> {
        let (offset, _) = {
            let state = self.state.read();
            let row = state.row(id)?;
            if row.status == Status::Forgotten {
                return None;
            }
            (row.offset, row.id)
        };
        let data = self.load_claim_data(offset).ok()?;
        let checks = workspace
            .map(|root| hivemind_core::verify_citations(root, &data.provenance.citations))
            .unwrap_or_default();
        let state = self.state.read();
        let row = state.row(id)?;
        Some(recall::detail_with_citations(
            &state, row, &data, context, checks,
        ))
    }

    /// Resolves a full claim ID or a unique prefix of at least 4 hex digits.
    pub fn resolve(&self, text: &str) -> Result<ClaimId> {
        let state = self.state.read();
        if let Ok(id) = text.parse::<ClaimId>() {
            return state
                .row(&id)
                .map(|_| id)
                .ok_or_else(|| Error::NotFound(format!("claim {id}")));
        }
        let prefix = text.trim().trim_start_matches("clm_").to_ascii_lowercase();
        if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::Invalid(format!(
                "`{text}` is not a claim ID or a prefix of at least 4 hex digits"
            )));
        }
        let mut matches = state
            .claims()
            .iter()
            .filter(|row| row.id.as_uuid().simple().to_string().starts_with(&prefix));
        match (matches.next(), matches.next()) {
            (Some(row), None) => Ok(row.id),
            (None, _) => Err(Error::NotFound(format!("no claim matches `{text}`"))),
            (Some(_), Some(_)) => Err(Error::Invalid(format!(
                "`{text}` matches more than one claim; use more digits"
            ))),
        }
    }

    pub fn artifact(&self, hash: &BlobHash) -> Result<Vec<u8>> {
        self.blobs.get(hash)
    }

    pub fn artifact_info(&self, hash: &BlobHash) -> Option<ArtifactRow> {
        self.state.read().artifacts.get(hash).cloned()
    }

    /// Events from offset `from`, in order: the change feed. Pass the offset
    /// after the last event you saw as the next cursor.
    pub fn events(&self, from: u64, limit: usize) -> Result<Vec<(u64, Event)>> {
        self.changes_since(from, limit, &ChangesFilter::default())
    }

    /// Change feed with optional scope / claim / kind filters.
    pub fn changes_since(
        &self,
        from: u64,
        limit: usize,
        filter: &ChangesFilter,
    ) -> Result<Vec<(u64, Event)>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let view = self.log.lock().view();
        let mut out = Vec::new();
        let mut cursor = from;
        // Scan ahead of `limit` so filtered feeds still make progress.
        let batch = limit.saturating_mul(4).max(limit);
        loop {
            let batch_rows = view.read(cursor, batch)?;
            if batch_rows.is_empty() {
                break;
            }
            for (offset, bytes) in batch_rows {
                cursor = offset + 1;
                let event = codec::decode(offset, &bytes)?;
                if filter.matches(self, &event) {
                    out.push((offset, event));
                    if out.len() >= limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Blocks until projections have applied through `min_offset`, or until
    /// `timeout` elapses. Used for read-your-writes after a write receipt.
    pub fn wait_for_offset(&self, min_offset: u64, timeout: Duration) -> Result<()> {
        if self.next_offset() > min_offset {
            return Ok(());
        }
        // `next_offset` is the next write's offset, so reaching `min_offset`
        // means next_offset > min_offset (the event at min_offset is applied).
        let deadline = Instant::now() + timeout;
        while self.next_offset() <= min_offset {
            if Instant::now() >= deadline {
                return Err(Error::Timeout(format!(
                    "projections at {} have not reached offset {min_offset}",
                    self.next_offset().saturating_sub(1)
                )));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }

    /// Acquire an exclusive lease on `work_item`. Fails if another principal
    /// still holds an unexpired lease. Returns a fencing token that must be
    /// presented on renew/release (and optionally on gated writes).
    pub fn acquire_lease(
        &self,
        actor: &Actor,
        work_item: &str,
        ttl: std::time::Duration,
        note: Option<&str>,
    ) -> Result<LeaseReceipt> {
        let work_item = clean_work_item(work_item)?;
        let note = note.map(clean_note).transpose()?.filter(|s| !s.is_empty());
        let ttl_ms = duration_millis(ttl)?;
        let receipt = self.commit_event(actor, None, |state| {
            let now = Timestamp::now();
            let next_token = match state.leases.get(&work_item) {
                Some(lease) if lease.is_active(now) && lease.holder.as_ref() != actor.principal => {
                    return Err(Error::Conflict(format!(
                        "work item `{work_item}` is held by {} until {} (token {})",
                        lease.holder,
                        lease.expires_at.to_rfc3339(),
                        lease.token
                    )));
                }
                Some(lease) => lease.token.saturating_add(1).max(1),
                None => 1,
            };
            Ok(EventBody::LeaseAcquired {
                work_item: work_item.clone(),
                token: next_token,
                expires_at: now.plus_millis(ttl_ms),
                note: note.clone(),
            })
        })?;
        let lease = self.lease(&work_item).expect("lease just acquired");
        Ok(LeaseReceipt {
            event: receipt.event,
            offset: receipt.offset,
            lease,
        })
    }

    /// Extend a held lease. The fencing token must match.
    pub fn renew_lease(
        &self,
        actor: &Actor,
        work_item: &str,
        token: u64,
        ttl: std::time::Duration,
    ) -> Result<LeaseReceipt> {
        let work_item = clean_work_item(work_item)?;
        let ttl_ms = duration_millis(ttl)?;
        let receipt = self.commit_event(actor, None, |state| {
            let now = Timestamp::now();
            let lease = state
                .leases
                .get(&work_item)
                .ok_or_else(|| Error::NotFound(format!("no lease for `{work_item}`")))?;
            check_fence(lease, actor, token, now)?;
            Ok(EventBody::LeaseRenewed {
                work_item: work_item.clone(),
                token,
                expires_at: now.plus_millis(ttl_ms),
            })
        })?;
        let lease = self.lease(&work_item).expect("lease just renewed");
        Ok(LeaseReceipt {
            event: receipt.event,
            offset: receipt.offset,
            lease,
        })
    }

    /// Release a held lease. The fencing token must match.
    pub fn release_lease(
        &self,
        actor: &Actor,
        work_item: &str,
        token: u64,
    ) -> Result<LeaseReceipt> {
        let work_item = clean_work_item(work_item)?;
        let receipt = self.commit_event(actor, None, |state| {
            let now = Timestamp::now();
            let lease = state
                .leases
                .get(&work_item)
                .ok_or_else(|| Error::NotFound(format!("no lease for `{work_item}`")))?;
            check_fence(lease, actor, token, now)?;
            Ok(EventBody::LeaseReleased {
                work_item: work_item.clone(),
                token,
            })
        })?;
        let lease = self.lease(&work_item).expect("lease just released");
        Ok(LeaseReceipt {
            event: receipt.event,
            offset: receipt.offset,
            lease,
        })
    }

    /// Current lease projection for a work item, if any.
    pub fn lease(&self, work_item: &str) -> Option<LeaseView> {
        let state = self.state.read();
        let lease = state.leases.get(work_item)?;
        Some(LeaseView::from_row(lease, Timestamp::now()))
    }

    /// List leases (active first). Expired/released rows remain for audit.
    pub fn leases(&self) -> Vec<LeaseView> {
        let now = Timestamp::now();
        let state = self.state.read();
        let mut rows: Vec<LeaseView> = state
            .leases
            .values()
            .map(|row| LeaseView::from_row(row, now))
            .collect();
        rows.sort_by(|a, b| b.active.cmp(&a.active).then(a.work_item.cmp(&b.work_item)));
        rows
    }

    /// Rejects a stale fencing token. Used by callers that gate writes on a lease.
    pub fn check_lease_token(&self, work_item: &str, token: u64, principal: &str) -> Result<()> {
        let work_item = clean_work_item(work_item)?;
        let state = self.state.read();
        let lease = state
            .leases
            .get(&work_item)
            .ok_or_else(|| Error::NotFound(format!("no lease for `{work_item}`")))?;
        let actor = Actor::agent(principal);
        check_fence(lease, &actor, token, Timestamp::now())
    }

    pub fn stats(&self) -> Stats {
        let view = self.log.lock().view();
        let state = self.state.read();
        let mut by_status = BTreeMap::new();
        let mut by_kind = BTreeMap::new();
        for row in state.claims() {
            *by_status.entry(row.status.to_string()).or_insert(0) += 1;
            *by_kind.entry(row.kind.to_string()).or_insert(0) += 1;
        }
        Stats {
            store_id: self.manifest.store_id.clone(),
            format: self.manifest.format,
            events: state.next_offset,
            claims: state.claims().len(),
            by_status,
            by_kind,
            edges: state.edges.len(),
            artifacts: state.artifacts.len(),
            entities: state.by_entity.len(),
            terms: state.lexical.terms(),
            interned_strings: state.intern.len(),
            log_segments: view.segment_count(),
            log_bytes: view.bytes(),
            skipped_events: state.skipped,
            recoveries: state.recoveries.clone(),
            recovery: self.recovery.clone(),
        }
    }

    /// Re-reads the whole log and every blob, checking checksums, decoding, and
    /// references.
    pub fn verify(&self) -> Result<VerifyReport> {
        let view = self.log.lock().view();
        let mut problems = Vec::new();
        let mut events = 0u64;
        let scanned = view.scan(0, |offset, bytes| {
            if let Err(e) = codec::decode(offset, bytes) {
                problems.push(e.to_string());
            }
            events += 1;
            Ok(true)
        });
        if let Err(e) = scanned {
            problems.push(e.to_string());
        }
        if events != view.end() {
            problems.push(format!(
                "read {events} events; the log holds {}",
                view.end()
            ));
        }
        let (blobs, blob_problems) = self.blobs.verify_all()?;
        problems.extend(blob_problems);
        for hash in self.state.read().artifacts.keys() {
            if !self.blobs.contains(hash) {
                problems.push(format!(
                    "artifact {hash} is recorded but its blob is missing"
                ));
            }
        }
        Ok(VerifyReport {
            events,
            log_segments: view.segment_count(),
            blobs,
            problems,
        })
    }
}

fn read_manifest(dir: &Path) -> Result<Option<Manifest>> {
    match fs::read(dir.join(MANIFEST)) {
        Ok(bytes) => {
            let manifest: Manifest = serde_json::from_slice(&bytes)
                .map_err(|e| Error::Format(format!("unreadable MANIFEST: {e}")))?;
            if manifest.format != STORE_FORMAT {
                return Err(Error::Format(format!(
                    "store format {} is not supported by this build (format {STORE_FORMAT})",
                    manifest.format
                )));
            }
            Ok(Some(manifest))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn create_manifest(dir: &Path, sync: bool) -> Result<Manifest> {
    let manifest = Manifest {
        format: STORE_FORMAT,
        store_id: uuid::Uuid::now_v7().to_string(),
        created_at: Timestamp::now(),
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|e| Error::Invalid(e.to_string()))?;
    fsutil::write_atomic(&dir.join(MANIFEST), &bytes, sync)?;
    Ok(manifest)
}

fn check_actor(actor: &Actor) -> Result<()> {
    let len = actor.principal.chars().count();
    if actor.principal.trim().is_empty() || len > MAX_PRINCIPAL_CHARS {
        return Err(Error::Invalid(format!(
            "principal must be 1-{MAX_PRINCIPAL_CHARS} characters"
        )));
    }
    Ok(())
}

/// True when no non-forgotten claim (other than `except`) still cites `hash`.
fn blob_exclusive_to(state: &State, hash: BlobHash, except: ClaimId) -> bool {
    let key = hash.to_string();
    for row in state.claims() {
        if row.id == except || row.status == Status::Forgotten {
            continue;
        }
        if row.evidence_keys.iter().any(|k| k.as_ref() == key) {
            return false;
        }
    }
    for edge in &state.edges {
        // Edges on the claim being forgotten are scrubbed with it.
        if edge.from == except || edge.to == except {
            continue;
        }
        for evidence in &edge.evidence {
            if matches!(evidence, EvidenceRef::Artifact(h) if *h == hash) {
                return false;
            }
        }
    }
    true
}

fn clean_work_item(work_item: &str) -> Result<String> {
    let work_item = hygiene::strip_invisible(work_item).trim().to_owned();
    let len = work_item.chars().count();
    if work_item.is_empty() || len > MAX_KEY_CHARS {
        return Err(Error::Invalid(format!(
            "work item ids must be 1-{MAX_KEY_CHARS} characters"
        )));
    }
    Ok(work_item)
}

fn duration_millis(ttl: Duration) -> Result<i64> {
    let ms = i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX);
    if ms <= 0 {
        return Err(Error::Invalid("lease TTL must be positive".into()));
    }
    Ok(ms)
}

fn check_fence(lease: &LeaseRow, actor: &Actor, token: u64, now: Timestamp) -> Result<()> {
    if lease.released {
        return Err(Error::Conflict(format!(
            "lease for `{}` is already released (token {})",
            lease.work_item, lease.token
        )));
    }
    if lease.expires_at < now {
        return Err(Error::Conflict(format!(
            "lease for `{}` expired at {} (token {})",
            lease.work_item,
            lease.expires_at.to_rfc3339(),
            lease.token
        )));
    }
    if lease.token != token {
        return Err(Error::Conflict(format!(
            "stale fencing token {token} for `{}`; current token is {}",
            lease.work_item, lease.token
        )));
    }
    if lease.holder.as_ref() != actor.principal {
        return Err(Error::Conflict(format!(
            "lease for `{}` is held by {}, not {}",
            lease.work_item, lease.holder, actor.principal
        )));
    }
    Ok(())
}

/// Default review-by dates when the writer omits one: handoffs fade quickly;
/// decisions and preferences do not auto-expire.
fn default_review_by(kind: ClaimKind, now: Timestamp) -> Option<Timestamp> {
    match kind {
        ClaimKind::Handoff => Some(now.plus_days(7)),
        ClaimKind::Question => Some(now.plus_days(30)),
        ClaimKind::Fact | ClaimKind::Solution | ClaimKind::Warning | ClaimKind::Procedure => {
            Some(now.plus_days(180))
        }
        ClaimKind::Decision | ClaimKind::Preference => None,
    }
}

/// Keys where two applicability maps disagree (present in both, different values).
/// `None` means compatible for supersession (C6).
fn context_contrast_keys(
    a: &std::collections::BTreeMap<String, String>,
    b: &std::collections::BTreeMap<String, String>,
) -> Option<Vec<String>> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    let mut keys = Vec::new();
    for (key, av) in a {
        if let Some(bv) = b.get(key)
            && av != bv
        {
            keys.push(key.clone());
        }
    }
    if keys.is_empty() {
        None
    } else {
        keys.sort();
        Some(keys)
    }
}

fn clean_note(note: &str) -> Result<String> {
    let note = hygiene::strip_invisible(note).trim().to_owned();
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(Error::Invalid(format!(
            "notes are limited to {MAX_NOTE_CHARS} characters"
        )));
    }
    if let Some(kind) = hygiene::find_secret(&note) {
        return Err(Error::Secret(kind));
    }
    Ok(note)
}

fn clean_evidence(evidence: Vec<EvidenceRef>) -> Result<Vec<EvidenceRef>> {
    evidence
        .into_iter()
        .map(|e| match e {
            EvidenceRef::Quote(q) => clean_note(&q).map(EvidenceRef::Quote),
            other => Ok(other),
        })
        .collect()
}

fn check_evidence(state: &State, evidence: &[EvidenceRef]) -> Result<()> {
    for item in evidence {
        match item {
            EvidenceRef::Artifact(hash) if !state.artifacts.contains_key(hash) => {
                return Err(Error::NotFound(format!(
                    "artifact {hash}; store it before citing it as evidence"
                )));
            }
            EvidenceRef::Claim(id) if state.row(id).is_none() => {
                return Err(Error::NotFound(format!("claim {id} cited as evidence")));
            }
            EvidenceRef::Url(url)
                if !(url.starts_with("https://") || url.starts_with("http://")) =>
            {
                return Err(Error::Invalid(format!(
                    "evidence URL `{url}` must be http(s)"
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Strips invisible characters from every user-supplied string.
fn sanitize(data: &mut ClaimData) {
    fn clean(text: &mut String) {
        if let std::borrow::Cow::Owned(stripped) = hygiene::strip_invisible(text) {
            *text = stripped;
        }
    }
    fn clean_json(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::String(s) => clean(s),
            serde_json::Value::Array(items) => items.iter_mut().for_each(clean_json),
            serde_json::Value::Object(map) => map.values_mut().for_each(clean_json),
            _ => {}
        }
    }
    clean(&mut data.statement);
    data.statement = data.statement.trim().to_owned();
    for entity in &mut data.about {
        clean(entity);
        *entity = entity.trim().to_owned();
    }
    data.payload.values_mut().for_each(clean_json);
    for evidence in &mut data.provenance.evidence {
        if let EvidenceRef::Quote(quote) = evidence {
            clean(quote);
        }
    }
}
