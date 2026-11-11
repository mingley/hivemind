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

use hivemind_core::applicability::{self, Context};
use hivemind_core::hygiene;
use hivemind_core::{
    Actor, ActorKind, BlobHash, ClaimData, ClaimId, EVENT_VERSION, EdgeKind, Event, EventBody,
    EventId, EvidenceRef, Origin, Outcome, RetractReason, Status, Timestamp,
};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::blob::BlobStore;
use crate::codec;
use crate::error::{Error, Result};
use crate::fsutil;
use crate::log::{DEFAULT_SEGMENT_BYTES, Durability, Log, Recovery};
use crate::recall::{self, Card, ClaimDetail, Query, Recall};
use crate::state::{ArtifactRow, State};

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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            durability: Durability::Fsync,
            segment_bytes: DEFAULT_SEGMENT_BYTES,
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

impl VerifyReport {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

#[derive(Debug)]
pub struct Store {
    dir: PathBuf,
    manifest: Manifest,
    log: Mutex<Log>,
    state: RwLock<State>,
    blobs: BlobStore,
    recovery: Recovery,
    _lock: File,
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
        let mut state = State::default();
        let (log, recovery) = Log::open(
            &dir.join("log"),
            options.durability,
            options.segment_bytes,
            |offset, bytes| {
                let event = codec::decode(offset, bytes)?;
                state.apply(offset, &event);
                Ok(())
            },
        )?;
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
        data.validate().map_err(|e| Error::Invalid(e.to_string()))?;
        if let Some(kind) = data.strings().into_iter().find_map(hygiene::find_secret) {
            return Err(Error::Secret(kind));
        }
        let claim = ClaimId::generate();
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
        let similar = self.similar(claim, &supersedes);
        Ok(Receipt { similar, ..receipt })
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

    fn similar(&self, claim: ClaimId, exclude: &[ClaimId]) -> Vec<Card> {
        let state = self.state.read();
        let Some(row) = state.row(&claim) else {
            return Vec::new();
        };
        let hits = state.lexical.search(&row.data.statement);
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
                    && other.data.scope == row.data.scope
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
                let applies =
                    applicability::evaluate(&other.data.applies_when, &row.data.applies_when);
                recall::card(&state, other, score / own, applies, now)
            })
            .collect()
    }

    // ----- reads --------------------------------------------------------

    pub fn recall(&self, query: &Query) -> Recall {
        recall::recall(&self.state.read(), query)
    }

    /// The full (L1) view of a claim. With a context, applicability is evaluated too.
    pub fn get(&self, id: &ClaimId, context: Option<&Context>) -> Option<ClaimDetail> {
        let state = self.state.read();
        state
            .row(id)
            .map(|row| recall::detail(&state, row, context))
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
        let view = self.log.lock().view();
        view.read(from, limit)?
            .into_iter()
            .map(|(offset, bytes)| codec::decode(offset, &bytes).map(|e| (offset, e)))
            .collect()
    }

    pub fn stats(&self) -> Stats {
        let view = self.log.lock().view();
        let state = self.state.read();
        let mut by_status = BTreeMap::new();
        let mut by_kind = BTreeMap::new();
        for row in state.claims() {
            *by_status.entry(row.status.to_string()).or_insert(0) += 1;
            *by_kind.entry(row.data.kind.to_string()).or_insert(0) += 1;
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
