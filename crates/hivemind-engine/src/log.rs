//! The segmented, checksummed, append-only log: the store's source of truth.
//!
//! Segments live in `log/<base_offset:020>.hlog`. Each starts with a 32-byte
//! header, followed by records. **One record is one commit**: every event of an
//! append is in one record, and the record is flushed to disk before the append
//! is acknowledged.
//!
//! ```text
//! header: magic "HMLOG\0\0\x01" | format u32 | reserved u32 | base_offset u64
//!         | crc32c(header[0..24]) u32 | zero u32
//! record: len u32 | crc32c(len ‖ first_offset ‖ count ‖ payload) u32
//!         | first_offset u64 | count u32 | payload[len]
//! payload: count × (event_len u32 ‖ event bytes)
//! ```
//!
//! All integers are little-endian. Events are numbered by a contiguous offset.
//!
//! Recovery reads every record in order and checks checksums, structure, and
//! offset continuity. A damaged record at the end of the last segment is a torn
//! write, an append whose flush never completed, so it was never acknowledged.
//! It is truncated. Damage anywhere else, or damage followed by a valid record,
//! means acknowledged data is corrupt. The store then refuses to open instead of
//! dropping data silently. A later record can exist only if the earlier one was
//! flushed first.

use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::fsutil::{self, ensure_disk_space};

pub const SEGMENT_MAGIC: [u8; 8] = *b"HMLOG\0\0\x01";
/// On-disk segment header format. Bump only with a deliberate migration; golden
/// fixtures in `tests/fixtures/log/` freeze the bytes for this value.
pub const SEGMENT_FORMAT: u32 = 1;
pub const SEGMENT_HEADER_LEN: u64 = 32;
pub const RECORD_HEADER_LEN: u64 = 20;
/// Largest record payload. Large content belongs in the blob store.
pub const MAX_RECORD_LEN: u32 = 16 * 1024 * 1024;
pub const DEFAULT_SEGMENT_BYTES: u64 = 64 * 1024 * 1024;
const SEGMENT_SUFFIX: &str = ".hlog";

/// Why a commit record failed to decode. Soft failures (torn tails) use this
/// rather than [`Error::Corruption`], which needs a file path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FrameError {
    TruncatedHeader,
    TruncatedPayload,
    LengthExceedsLimit(u32),
    ChecksumMismatch,
    OffsetMismatch { got: u64, expected: u64 },
    MalformedPayload,
    EmptyBatch,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::TruncatedHeader => f.write_str("truncated record header"),
            FrameError::TruncatedPayload => f.write_str("truncated record payload"),
            FrameError::LengthExceedsLimit(len) => {
                write!(f, "record length {len} exceeds the limit")
            }
            FrameError::ChecksumMismatch => f.write_str("record checksum mismatch"),
            FrameError::OffsetMismatch { got, expected } => {
                write!(f, "record starts at offset {got}, expected {expected}")
            }
            FrameError::MalformedPayload => f.write_str("record payload is malformed"),
            FrameError::EmptyBatch => f.write_str("record payload is malformed"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    /// Flush to stable storage before acknowledging (`F_FULLFSYNC` on macOS).
    Fsync,
    /// No flush. Survives a process crash but not power loss. Tests and
    /// benchmarks only.
    NoFsync,
}

/// What recovery found when the log was opened.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Recovery {
    pub segments: usize,
    pub records: u64,
    pub events: u64,
    /// Bytes of an unacknowledged, torn append removed from the last segment.
    pub truncated_bytes: u64,
    /// The log didn't exist and was created by this open.
    pub created: bool,
}

#[derive(Debug)]
pub struct Log {
    dir: PathBuf,
    durability: Durability,
    segment_bytes: u64,
    segments: Vec<u64>,
    active: File,
    active_len: u64,
    next_offset: u64,
    poisoned: bool,
}

impl Log {
    /// Opens or creates the log in `dir`, replaying every acknowledged event
    /// into `visit` in offset order.
    pub fn open(
        dir: &Path,
        durability: Durability,
        segment_bytes: u64,
        visit: impl FnMut(u64, &[u8]) -> Result<()>,
    ) -> Result<(Self, Recovery)> {
        Self::open_from(dir, durability, segment_bytes, 0, visit)
    }

    /// Like [`Log::open`], but only invokes `visit` for events with
    /// `offset >= apply_from`. Earlier events are still validated. Used when
    /// projections were loaded from a snapshot covering `[0, apply_from)`.
    pub fn open_from(
        dir: &Path,
        durability: Durability,
        segment_bytes: u64,
        apply_from: u64,
        mut visit: impl FnMut(u64, &[u8]) -> Result<()>,
    ) -> Result<(Self, Recovery)> {
        fs::create_dir_all(dir)?;
        let mut segments = list_segments(dir)?;
        let created = segments.is_empty();
        if created {
            create_segment(dir, 0, durability)?;
            segments.push(0);
        }

        let mut recovery = Recovery {
            segments: segments.len(),
            created,
            ..Recovery::default()
        };
        let mut expected = 0u64;
        let mut active_len = SEGMENT_HEADER_LEN;
        let last = segments.len() - 1;
        for (i, &base) in segments.iter().enumerate() {
            let path = segment_path(dir, base);
            if base != expected {
                return Err(Error::corruption(
                    &path,
                    0,
                    format!(
                        "segment starts at offset {base}, but the log is contiguous up to {expected}"
                    ),
                ));
            }
            let mut reader = SegmentReader::open(&path, base)?;
            let problem = loop {
                match reader.step()? {
                    Step::Record => {
                        for (offset, event) in reader.events() {
                            if offset >= apply_from {
                                visit(offset, event)?;
                            }
                            recovery.events += 1;
                        }
                        recovery.records += 1;
                    }
                    Step::End => break None,
                    Step::Bad { position, reason } => break Some((position, reason)),
                }
            };
            expected = reader.next_offset;
            let valid_len = reader.pos;
            if let Some((position, reason)) = problem {
                if i != last {
                    return Err(Error::corruption(&path, position, reason));
                }
                if let Some(later) = find_record_after(&path, position, expected)? {
                    return Err(Error::corruption(
                        &path,
                        position,
                        format!(
                            "{reason}; a valid record follows at byte {later}, so this is not a torn write"
                        ),
                    ));
                }
                let file = OpenOptions::new().write(true).open(&path)?;
                let len = file.metadata()?.len();
                file.set_len(valid_len)?;
                if durability == Durability::Fsync {
                    file.sync_all()?;
                }
                recovery.truncated_bytes = len - valid_len;
            }
            if i == last {
                active_len = valid_len;
            }
        }

        let active_base = segments[last];
        let mut active = OpenOptions::new()
            .read(true)
            .write(true)
            .open(segment_path(dir, active_base))?;
        active.seek(SeekFrom::Start(active_len))?;
        let log = Log {
            dir: dir.to_path_buf(),
            durability,
            segment_bytes,
            segments,
            active,
            active_len,
            next_offset: expected,
            poisoned: false,
        };
        Ok((log, recovery))
    }

    /// Offset the next event will get; equals the number of acknowledged events.
    pub fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub fn durability(&self) -> Durability {
        self.durability
    }

    /// Appends `events` as one commit and returns the offset of the first.
    /// When this returns `Ok`, every event is durable under the log's
    /// [`Durability`]. Any I/O failure poisons the log: after a failed flush
    /// the file's state is unknown, so writes stop until the store is reopened
    /// and recovery has run.
    pub fn append(&mut self, events: &[Vec<u8>]) -> Result<u64> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        let first = self.next_offset;
        if events.is_empty() {
            return Ok(first);
        }
        let record = encode_record(first, events)?;
        let count = u32::try_from(events.len()).expect("checked in encode_record");

        // Refuse before any write so a DiskFull (or similar) does not poison.
        let mut needed = record.len() as u64;
        if self.active_len >= self.segment_bytes {
            needed = needed.saturating_add(SEGMENT_HEADER_LEN);
        }
        ensure_disk_space(&self.dir, needed)?;

        let result = self.write_record(&record);
        if let Err(e) = result {
            self.poisoned = true;
            return Err(e);
        }
        self.next_offset = first + u64::from(count);
        Ok(first)
    }

    fn write_record(&mut self, record: &[u8]) -> Result<()> {
        if self.active_len >= self.segment_bytes {
            self.roll()?;
        }
        self.active.write_all(record)?;
        if self.durability == Durability::Fsync {
            self.active.sync_data()?;
        }
        self.active_len += record.len() as u64;
        Ok(())
    }

    fn roll(&mut self) -> Result<()> {
        let base = self.next_offset;
        create_segment(&self.dir, base, self.durability)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(segment_path(&self.dir, base))?;
        file.seek(SeekFrom::Start(SEGMENT_HEADER_LEN))?;
        self.segments.push(base);
        self.active = file;
        self.active_len = SEGMENT_HEADER_LEN;
        Ok(())
    }

    /// A read-only view of everything acknowledged so far.
    pub fn view(&self) -> LogView {
        LogView {
            dir: self.dir.clone(),
            segments: self.segments.clone(),
            end: self.next_offset,
        }
    }
}

/// A consistent, read-only view of the log up to the offset that was
/// acknowledged when the view was taken. Safe to use while the log appends.
#[derive(Clone, Debug)]
pub struct LogView {
    dir: PathBuf,
    segments: Vec<u64>,
    end: u64,
}

impl LogView {
    pub fn end(&self) -> u64 {
        self.end
    }

    /// Events with offsets in `[from, end)`, at most `limit` of them.
    pub fn read(&self, from: u64, limit: usize) -> Result<Vec<(u64, Vec<u8>)>> {
        let mut out = Vec::new();
        if limit == 0 {
            return Ok(out);
        }
        self.scan(from, |offset, event| {
            out.push((offset, event.to_vec()));
            Ok(out.len() < limit)
        })?;
        Ok(out)
    }

    /// Streams events with offsets in `[from, end)` to `visit` in order, until
    /// `visit` returns `Ok(false)`.
    pub fn scan(&self, from: u64, mut visit: impl FnMut(u64, &[u8]) -> Result<bool>) -> Result<()> {
        if from >= self.end {
            return Ok(());
        }
        let start = self
            .segments
            .partition_point(|base| *base <= from)
            .saturating_sub(1);
        for &base in &self.segments[start..] {
            let mut reader = SegmentReader::open(&segment_path(&self.dir, base), base)?;
            loop {
                match reader.step()? {
                    Step::Record => {
                        for (offset, event) in reader.events() {
                            if offset >= self.end {
                                return Ok(());
                            }
                            if offset >= from && !visit(offset, event)? {
                                return Ok(());
                            }
                        }
                    }
                    Step::End => break,
                    Step::Bad { position, reason } => {
                        if reader.next_offset >= self.end {
                            // Past the acknowledged end: an append in progress.
                            return Ok(());
                        }
                        return Err(Error::corruption(
                            segment_path(&self.dir, base),
                            position,
                            reason,
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Total size of the log's segment files.
    pub fn bytes(&self) -> u64 {
        self.segments
            .iter()
            .filter_map(|base| fs::metadata(segment_path(&self.dir, *base)).ok())
            .map(|m| m.len())
            .sum()
    }

    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    /// Base offsets of every segment file in this view (including the active one).
    pub fn segment_bases(&self) -> &[u64] {
        &self.segments
    }
}

fn segment_path(dir: &Path, base: u64) -> PathBuf {
    dir.join(format!("{base:020}{SEGMENT_SUFFIX}"))
}

fn list_segments(dir: &Path) -> Result<Vec<u64>> {
    let mut bases = Vec::new();
    for entry in fs::read_dir(dir)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(stem) = name.strip_suffix(SEGMENT_SUFFIX) {
            match stem.parse::<u64>() {
                Ok(base) if stem.len() == 20 => bases.push(base),
                _ => {
                    return Err(Error::Format(format!(
                        "unexpected file `{name}` in {}",
                        dir.display()
                    )));
                }
            }
        }
    }
    bases.sort_unstable();
    Ok(bases)
}

/// Builds the 32-byte segment header for `base` (format [`SEGMENT_FORMAT`]).
pub fn encode_segment_header(base: u64) -> [u8; SEGMENT_HEADER_LEN as usize] {
    let mut header = [0u8; SEGMENT_HEADER_LEN as usize];
    header[0..8].copy_from_slice(&SEGMENT_MAGIC);
    header[8..12].copy_from_slice(&SEGMENT_FORMAT.to_le_bytes());
    header[16..24].copy_from_slice(&base.to_le_bytes());
    let crc = crc32c::crc32c(&header[0..24]);
    header[24..28].copy_from_slice(&crc.to_le_bytes());
    header
}

/// Encodes one commit record: `len ‖ crc ‖ first_offset ‖ count ‖ payload`.
///
/// The CRC covers `len ‖ first_offset ‖ count ‖ payload` (everything except the
/// CRC field). Empty `events` is rejected; oversized batches return
/// [`Error::RecordTooLarge`].
pub fn encode_record(first_offset: u64, events: &[Vec<u8>]) -> Result<Vec<u8>> {
    if events.is_empty() {
        return Err(Error::Invalid(
            "commit record requires at least one event".into(),
        ));
    }
    let payload_len: usize = events.iter().map(|e| 4 + e.len()).sum();
    let len = u32::try_from(payload_len)
        .ok()
        .filter(|len| *len <= MAX_RECORD_LEN)
        .ok_or(Error::RecordTooLarge(payload_len))?;
    let count = u32::try_from(events.len()).map_err(|_| Error::RecordTooLarge(payload_len))?;

    let mut record = Vec::with_capacity(RECORD_HEADER_LEN as usize + payload_len);
    record.extend_from_slice(&len.to_le_bytes());
    record.extend_from_slice(&[0; 4]);
    record.extend_from_slice(&first_offset.to_le_bytes());
    record.extend_from_slice(&count.to_le_bytes());
    for event in events {
        // Each length fits: the total was checked against MAX_RECORD_LEN.
        record.extend_from_slice(&(event.len() as u32).to_le_bytes());
        record.extend_from_slice(event);
    }
    let crc = record_crc(&record);
    record[4..8].copy_from_slice(&crc.to_le_bytes());
    Ok(record)
}

/// One decoded commit: bytes consumed, then `(offset, event_bytes)` pairs.
pub type DecodedRecord = (usize, Vec<(u64, Vec<u8>)>);

/// Validates a commit record that starts at `bytes[0]` and returns how many
/// bytes it consumes plus each `(offset, event)` pair.
///
/// Used by recovery and by property/fuzz tests so the on-disk layout stays
/// covered without going through the filesystem.
pub fn try_decode_record(
    expected_first: u64,
    bytes: &[u8],
) -> std::result::Result<DecodedRecord, FrameError> {
    if bytes.len() < RECORD_HEADER_LEN as usize {
        return Err(FrameError::TruncatedHeader);
    }
    let len = u32_at(bytes, 0);
    let crc = u32_at(bytes, 4);
    let first = u64_at(bytes, 8);
    let count = u32_at(bytes, 16);
    if len > MAX_RECORD_LEN {
        return Err(FrameError::LengthExceedsLimit(len));
    }
    let total = RECORD_HEADER_LEN as usize + len as usize;
    if bytes.len() < total {
        return Err(FrameError::TruncatedPayload);
    }
    let record = &bytes[..total];
    if record_crc(record) != crc {
        return Err(FrameError::ChecksumMismatch);
    }
    if first != expected_first {
        return Err(FrameError::OffsetMismatch {
            got: first,
            expected: expected_first,
        });
    }
    if count == 0 {
        return Err(FrameError::EmptyBatch);
    }
    let mut events = Vec::with_capacity(count as usize);
    let mut at = RECORD_HEADER_LEN as usize;
    for i in 0..u64::from(count) {
        if at + 4 > record.len() {
            return Err(FrameError::MalformedPayload);
        }
        let event_len = u32_at(record, at) as usize;
        let start = at + 4;
        let end = start + event_len;
        if end > record.len() {
            return Err(FrameError::MalformedPayload);
        }
        events.push((first + i, record[start..end].to_vec()));
        at = end;
    }
    if at != record.len() {
        return Err(FrameError::MalformedPayload);
    }
    Ok((total, events))
}

/// Creates an empty segment atomically, so a segment file either exists with a
/// valid header or does not exist.
fn create_segment(dir: &Path, base: u64, durability: Durability) -> Result<()> {
    let path = segment_path(dir, base);
    if path.exists() {
        return Err(Error::Format(format!(
            "segment {} already exists",
            path.display()
        )));
    }
    fsutil::write_atomic(
        &path,
        &encode_segment_header(base),
        durability == Durability::Fsync,
    )?;
    Ok(())
}

/// Checksum over a whole encoded record except its checksum field.
fn record_crc(record: &[u8]) -> u32 {
    let crc = crc32c::crc32c(&record[0..4]);
    crc32c::crc32c_append(crc, &record[8..])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(buf)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(buf)
}

enum Step {
    Record,
    End,
    Bad { position: u64, reason: String },
}

/// Reads records from one segment, validating each before exposing it.
struct SegmentReader {
    reader: BufReader<File>,
    file_len: u64,
    /// Byte position after the last valid record.
    pos: u64,
    /// Offset of the next event.
    next_offset: u64,
    record: Vec<u8>,
    /// `(offset, start, end)` of each event of the current record, in `record`.
    events: Vec<(u64, usize, usize)>,
}

impl SegmentReader {
    fn open(path: &Path, base: u64) -> Result<Self> {
        let file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut reader = BufReader::with_capacity(1 << 16, file);
        if file_len < SEGMENT_HEADER_LEN {
            return Err(Error::corruption(path, 0, "segment header is truncated"));
        }
        let mut header = [0u8; SEGMENT_HEADER_LEN as usize];
        reader.read_exact(&mut header)?;
        if header[0..8] != SEGMENT_MAGIC {
            return Err(Error::corruption(path, 0, "not a hivemind log segment"));
        }
        if crc32c::crc32c(&header[0..24]) != u32_at(&header, 24) {
            return Err(Error::corruption(
                path,
                0,
                "segment header checksum mismatch",
            ));
        }
        let format = u32_at(&header, 8);
        if format != SEGMENT_FORMAT {
            return Err(Error::Format(format!(
                "segment {} has format {format}; this build reads format {SEGMENT_FORMAT}",
                path.display()
            )));
        }
        if u64_at(&header, 16) != base {
            return Err(Error::corruption(
                path,
                16,
                "segment header disagrees with file name",
            ));
        }
        Ok(Self {
            reader,
            file_len,
            pos: SEGMENT_HEADER_LEN,
            next_offset: base,
            record: Vec::new(),
            events: Vec::new(),
        })
    }

    fn bad(&self, reason: impl Into<String>) -> Result<Step> {
        Ok(Step::Bad {
            position: self.pos,
            reason: reason.into(),
        })
    }

    fn step(&mut self) -> Result<Step> {
        self.events.clear();
        let remaining = self.file_len - self.pos;
        if remaining == 0 {
            return Ok(Step::End);
        }
        if remaining < RECORD_HEADER_LEN {
            return self.bad(FrameError::TruncatedHeader.to_string());
        }
        // Peek the length so we know how many bytes to pull before validating.
        self.record.resize(RECORD_HEADER_LEN as usize, 0);
        self.reader.read_exact(&mut self.record)?;
        let len = u32_at(&self.record, 0);
        if len > MAX_RECORD_LEN {
            return self.bad(FrameError::LengthExceedsLimit(len).to_string());
        }
        if remaining - RECORD_HEADER_LEN < u64::from(len) {
            return self.bad(FrameError::TruncatedPayload.to_string());
        }
        self.record
            .resize(RECORD_HEADER_LEN as usize + len as usize, 0);
        self.reader
            .read_exact(&mut self.record[RECORD_HEADER_LEN as usize..])?;
        match try_decode_record(self.next_offset, &self.record) {
            Ok((consumed, decoded)) => {
                debug_assert_eq!(consumed, self.record.len());
                // Keep event bytes borrowed from `self.record` for zero-copy scan.
                let mut at = RECORD_HEADER_LEN as usize;
                for (offset, event) in &decoded {
                    let start = at + 4;
                    let end = start + event.len();
                    debug_assert_eq!(&self.record[start..end], event.as_slice());
                    self.events.push((*offset, start, end));
                    at = end;
                }
                let first = decoded[0].0;
                let count = decoded.len() as u64;
                self.pos += consumed as u64;
                self.next_offset = first + count;
                Ok(Step::Record)
            }
            Err(err) => self.bad(err.to_string()),
        }
    }

    fn events(&self) -> impl Iterator<Item = (u64, &[u8])> {
        self.events
            .iter()
            .map(|(offset, start, end)| (*offset, &self.record[*start..*end]))
    }
}

/// Looks past a damaged record for any valid record. Finding one proves the
/// damage is not a torn final write. Header plausibility checks run before the
/// checksum, so ordinary bytes can't pass by chance.
fn find_record_after(path: &Path, position: u64, min_offset: u64) -> Result<Option<u64>> {
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(position))?;
    let mut rest = Vec::new();
    file.read_to_end(&mut rest)?;
    let header = RECORD_HEADER_LEN as usize;
    let max_events = (rest.len() / 4) as u64;
    for at in 1..rest.len().saturating_sub(header) {
        let len = u32_at(&rest, at) as usize;
        let first = u64_at(&rest, at + 8);
        let count = u32_at(&rest, at + 16);
        let plausible = len <= MAX_RECORD_LEN as usize
            && at + header + len <= rest.len()
            && count >= 1
            && (count as usize) * 4 <= len
            && first >= min_offset
            && first <= min_offset + max_events;
        if plausible && record_crc(&rest[at..at + header + len]) == u32_at(&rest, at + 4) {
            return Ok(Some(position + at as u64));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Opened = (Log, Recovery, Vec<(u64, Vec<u8>)>);

    fn open(dir: &Path) -> Opened {
        open_with(dir, DEFAULT_SEGMENT_BYTES).unwrap()
    }

    fn open_with(dir: &Path, segment_bytes: u64) -> Result<Opened> {
        let mut seen = Vec::new();
        let (log, recovery) =
            Log::open(dir, Durability::NoFsync, segment_bytes, |offset, event| {
                seen.push((offset, event.to_vec()));
                Ok(())
            })?;
        Ok((log, recovery, seen))
    }

    fn event(i: u64) -> Vec<u8> {
        format!("event-{i}-{}", "x".repeat((i % 7) as usize)).into_bytes()
    }

    fn only_segment(dir: &Path) -> PathBuf {
        segment_path(dir, 0)
    }

    #[test]
    fn appends_and_replays_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, seen) = open(tmp.path());
        assert!(seen.is_empty());
        assert_eq!(log.append(&[event(0)]).unwrap(), 0);
        assert_eq!(log.append(&[event(1), event(2)]).unwrap(), 1);
        assert_eq!(log.next_offset(), 3);
        drop(log);

        let (log, recovery, seen) = open(tmp.path());
        assert_eq!(recovery.events, 3);
        assert_eq!(recovery.records, 2);
        assert_eq!(recovery.truncated_bytes, 0);
        assert_eq!(seen, vec![(0, event(0)), (1, event(1)), (2, event(2))]);
        assert_eq!(log.next_offset(), 3);
    }

    #[test]
    fn view_reads_ranges_across_segments() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open_with(tmp.path(), 128).unwrap();
        for i in 0..20 {
            log.append(&[event(i)]).unwrap();
        }
        let view = log.view();
        assert!(view.segment_count() > 3, "small segments should roll");
        let got = view.read(5, 4).unwrap();
        assert_eq!(got, (5..9).map(|i| (i, event(i))).collect::<Vec<_>>());
        assert_eq!(view.read(18, 10).unwrap().len(), 2);
        assert!(view.read(20, 10).unwrap().is_empty());
        drop(log);

        let (log, recovery, seen) = open_with(tmp.path(), 128).unwrap();
        assert_eq!(seen.len(), 20);
        assert_eq!(recovery.segments, view.segment_count());
        assert_eq!(log.next_offset(), 20);
    }

    /// Cutting the file anywhere inside the last record must recover exactly
    /// the earlier records, and the next append must reuse the lost offsets.
    #[test]
    fn truncates_torn_tail_at_every_byte() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        log.append(&[event(0)]).unwrap();
        log.append(&[event(1), event(2)]).unwrap();
        let committed = log.active_len;
        log.append(&[event(3), event(4)]).unwrap();
        let full = log.active_len;
        drop(log);
        let pristine = fs::read(only_segment(tmp.path())).unwrap();

        for cut in committed + 1..full {
            fs::write(only_segment(tmp.path()), &pristine[..cut as usize]).unwrap();
            let (mut log, recovery, seen) = open(tmp.path());
            assert_eq!(seen.len(), 3, "cut at {cut}");
            assert_eq!(recovery.truncated_bytes, cut - committed);
            assert_eq!(
                fs::metadata(only_segment(tmp.path())).unwrap().len(),
                committed
            );
            assert_eq!(log.append(&[event(9)]).unwrap(), 3);
        }
    }

    #[test]
    fn bit_flips_in_the_last_record_are_treated_as_torn() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        log.append(&[event(0)]).unwrap();
        let committed = log.active_len;
        log.append(&[event(1)]).unwrap();
        let full = log.active_len;
        drop(log);
        let pristine = fs::read(only_segment(tmp.path())).unwrap();

        for at in committed..full {
            let mut damaged = pristine.clone();
            damaged[at as usize] ^= 0x40;
            fs::write(only_segment(tmp.path()), &damaged).unwrap();
            let (_, recovery, seen) = open(tmp.path());
            assert_eq!(seen.len(), 1, "flip at {at}");
            assert_eq!(recovery.truncated_bytes, full - committed);
        }
    }

    /// Damage followed by a valid record is corruption of acknowledged data, and
    /// the log refuses to open rather than truncating it away.
    #[test]
    fn refuses_to_drop_records_after_damage() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        log.append(&[event(0)]).unwrap();
        let second = log.active_len;
        log.append(&[event(1)]).unwrap();
        log.append(&[event(2)]).unwrap();
        drop(log);
        let path = only_segment(tmp.path());
        let mut bytes = fs::read(&path).unwrap();
        bytes[second as usize + RECORD_HEADER_LEN as usize + 2] ^= 0x01;
        fs::write(&path, &bytes).unwrap();
        let err = open_with(tmp.path(), DEFAULT_SEGMENT_BYTES).unwrap_err();
        assert!(matches!(err, Error::Corruption { .. }), "{err}");
    }

    #[test]
    fn damage_in_an_earlier_segment_is_corruption() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open_with(tmp.path(), 64).unwrap();
        for i in 0..6 {
            log.append(&[event(i)]).unwrap();
        }
        assert!(log.view().segment_count() > 1);
        drop(log);
        let path = only_segment(tmp.path());
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&path, &bytes).unwrap();
        assert!(matches!(
            open_with(tmp.path(), 64).unwrap_err(),
            Error::Corruption { .. }
        ));
    }

    #[test]
    fn rejects_foreign_files_and_bad_headers() {
        let tmp = tempfile::tempdir().unwrap();
        let (log, _, _) = open(tmp.path());
        drop(log);
        let path = only_segment(tmp.path());
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'X';
        fs::write(&path, &bytes).unwrap();
        assert!(open_with(tmp.path(), DEFAULT_SEGMENT_BYTES).is_err());

        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("notes.hlog"), b"").unwrap();
        assert!(matches!(
            open_with(other.path(), DEFAULT_SEGMENT_BYTES).unwrap_err(),
            Error::Format(_)
        ));
    }

    #[test]
    fn oversized_records_are_rejected_without_poisoning() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        let huge = vec![0u8; MAX_RECORD_LEN as usize + 1];
        assert!(matches!(log.append(&[huge]), Err(Error::RecordTooLarge(_))));
        assert_eq!(log.append(&[event(0)]).unwrap(), 0);
    }

    #[test]
    fn disk_full_is_rejected_without_poisoning() {
        use crate::fsutil::{DISK_SAFETY_MARGIN, with_free_bytes};

        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        with_free_bytes(Some(0), || {
            let err = log.append(&[event(0)]).unwrap_err();
            match err {
                Error::DiskFull { available, needed } => {
                    assert_eq!(available, 0);
                    assert!(needed > DISK_SAFETY_MARGIN, "needed={needed}");
                }
                other => panic!("expected DiskFull, got {other}"),
            }
        });
        // Next append still works: the log was not poisoned.
        assert_eq!(log.append(&[event(0)]).unwrap(), 0);
        assert_eq!(log.next_offset(), 1);
    }

    #[test]
    fn fsync_mode_works_on_this_platform() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _) = Log::open(tmp.path(), Durability::Fsync, 256, |_, _| Ok(())).unwrap();
        for i in 0..10 {
            log.append(&[event(i)]).unwrap();
        }
        assert!(log.view().segment_count() > 1);
    }

    #[test]
    fn encode_record_roundtrips_and_matches_append_bytes() {
        let events = vec![event(0), event(1)];
        let encoded = encode_record(0, &events).unwrap();
        let (consumed, decoded) = try_decode_record(0, &encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded, vec![(0, event(0)), (1, event(1))]);

        let tmp = tempfile::tempdir().unwrap();
        let (mut log, _, _) = open(tmp.path());
        log.append(&events).unwrap();
        drop(log);
        let on_disk = fs::read(only_segment(tmp.path())).unwrap();
        assert_eq!(&on_disk[SEGMENT_HEADER_LEN as usize..], encoded.as_slice());
    }

    #[test]
    fn segment_format_version_is_explicit_and_enforced() {
        assert_eq!(SEGMENT_FORMAT, 1);
        assert_eq!(&SEGMENT_MAGIC, b"HMLOG\0\0\x01");
        let tmp = tempfile::tempdir().unwrap();
        let (log, _, _) = open(tmp.path());
        drop(log);
        let path = only_segment(tmp.path());
        let mut bytes = fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&2u32.to_le_bytes());
        let crc = crc32c::crc32c(&bytes[0..24]);
        bytes[24..28].copy_from_slice(&crc.to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        let err = open_with(tmp.path(), DEFAULT_SEGMENT_BYTES).unwrap_err();
        match err {
            Error::Format(msg) => {
                assert!(msg.contains("format 2"), "{msg}");
                assert!(msg.contains("format 1"), "{msg}");
            }
            other => panic!("expected Format, got {other}"),
        }
    }

    #[test]
    fn try_decode_record_rejects_truncated_and_bad_crc() {
        let events = [event(0)];
        let mut record = encode_record(0, &events).unwrap();
        assert_eq!(
            try_decode_record(0, &record[..RECORD_HEADER_LEN as usize - 1]),
            Err(FrameError::TruncatedHeader)
        );
        assert_eq!(
            try_decode_record(0, &record[..record.len() - 1]),
            Err(FrameError::TruncatedPayload)
        );
        record[9] ^= 0xff;
        assert_eq!(
            try_decode_record(0, &record),
            Err(FrameError::ChecksumMismatch)
        );
    }
}
