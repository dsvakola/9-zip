//! The .9z container.
//!
//! A .9z archive has two layers.
//!
//! **Content.** Files that compress well are packed together in solid LZMA2
//! blocks, sorted by type, as 7-Zip does. Files that are already compressed
//! (photos, videos, other archives) or that do not shrink are stored as they
//! are, so no time is wasted on them. Every file carries a SHA-256
//! fingerprint, checked when it is extracted or tested. The file list
//! (manifest) is LZMA2-compressed JSON stored at the end.
//!
//! **Recovery.** All content bytes (the protected stream) are cut into
//! shards. Every `data_shards` shards form a stripe that gets `parity_shards`
//! Reed-Solomon recovery shards, and every shard has a CRC32 so damage is
//! found exactly. Stripes are grouped and their shards interleaved on disk,
//! so one long damaged stretch (a bad sector, a broken download) hits each
//! stripe only a little and can still be repaired. The header and the
//! checksum table are both stored twice.
//!
//! Version 3 layout (all integers little-endian):
//!
//! ```text
//! [superblock, 64 B]            copy A, rewritten when the archive is finished
//! [group 0][group 1] ... [group n-1]
//! [shard CRC table][shard CRC table]
//! [superblock, 64 B]            copy B
//! ```
//!
//! Inside a group of `g` stripes, shard `j` of stripe `s` is at position
//! `j * g + s`. The last group uses fewer stripes and smaller shards so
//! small archives carry no padding.
//!
//! Versions 1 and 2 (archives made by earlier test builds) remain readable.

use crate::engine::compress::InputFileItem;
use crate::error::AppError;
use crate::models::{
    ArchiveEntry, CompressOptions, CompressionLevel, ExtractOptions, ProgressPayload, RecoveryLevel,
};
use crate::task_manager::TaskManager;
use reed_solomon_erasure::galois_8::ReedSolomon;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::num::NonZeroU64;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;

pub const NINE_Z_MAGIC_V1: &[u8; 4] = b"9Z\x01\x00";
pub const NINE_Z_MAGIC_V2: &[u8; 4] = b"9Z\x02\x00";
pub const NINE_Z_MAGIC: &[u8; 4] = b"9Z\x03\x00";

const DATA_SHARDS: usize = 40;
const SHARD_SIZE: usize = 16 * 1024;
const GROUP_STRIPES: usize = 16;
const SUPERBLOCK_LEN: usize = 64;
const COPY_CHUNK: usize = 256 * 1024;
const SNIFF_LEN: usize = 64 * 1024;
const MAX_MANIFEST_LEN: u64 = 512 * 1024 * 1024;
/// Largest amount of file data in one solid block. Extracting a single file
/// never has to decode more than this.
const SOLID_BLOCK_LIMIT: u64 = 256 * 1024 * 1024;
const MAX_THREADS: usize = 8;

const MANIFEST_RAW: u8 = 0;
const MANIFEST_LZMA2: u8 = 1;

fn parity_shards_for(level: RecoveryLevel) -> usize {
    match level {
        RecoveryLevel::Low => 2,    // 2 / 40 = 5%
        RecoveryLevel::Normal => 4, // 4 / 40 = 10%
        RecoveryLevel::High => 8,   // 8 / 40 = 20%
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NineZEntryMeta {
    pub path: String,
    pub uncompressed_size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
    pub modified: Option<u64>,
    pub sha256: String,
    /// "solid" (inside a solid block), "store", or "zstd" (version 2).
    pub method: String,
    /// For "solid": position inside the block's unpacked data. Otherwise:
    /// position in the protected stream.
    #[serde(default)]
    pub offset: u64,
    /// Index into `NineZManifest::blocks` for "solid" entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NineZBlock {
    /// Position in the protected stream.
    pub offset: u64,
    pub packed_size: u64,
    pub unpacked_size: u64,
    /// "lzma2"
    pub method: String,
    pub dict_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NineZManifest {
    pub version: String,
    pub creator: String,
    pub organization: String,
    pub store_url: String,
    pub data_shards: usize,
    pub parity_shards: usize,
    pub shard_size: usize,
    #[serde(default)]
    pub recovery_percent: u32,
    #[serde(default)]
    pub blocks: Vec<NineZBlock>,
    pub entries: Vec<NineZEntryMeta>,
}

/// What the self-healing layer found while reading an archive.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NineZHealth {
    pub format_version: u32,
    pub recovery_percent: u32,
    pub blocks_checked: usize,
    pub blocks_repaired: usize,
    pub header_repaired: bool,
    pub index_repaired: bool,
    /// Both copies of the block checksum table were unreadable, so damage
    /// could only be caught by the per-file SHA-256 seal.
    pub index_lost: bool,
    pub files_verified: usize,
}

impl NineZHealth {
    fn repair_notice(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.blocks_repaired > 0 {
            parts.push(format!("{} damaged block(s) rebuilt", self.blocks_repaired));
        }
        if self.header_repaired {
            parts.push("damaged header restored from its backup copy".to_string());
        }
        if self.index_repaired {
            parts.push("damaged block index restored from its backup copy".to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(format!("9-zip self-healing: {}.", parts.join(", ")))
        }
    }
}

// ---------------------------------------------------------------------------
// Layout (superblock)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Layout {
    version: u32,
    data_shards: usize,
    parity_shards: usize,
    shard_size: usize,
    group_stripes: usize,
    group_count: u64,
    last_group_stripes: usize,
    last_shard_size: usize,
    payload_len: u64,
    manifest_len: u64,
    crc_table_offset: u64,
    manifest_codec: u8,
    manifest_dict: u32,
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

impl Layout {
    fn encode(&self) -> [u8; SUPERBLOCK_LEN] {
        let mut b = [0u8; SUPERBLOCK_LEN];
        b[0..4].copy_from_slice(NINE_Z_MAGIC);
        b[4] = self.data_shards as u8;
        b[5] = self.parity_shards as u8;
        b[6] = self.manifest_codec;
        b[8..12].copy_from_slice(&(self.shard_size as u32).to_le_bytes());
        b[12..16].copy_from_slice(&(self.last_shard_size as u32).to_le_bytes());
        b[16..20].copy_from_slice(&(self.group_stripes as u32).to_le_bytes());
        b[20..24].copy_from_slice(&(self.last_group_stripes as u32).to_le_bytes());
        b[24..32].copy_from_slice(&self.group_count.to_le_bytes());
        b[32..40].copy_from_slice(&self.payload_len.to_le_bytes());
        b[40..48].copy_from_slice(&self.manifest_len.to_le_bytes());
        b[48..56].copy_from_slice(&self.crc_table_offset.to_le_bytes());
        b[56..60].copy_from_slice(&self.manifest_dict.to_le_bytes());
        let crc = crc32fast::hash(&b[..60]);
        b[60..64].copy_from_slice(&crc.to_le_bytes());
        b
    }

    fn decode(b: &[u8; SUPERBLOCK_LEN]) -> Option<Self> {
        if crc32fast::hash(&b[..60]) != le_u32(b, 60) {
            return None;
        }
        let layout = if &b[0..4] == NINE_Z_MAGIC {
            Self {
                version: 3,
                data_shards: b[4] as usize,
                parity_shards: b[5] as usize,
                manifest_codec: b[6],
                shard_size: le_u32(b, 8) as usize,
                last_shard_size: le_u32(b, 12) as usize,
                group_stripes: le_u32(b, 16) as usize,
                last_group_stripes: le_u32(b, 20) as usize,
                group_count: le_u64(b, 24),
                payload_len: le_u64(b, 32),
                manifest_len: le_u64(b, 40),
                crc_table_offset: le_u64(b, 48),
                manifest_dict: le_u32(b, 56),
            }
        } else if &b[0..4] == NINE_Z_MAGIC_V2 {
            // Version 2: one stripe per group, raw JSON manifest.
            Self {
                version: 2,
                data_shards: b[4] as usize,
                parity_shards: b[5] as usize,
                manifest_codec: MANIFEST_RAW,
                shard_size: le_u32(b, 8) as usize,
                last_shard_size: le_u32(b, 12) as usize,
                group_stripes: 1,
                last_group_stripes: 1,
                group_count: le_u64(b, 16),
                payload_len: le_u64(b, 24),
                manifest_len: le_u64(b, 32),
                crc_table_offset: le_u64(b, 40),
                manifest_dict: 0,
            }
        } else {
            return None;
        };
        let sane = layout.data_shards > 0
            && layout.parity_shards > 0
            && layout.data_shards + layout.parity_shards <= 255
            && layout.shard_size > 0
            && layout.last_shard_size > 0
            && layout.last_shard_size <= layout.shard_size
            && layout.group_stripes > 0
            && layout.last_group_stripes > 0
            && layout.last_group_stripes <= layout.group_stripes
            && layout.group_count > 0
            && layout
                .payload_len
                .checked_add(layout.manifest_len)
                .is_some();
        sane.then_some(layout)
    }

    fn per_stripe(&self) -> usize {
        self.data_shards + self.parity_shards
    }

    fn protected_len(&self) -> u64 {
        self.payload_len + self.manifest_len
    }

    /// Stripes and shard size of a group.
    fn group_geometry(&self, group: u64) -> (usize, usize) {
        if group + 1 == self.group_count {
            (self.last_group_stripes, self.last_shard_size)
        } else {
            (self.group_stripes, self.shard_size)
        }
    }

    fn group_capacity(&self) -> u64 {
        (self.group_stripes * self.data_shards * self.shard_size) as u64
    }

    fn group_offset(&self, group: u64) -> u64 {
        SUPERBLOCK_LEN as u64
            + group * (self.group_stripes * self.per_stripe() * self.shard_size) as u64
    }

    fn shard_total(&self) -> Option<u64> {
        let full =
            (self.group_count - 1).checked_mul((self.group_stripes * self.per_stripe()) as u64)?;
        full.checked_add((self.last_group_stripes * self.per_stripe()) as u64)
    }

    /// Index of a shard in the CRC table (logical order).
    fn crc_index(&self, group: u64, stripe: usize, shard: usize) -> usize {
        (group as usize * self.group_stripes + stripe) * self.per_stripe() + shard
    }
}

fn new_rs(data: usize, parity: usize) -> Result<ReedSolomon, AppError> {
    ReedSolomon::new(data, parity)
        .map_err(|e| AppError::new(format!("Reed-Solomon initialization error: {:?}", e)))
}

// ---------------------------------------------------------------------------
// Recovery layer: writer
// ---------------------------------------------------------------------------

/// Streams the protected bytes to disk one group at a time, adding parity
/// and interleaving the shards.
struct ProtectedWriter {
    out: BufWriter<File>,
    rs: ReedSolomon,
    parity_shards: usize,
    buf: Vec<u8>,
    crcs: Vec<u32>,
    groups: u64,
    logical_len: u64,
}

impl ProtectedWriter {
    fn create(path: &Path, parity_shards: usize) -> Result<Self, AppError> {
        let mut out = BufWriter::with_capacity(1024 * 1024, File::create(path)?);
        // Placeholder; finish() writes the real superblock here.
        out.write_all(&[0u8; SUPERBLOCK_LEN])?;
        Ok(Self {
            out,
            rs: new_rs(DATA_SHARDS, parity_shards)?,
            parity_shards,
            buf: Vec::with_capacity(GROUP_STRIPES * DATA_SHARDS * SHARD_SIZE),
            crcs: Vec::new(),
            groups: 0,
            logical_len: 0,
        })
    }

    fn position(&self) -> u64 {
        self.logical_len
    }

    fn flush_group(&mut self, stripes: usize, shard_len: usize) -> io::Result<()> {
        let per_stripe = DATA_SHARDS + self.parity_shards;
        let mut all: Vec<Vec<Vec<u8>>> = Vec::with_capacity(stripes);
        for s in 0..stripes {
            let mut shards: Vec<Vec<u8>> = (0..per_stripe)
                .map(|j| {
                    let mut shard = vec![0u8; shard_len];
                    let start = (s * DATA_SHARDS + j) * shard_len;
                    if j < DATA_SHARDS && start < self.buf.len() {
                        let end = (start + shard_len).min(self.buf.len());
                        shard[..end - start].copy_from_slice(&self.buf[start..end]);
                    }
                    shard
                })
                .collect();
            self.rs.encode(&mut shards).map_err(|e| {
                io::Error::other(format!("Reed-Solomon parity encode error: {:?}", e))
            })?;
            self.crcs
                .extend(shards.iter().map(|shard| crc32fast::hash(shard)));
            all.push(shards);
        }
        // Interleave: shard j of every stripe, then shard j + 1, ...
        for j in 0..per_stripe {
            for stripe in &all {
                self.out.write_all(&stripe[j])?;
            }
        }
        self.groups += 1;
        self.buf.clear();
        Ok(())
    }

    fn finish(
        mut self,
        payload_len: u64,
        manifest_codec: u8,
        manifest_dict: u32,
    ) -> Result<(), AppError> {
        let (last_group_stripes, last_shard_size) = if self.buf.is_empty() && self.groups > 0 {
            (GROUP_STRIPES, SHARD_SIZE)
        } else {
            let len = self.buf.len().max(1);
            let stripes = len.div_ceil(DATA_SHARDS * SHARD_SIZE);
            let shard_len = len.div_ceil(stripes * DATA_SHARDS);
            self.flush_group(stripes, shard_len)?;
            (stripes, shard_len)
        };

        let mut table = Vec::with_capacity(self.crcs.len() * 4 + 4);
        for crc in &self.crcs {
            table.extend_from_slice(&crc.to_le_bytes());
        }
        let table_crc = crc32fast::hash(&table);
        table.extend_from_slice(&table_crc.to_le_bytes());

        let crc_table_offset = self.out.stream_position()?;
        self.out.write_all(&table)?;
        self.out.write_all(&table)?;

        let superblock = Layout {
            version: 3,
            data_shards: DATA_SHARDS,
            parity_shards: self.parity_shards,
            shard_size: SHARD_SIZE,
            group_stripes: GROUP_STRIPES,
            group_count: self.groups,
            last_group_stripes,
            last_shard_size,
            payload_len,
            manifest_len: self.logical_len - payload_len,
            crc_table_offset,
            manifest_codec,
            manifest_dict,
        }
        .encode();
        self.out.write_all(&superblock)?;
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&superblock)?;
        self.out.flush()?;
        self.out.get_ref().sync_all()?;
        Ok(())
    }
}

impl Write for ProtectedWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let capacity = GROUP_STRIPES * DATA_SHARDS * SHARD_SIZE;
        let n = (capacity - self.buf.len()).min(data.len());
        self.buf.extend_from_slice(&data[..n]);
        self.logical_len += n as u64;
        if self.buf.len() == capacity {
            self.flush_group(GROUP_STRIPES, SHARD_SIZE)?;
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Full groups are written as they fill; finish() writes the rest.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Recovery layer: reader
// ---------------------------------------------------------------------------

fn read_at(file: &mut File, offset: u64, buf: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buf)
}

/// Reads as much as is there; a truncated archive leaves the rest zeroed.
fn read_at_partial(file: &mut File, offset: u64, buf: &mut [u8]) -> usize {
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return 0;
    }
    let mut filled = 0;
    while filled < buf.len() {
        match file.read(&mut buf[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled
}

/// Reads the protected stream, checking every shard and rebuilding damaged
/// ones from parity.
struct ProtectedReader {
    file: File,
    layout: Layout,
    crcs: Option<Vec<u32>>,
    rs: ReedSolomon,
    cached: Option<(u64, Vec<u8>)>,
    counted: BTreeSet<u64>,
    pos: u64,
    health: NineZHealth,
    fatal: Option<String>,
}

impl ProtectedReader {
    fn open(path: &Path) -> Result<Self, AppError> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        let mut health = NineZHealth::default();

        let mut block = [0u8; SUPERBLOCK_LEN];
        let mut layout = read_at(&mut file, 0, &mut block)
            .ok()
            .and_then(|_| Layout::decode(&block));
        if layout.is_none() && file_len >= SUPERBLOCK_LEN as u64 {
            layout = read_at(&mut file, file_len - SUPERBLOCK_LEN as u64, &mut block)
                .ok()
                .and_then(|_| Layout::decode(&block));
            health.header_repaired = layout.is_some();
        }
        let layout = layout.ok_or_else(|| {
            AppError::new(
                "This .9z archive is incomplete, or both copies of its header are damaged.",
            )
        })?;
        health.format_version = layout.version;
        health.recovery_percent = (layout.parity_shards * 100 / layout.data_shards) as u32;

        let shard_count = layout
            .shard_total()
            .filter(|n| n.saturating_mul(4) < file_len)
            .ok_or_else(|| AppError::new("This .9z archive has an invalid header."))?;
        let table_len = shard_count * 4 + 4;

        let mut crcs = None;
        for copy in 0..2u64 {
            let offset = layout.crc_table_offset + copy * table_len;
            if offset + table_len > file_len {
                continue;
            }
            let mut table = vec![0u8; table_len as usize];
            if read_at(&mut file, offset, &mut table).is_err() {
                continue;
            }
            let body = &table[..table.len() - 4];
            if crc32fast::hash(body) == le_u32(&table, table.len() - 4) {
                crcs = Some(
                    body.as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| u32::from_le_bytes(*c))
                        .collect(),
                );
                health.index_repaired = copy == 1;
                break;
            }
        }
        health.index_lost = crcs.is_none();

        Ok(Self {
            file,
            rs: new_rs(layout.data_shards, layout.parity_shards)?,
            layout,
            crcs,
            cached: None,
            counted: BTreeSet::new(),
            pos: 0,
            health,
            fatal: None,
        })
    }

    fn load_group(&mut self, group: u64) -> Result<Vec<u8>, AppError> {
        let layout = self.layout;
        let (stripes, shard_len) = layout.group_geometry(group);
        let per_stripe = layout.per_stripe();

        let mut raw = vec![0u8; stripes * per_stripe * shard_len];
        let available = read_at_partial(&mut self.file, layout.group_offset(group), &mut raw);

        let first_visit = self.counted.insert(group);
        if first_visit {
            self.health.blocks_checked += stripes * per_stripe;
        }

        let mut data = Vec::with_capacity(stripes * layout.data_shards * shard_len);
        for s in 0..stripes {
            let mut shards: Vec<Option<Vec<u8>>> = Vec::with_capacity(per_stripe);
            let mut damaged = 0usize;
            for j in 0..per_stripe {
                let at = (j * stripes + s) * shard_len;
                let shard = raw[at..at + shard_len].to_vec();
                let intact = at + shard_len <= available
                    && self.crcs.as_ref().is_none_or(|table| {
                        table
                            .get(layout.crc_index(group, s, j))
                            .is_some_and(|&crc| crc32fast::hash(&shard) == crc)
                    });
                if intact {
                    shards.push(Some(shard));
                } else {
                    damaged += 1;
                    shards.push(None);
                }
            }
            if damaged > 0 {
                if damaged > layout.parity_shards {
                    return Err(AppError::new(format!(
                        "Part of this .9z archive is damaged too badly to repair \
                         ({} of {} blocks in one group are bad; at most {} can be rebuilt). \
                         Files stored in this part cannot be recovered.",
                        damaged, per_stripe, layout.parity_shards
                    )));
                }
                self.rs
                    .reconstruct_data(&mut shards)
                    .map_err(|e| AppError::new(format!("Reed-Solomon repair error: {:?}", e)))?;
                if first_visit {
                    self.health.blocks_repaired += damaged;
                }
            }
            for shard in shards.into_iter().take(layout.data_shards) {
                let shard = shard.ok_or_else(|| AppError::new("Reed-Solomon repair incomplete"))?;
                data.extend_from_slice(&shard);
            }
        }

        let start = group * layout.group_capacity();
        let len = (layout.protected_len() - start).min(data.len() as u64);
        data.truncate(len as usize);
        Ok(data)
    }

    /// Turns an I/O error that surfaced through a decoder back into the
    /// self-healing layer's own message when there is one.
    fn explain(&mut self, err: AppError) -> AppError {
        match self.fatal.take() {
            Some(message) => AppError::new(message),
            None => err,
        }
    }
}

impl Read for ProtectedReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.pos >= self.layout.protected_len() {
            return Ok(0);
        }
        let capacity = self.layout.group_capacity();
        let group = (self.pos / capacity).min(self.layout.group_count - 1);
        if self.cached.as_ref().map(|(g, _)| *g) != Some(group) {
            match self.load_group(group) {
                Ok(data) => self.cached = Some((group, data)),
                Err(e) => {
                    self.fatal = Some(e.message.clone());
                    return Err(io::Error::new(io::ErrorKind::InvalidData, e.message));
                }
            }
        }
        let data = match &self.cached {
            Some((_, data)) => data,
            None => return Ok(0),
        };
        let offset = (self.pos - group * capacity) as usize;
        if offset >= data.len() {
            return Ok(0);
        }
        let n = out.len().min(data.len() - offset);
        out[..n].copy_from_slice(&data[offset..offset + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

fn read_manifest_protected(reader: &mut ProtectedReader) -> Result<NineZManifest, AppError> {
    let layout = reader.layout;
    if layout.manifest_len == 0 || layout.manifest_len > MAX_MANIFEST_LEN {
        return Err(AppError::new("This .9z archive has an invalid file list."));
    }
    reader.pos = layout.payload_len;
    let mut stored = vec![0u8; layout.manifest_len as usize];
    if let Err(e) = reader.read_exact(&mut stored) {
        let err = AppError::from(e);
        return Err(reader.explain(err));
    }
    let json = match layout.manifest_codec {
        MANIFEST_RAW => stored,
        MANIFEST_LZMA2 => {
            let mut json = Vec::new();
            lzma_rust2::Lzma2Reader::new(&stored[..], layout.manifest_dict, None)
                .take(MAX_MANIFEST_LEN)
                .read_to_end(&mut json)
                .map_err(|e| AppError::new(format!("The .9z file list is unreadable: {}", e)))?;
            json
        }
        other => {
            return Err(AppError::new(format!(
                "This .9z archive uses an unknown file list format ({}). \
                 It may need a newer version of 9-zip.",
                other
            )));
        }
    };
    Ok(serde_json::from_slice(&json)?)
}

fn is_version_1(path: &Path) -> Result<bool, AppError> {
    let mut magic = [0u8; 4];
    let mut file = File::open(path)?;
    Ok(file.read_exact(&mut magic).is_ok() && &magic == NINE_Z_MAGIC_V1)
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

struct Meter<'a> {
    progress: &'a Channel<ProgressPayload>,
    task_id: String,
    phase: &'static str,
    total_files: usize,
    total_bytes: u64,
    files_done: usize,
    bytes_done: u64,
    start: Instant,
    last: Option<Instant>,
}

impl<'a> Meter<'a> {
    fn new(
        progress: &'a Channel<ProgressPayload>,
        task_id: &str,
        phase: &'static str,
        total_files: usize,
        total_bytes: u64,
    ) -> Self {
        Self {
            progress,
            task_id: task_id.to_string(),
            phase,
            total_files,
            total_bytes,
            files_done: 0,
            bytes_done: 0,
            start: Instant::now(),
            last: None,
        }
    }

    fn add_bytes(&mut self, n: u64, name: &str) {
        self.bytes_done += n;
        self.emit(name, false);
    }

    fn file_done(&mut self, name: &str) {
        self.files_done += 1;
        let last = self.files_done == self.total_files;
        self.emit(name, last);
    }

    fn emit(&mut self, name: &str, force: bool) {
        if !force
            && self
                .last
                .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        self.last = Some(Instant::now());
        let elapsed = self.start.elapsed().as_secs_f32().max(0.001);
        let percent = if self.total_bytes > 0 {
            self.bytes_done as f32 / self.total_bytes as f32 * 100.0
        } else if self.total_files > 0 {
            self.files_done as f32 / self.total_files as f32 * 100.0
        } else {
            100.0
        };
        let _ = self.progress.send(ProgressPayload {
            task_id: self.task_id.clone(),
            phase: self.phase.to_string(),
            current_file: name.to_string(),
            files_processed: self.files_done,
            total_files: self.total_files,
            bytes_processed: self.bytes_done,
            total_bytes: self.total_bytes,
            percent: percent.min(100.0),
            speed_mb_per_sec: (self.bytes_done as f32 / (1024.0 * 1024.0)) / elapsed,
        });
    }

    fn notice(&self, phase: &str, message: String) {
        let _ = self.progress.send(ProgressPayload {
            task_id: self.task_id.clone(),
            phase: phase.to_string(),
            current_file: message,
            files_processed: self.files_done,
            total_files: self.total_files,
            bytes_processed: self.bytes_done,
            total_bytes: self.total_bytes,
            percent: 100.0,
            speed_mb_per_sec: 0.0,
        });
    }
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

fn extension_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn is_already_compressed(path_str: &str) -> bool {
    matches!(
        extension_of(path_str).as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "webp"
            | "gif"
            | "heic"
            | "avif"
            | "mp4"
            | "mkv"
            | "avi"
            | "mov"
            | "webm"
            | "mp3"
            | "aac"
            | "m4a"
            | "ogg"
            | "opus"
            | "flac"
            | "zip"
            | "7z"
            | "rar"
            | "gz"
            | "tgz"
            | "bz2"
            | "xz"
            | "zst"
            | "lz4"
            | "br"
            | "9z"
            | "cab"
            | "pdf"
            | "docx"
            | "xlsx"
            | "pptx"
            | "odt"
            | "epub"
            | "jar"
            | "apk"
            | "msix"
            | "appx"
    )
}

/// Smart-adaptive choice: known compressed formats are stored as they are,
/// and so is anything whose first 64 KB does not shrink by at least 3%.
fn worth_compressing(name: &str, sample: &[u8]) -> bool {
    if sample.is_empty() || is_already_compressed(name) {
        return false;
    }
    match zstd::bulk::compress(sample, 1) {
        Ok(packed) => packed.len() * 100 < sample.len() * 97,
        Err(_) => true,
    }
}

fn read_full(input: &mut (impl Read + ?Sized), buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match input.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn modified_secs(path: &Path) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

fn cancelled() -> AppError {
    AppError::new("Compression cancelled by user")
}

/// Copies a whole file into `sink`, hashing it. Returns (bytes, sha256).
fn pack_file(
    item: &InputFileItem,
    sink: &mut dyn Write,
    chunk: &mut [u8],
    cancel_flag: &Arc<AtomicBool>,
    meter: &mut Meter,
) -> Result<(u64, String), AppError> {
    let mut input = File::open(&item.disk_path)
        .map_err(|e| AppError::new(format!("Cannot read {}: {}", item.disk_path.display(), e)))?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    loop {
        if TaskManager::is_cancelled(cancel_flag) {
            return Err(cancelled());
        }
        let n = read_full(&mut input, chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
        sink.write_all(&chunk[..n])?;
        total += n as u64;
        meter.add_bytes(n as u64, &item.archive_path);
    }
    Ok((total, format!("{:x}", hasher.finalize())))
}

fn lzma2_options(level: CompressionLevel) -> lzma_rust2::Lzma2Options {
    lzma_rust2::Lzma2Options::with_preset(level.as_u32())
}

pub fn compress_nine_z(
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let result = write_nine_z(items, total_bytes, options, cancel_flag, progress);
    if result.is_err() {
        let _ = fs::remove_file(&options.output_path);
    }
    if TaskManager::is_cancelled(cancel_flag) {
        return Err(cancelled());
    }
    result
}

fn write_nine_z(
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let parity_shards = parity_shards_for(options.recovery);
    let mut writer = ProtectedWriter::create(Path::new(&options.output_path), parity_shards)?;
    let mut meter = Meter::new(
        progress,
        &options.task_id,
        "compressing",
        items.len(),
        total_bytes,
    );
    let mut chunk = vec![0u8; COPY_CHUNK];
    let compress = options.level != CompressionLevel::Store;

    // 1. Sort the files: smart-adaptive sniffing decides between the solid
    //    stream and plain storage.
    let mut entries: Vec<NineZEntryMeta> = Vec::with_capacity(items.len());
    let mut solid: Vec<usize> = Vec::new();
    let mut stored: Vec<usize> = Vec::new();
    for item in items {
        if TaskManager::is_cancelled(cancel_flag) {
            return Err(cancelled());
        }
        let index = entries.len();
        entries.push(NineZEntryMeta {
            path: item.archive_path.clone(),
            uncompressed_size: 0,
            compressed_size: 0,
            is_dir: item.is_dir,
            modified: modified_secs(&item.disk_path),
            sha256: String::new(),
            method: "store".to_string(),
            offset: 0,
            block: None,
        });
        if item.is_dir {
            continue;
        }
        let goes_solid = compress && {
            let mut sample = vec![0u8; SNIFF_LEN.min(item.size as usize)];
            let n = File::open(&item.disk_path)
                .and_then(|mut f| read_full(&mut f, &mut sample))
                .map_err(|e| {
                    AppError::new(format!("Cannot read {}: {}", item.disk_path.display(), e))
                })?;
            worth_compressing(&item.archive_path, &sample[..n])
        };
        if goes_solid {
            solid.push(index);
        } else {
            stored.push(index);
        }
    }
    // Like 7-Zip: similar files side by side compress better.
    solid.sort_by(|&a, &b| {
        let (pa, pb) = (&items[a].archive_path, &items[b].archive_path);
        extension_of(pa)
            .cmp(&extension_of(pb))
            .then_with(|| pa.cmp(pb))
    });

    // 2. Stored files go straight into the protected stream.
    for &i in &stored {
        let offset = writer.position();
        let (size, sha) = pack_file(&items[i], &mut writer, &mut chunk, cancel_flag, &mut meter)?;
        let entry = &mut entries[i];
        entry.offset = offset;
        entry.uncompressed_size = size;
        entry.compressed_size = size;
        entry.sha256 = sha;
        meter.file_done(&items[i].archive_path);
    }

    // 3. Compressible files go into solid LZMA2 blocks.
    let threads = options.threads.clamp(1, MAX_THREADS) as u32;
    let mut blocks = Vec::new();
    let mut rest = &solid[..];
    while !rest.is_empty() {
        // Take files until the block is full (a big file gets a block alone).
        let mut take = 0;
        let mut planned = 0u64;
        while take < rest.len()
            && (take == 0 || planned + items[rest[take]].size <= SOLID_BLOCK_LIMIT)
        {
            planned += items[rest[take]].size;
            take += 1;
        }
        let (members, remaining) = rest.split_at(take);
        rest = remaining;

        let block_index = blocks.len();
        let block_offset = writer.position();
        let lzma = lzma2_options(options.level);
        let dict_size = lzma.lzma_options.dict_size;
        let mut unpacked = 0u64;
        let mut packed_entries = Vec::with_capacity(members.len());

        let encoder_err = |e: io::Error| AppError::new(format!("LZMA2 compression error: {}", e));
        if threads > 1 {
            let mut lzma = lzma;
            // Independent chunks of 4x the dictionary, as 7-Zip does.
            lzma.set_chunk_size(NonZeroU64::new(u64::from(dict_size) * 4));
            let mut encoder =
                lzma_rust2::Lzma2WriterMt::new(&mut writer, lzma, threads).map_err(encoder_err)?;
            encoder
                .set_cancellation(Arc::clone(cancel_flag))
                .map_err(encoder_err)?;
            for &i in members {
                let (size, sha) =
                    pack_file(&items[i], &mut encoder, &mut chunk, cancel_flag, &mut meter)?;
                packed_entries.push((i, unpacked, size, sha));
                unpacked += size;
                meter.file_done(&items[i].archive_path);
            }
            encoder.finish().map_err(encoder_err)?;
        } else {
            let mut encoder = lzma_rust2::Lzma2Writer::new(&mut writer, lzma);
            for &i in members {
                let (size, sha) =
                    pack_file(&items[i], &mut encoder, &mut chunk, cancel_flag, &mut meter)?;
                packed_entries.push((i, unpacked, size, sha));
                unpacked += size;
                meter.file_done(&items[i].archive_path);
            }
            encoder.finish().map_err(encoder_err)?;
        }

        let packed_size = writer.position() - block_offset;
        for (i, offset, size, sha) in packed_entries {
            let entry = &mut entries[i];
            entry.method = "solid".to_string();
            entry.block = Some(block_index);
            entry.offset = offset;
            entry.uncompressed_size = size;
            // Each file's share of the packed block, for display only.
            entry.compressed_size = if unpacked > 0 {
                ((size as u128 * packed_size as u128) / unpacked as u128) as u64
            } else {
                0
            };
            entry.sha256 = sha;
        }
        blocks.push(NineZBlock {
            offset: block_offset,
            packed_size,
            unpacked_size: unpacked,
            method: "lzma2".to_string(),
            dict_size,
        });
    }
    for (i, item) in items.iter().enumerate() {
        if item.is_dir {
            meter.file_done(&entries[i].path);
        }
    }

    // 4. The file list, LZMA2-compressed, closes the protected stream.
    let payload_len = writer.position();
    let manifest = NineZManifest {
        version: "3.0".to_string(),
        creator: "9-zip".to_string(),
        organization: "Vidyasagar Academy".to_string(),
        store_url: "https://vsa.edu.in/software/".to_string(),
        data_shards: DATA_SHARDS,
        parity_shards,
        shard_size: SHARD_SIZE,
        recovery_percent: (parity_shards * 100 / DATA_SHARDS) as u32,
        blocks,
        entries,
    };
    let json = serde_json::to_vec(&manifest)?;
    let manifest_options = lzma_rust2::Lzma2Options::with_preset(6);
    let manifest_dict = manifest_options.lzma_options.dict_size;
    let mut packed = lzma_rust2::Lzma2Writer::new(Vec::new(), manifest_options);
    packed.write_all(&json)?;
    let packed = packed
        .finish()
        .map_err(|e| AppError::new(format!("LZMA2 compression error: {}", e)))?;
    writer.write_all(&packed)?;
    writer.finish(payload_len, MANIFEST_LZMA2, manifest_dict)
}

// ---------------------------------------------------------------------------
// Listing, extraction and testing
// ---------------------------------------------------------------------------

fn to_archive_entries(manifest: NineZManifest) -> Vec<ArchiveEntry> {
    manifest
        .entries
        .into_iter()
        .map(|entry| ArchiveEntry {
            name: Path::new(&entry.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| entry.path.clone()),
            crc32: (entry.sha256.len() >= 8).then(|| entry.sha256[..8].to_uppercase()),
            method: (!entry.is_dir).then(|| match entry.method.as_str() {
                "solid" => "LZMA2 (solid)".to_string(),
                "store" => "Stored".to_string(),
                "zstd" => "Zstandard".to_string(),
                other => other.to_string(),
            }),
            packed_known: entry.method != "solid",
            sha256: (!entry.sha256.is_empty()).then(|| entry.sha256.clone()),
            encrypted: false,
            path: entry.path,
            size: entry.uncompressed_size,
            compressed_size: entry.compressed_size,
            is_dir: entry.is_dir,
            modified: entry.modified,
        })
        .collect()
}

pub fn list_nine_z(archive_path: &Path) -> Result<Vec<ArchiveEntry>, AppError> {
    Ok(to_archive_entries(read_manifest_nine_z(archive_path)?))
}

pub fn read_manifest_nine_z(archive_path: &Path) -> Result<NineZManifest, AppError> {
    if is_version_1(archive_path)? {
        return v1::read_manifest(archive_path);
    }
    let mut reader = ProtectedReader::open(archive_path)?;
    read_manifest_protected(&mut reader)
}

/// The .9z section of the Properties window.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NineZDetails {
    pub format_version: u32,
    pub recovery_percent: u32,
    /// Bytes of Reed-Solomon recovery data in the file.
    pub recovery_size: u64,
    pub data_blocks: u64,
    pub recovery_blocks: u64,
    pub block_size: u64,
    pub solid_blocks: usize,
    pub solid_files: usize,
    pub stored_files: usize,
    /// Files carrying a SHA-256 fingerprint (the VSA Integrity Seal).
    pub sealed_files: usize,
    /// Header copies, block index copies and the packed file list.
    pub headers_size: u64,
    pub dictionary: Option<u32>,
    pub created_by: String,
}

/// Reads the facts shown in Properties without unpacking any file.
pub fn nine_z_details(archive_path: &Path) -> Result<NineZDetails, AppError> {
    if is_version_1(archive_path)? {
        let manifest = v1::read_manifest(archive_path)?;
        return Ok(NineZDetails {
            format_version: 1,
            sealed_files: manifest
                .entries
                .iter()
                .filter(|e| !e.sha256.is_empty())
                .count(),
            created_by: manifest.creator,
            ..Default::default()
        });
    }
    let mut reader = ProtectedReader::open(archive_path)?;
    let manifest = read_manifest_protected(&mut reader)?;
    let layout = reader.layout;
    let full_groups = layout.group_count - 1;
    let parity_per_full = (layout.group_stripes * layout.parity_shards * layout.shard_size) as u64;
    let parity_last =
        (layout.last_group_stripes * layout.parity_shards * layout.last_shard_size) as u64;
    let shards = layout.shard_total().unwrap_or(0);
    let recovery_blocks = (full_groups * layout.group_stripes as u64
        + layout.last_group_stripes as u64)
        * layout.parity_shards as u64;
    let files: Vec<&NineZEntryMeta> = manifest.entries.iter().filter(|e| !e.is_dir).collect();
    Ok(NineZDetails {
        format_version: layout.version,
        recovery_percent: (layout.parity_shards * 100 / layout.data_shards) as u32,
        recovery_size: full_groups * parity_per_full + parity_last,
        data_blocks: shards - recovery_blocks,
        recovery_blocks,
        block_size: layout.shard_size as u64,
        solid_blocks: manifest.blocks.len(),
        solid_files: files.iter().filter(|e| e.method == "solid").count(),
        stored_files: files.iter().filter(|e| e.method != "solid").count(),
        sealed_files: files.iter().filter(|e| !e.sha256.is_empty()).count(),
        headers_size: 2 * SUPERBLOCK_LEN as u64 + 2 * (shards * 4 + 4) + layout.manifest_len,
        dictionary: manifest.blocks.first().map(|b| b.dict_size),
        created_by: manifest.creator.clone(),
    })
}

/// Joins an archive path onto the destination, refusing anything that could
/// escape it (absolute paths, drive letters, `..`).
fn safe_join(dest_dir: &Path, name: &str) -> Result<PathBuf, AppError> {
    let cleaned = name.replace('\\', "/");
    let mut out = dest_dir.to_path_buf();
    let mut depth = 0;
    for part in Path::new(&cleaned).components() {
        match part {
            Component::Normal(p) => {
                out.push(p);
                depth += 1;
            }
            Component::CurDir => {}
            _ => {
                return Err(AppError::new(format!(
                    "Security warning: blocked unsafe path in archive: {}",
                    name
                )));
            }
        }
    }
    if depth == 0 {
        return Err(AppError::new(
            "The archive contains an entry with an empty name.",
        ));
    }
    Ok(out)
}

fn part_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".9z-part");
    target.with_file_name(name)
}

/// Where each file's bytes go.
enum Destination<'a> {
    /// Write into a folder; existing files are replaced only with `overwrite`.
    Folder { dir: &'a Path, overwrite: bool },
    /// Only check (the Test command).
    Check,
}

struct Unpacker<'a, 'b> {
    destination: Destination<'b>,
    /// Which entries to write; the others are only read past.
    selection: Option<&'b ExtractOptions>,
    cancel_flag: &'b Arc<AtomicBool>,
    meter: Meter<'a>,
    chunk: Vec<u8>,
    files_verified: usize,
    /// Files that could not be restored; the others still come out.
    damaged: Vec<String>,
}

impl Unpacker<'_, '_> {
    fn wants(&self, entry: &NineZEntryMeta) -> bool {
        self.selection
            .is_none_or(|options| options.wants(&entry.path))
    }

    /// Reads exactly one entry from `source`, checks its fingerprint and
    /// writes it out. The file goes to a temporary name first, so a failed
    /// check never leaves a half-written or wrong file behind.
    fn unpack(&mut self, entry: &NineZEntryMeta, source: &mut dyn Read) -> Result<(), AppError> {
        let target = match self.destination {
            Destination::Folder { dir, overwrite } => {
                let target = safe_join(dir, &entry.path)?;
                if target.exists() && !overwrite {
                    // Still read past it: solid blocks are decoded in order.
                    self.copy(entry, source, &mut io::sink())?;
                    return Ok(());
                }
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                Some(target)
            }
            Destination::Check => None,
        };

        match target {
            None => {
                self.copy(entry, source, &mut io::sink())?;
            }
            Some(target) => {
                let temp = part_path(&target);
                let result = (|| -> Result<(), AppError> {
                    let mut out = BufWriter::new(File::create(&temp)?);
                    self.copy(entry, source, &mut out)?;
                    let file = out
                        .into_inner()
                        .map_err(|e| AppError::from(e.into_error()))?;
                    if let Some(secs) = entry.modified {
                        let _ =
                            file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs));
                    }
                    drop(file);
                    fs::rename(&temp, &target)?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = fs::remove_file(&temp);
                }
                result?;
            }
        }
        self.files_verified += 1;
        Ok(())
    }

    fn copy(
        &mut self,
        entry: &NineZEntryMeta,
        source: &mut dyn Read,
        sink: &mut dyn Write,
    ) -> Result<(), AppError> {
        let mut hasher = Sha256::new();
        let mut remaining = entry.uncompressed_size;
        while remaining > 0 {
            if TaskManager::is_cancelled(self.cancel_flag) {
                return Err(AppError::new("Operation cancelled by user"));
            }
            let want = remaining.min(self.chunk.len() as u64) as usize;
            let n = read_full(source, &mut self.chunk[..want])?;
            if n == 0 {
                break;
            }
            hasher.update(&self.chunk[..n]);
            sink.write_all(&self.chunk[..n])?;
            remaining -= n as u64;
            self.meter.add_bytes(n as u64, &entry.path);
        }
        if remaining > 0
            || (!entry.sha256.is_empty() && format!("{:x}", hasher.finalize()) != entry.sha256)
        {
            return Err(AppError::new(format!(
                "VSA Integrity Seal failed: {} does not match its original fingerprint. \
                 It is damaged beyond repair or has been altered.",
                entry.path
            )));
        }
        Ok(())
    }
}

/// Runs every entry of a version 2/3 archive through the unpacker.
fn unpack_all(
    reader: &mut ProtectedReader,
    manifest: &NineZManifest,
    unpacker: &mut Unpacker,
    threads: usize,
) -> Result<(), AppError> {
    // Folders first, so empty folders exist even if a later file fails.
    for entry in manifest.entries.iter().filter(|e| e.is_dir) {
        if !unpacker.wants(entry) {
            continue;
        }
        if let Destination::Folder { dir, .. } = unpacker.destination {
            fs::create_dir_all(safe_join(dir, &entry.path)?)?;
        }
        unpacker.meter.file_done(&entry.path);
    }

    // Loose files (stored, or zstd in version 2), in archive order.
    let mut loose: Vec<&NineZEntryMeta> = manifest
        .entries
        .iter()
        .filter(|e| !e.is_dir && e.block.is_none() && unpacker.wants(e))
        .collect();
    loose.sort_by_key(|e| e.offset);
    for entry in loose {
        if TaskManager::is_cancelled(unpacker.cancel_flag) {
            return Err(AppError::new("Operation cancelled by user"));
        }
        reader.pos = entry.offset;
        let result = {
            let limited = (&mut *reader).take(entry.compressed_size);
            match entry.method.as_str() {
                "store" => {
                    let mut source = limited;
                    unpacker.unpack(entry, &mut source)
                }
                "zstd" => zstd::stream::read::Decoder::new(limited)
                    .map_err(AppError::from)
                    .and_then(|mut source| unpacker.unpack(entry, &mut source)),
                other => Err(AppError::new(format!(
                    "Unknown compression method '{}' for {}",
                    other, entry.path
                ))),
            }
        };
        if let Err(e) = result.map_err(|e| reader.explain(e)) {
            if e.is_cancel() {
                return Err(e);
            }
            unpacker.damaged.push(entry.path.clone());
        }
        unpacker.meter.file_done(&entry.path);
    }

    // Solid blocks: decode each once, handing out its files in order.
    for (index, block) in manifest.blocks.iter().enumerate() {
        let mut members: Vec<&NineZEntryMeta> = manifest
            .entries
            .iter()
            .filter(|e| !e.is_dir && e.block == Some(index))
            .collect();
        members.sort_by_key(|e| e.offset);
        // Skip blocks with nothing selected; trim the tail after the last pick.
        while members.last().is_some_and(|e| !unpacker.wants(e)) {
            members.pop();
        }
        if members.is_empty() {
            continue;
        }
        if block.method != "lzma2" {
            return Err(AppError::new(format!(
                "This .9z archive uses an unknown compression method '{}'. \
                 It may need a newer version of 9-zip.",
                block.method
            )));
        }

        reader.pos = block.offset;
        let mut done = 0usize;
        let result = (|| -> Result<(), AppError> {
            let limited = (&mut *reader).take(block.packed_size);
            let mut source: Box<dyn Read + '_> = if threads > 1 {
                Box::new(lzma_rust2::Lzma2ReaderMt::new(
                    limited,
                    block.dict_size,
                    None,
                    threads as u32,
                ))
            } else {
                Box::new(lzma_rust2::Lzma2Reader::new(limited, block.dict_size, None))
            };
            let mut position = 0u64;
            for entry in &members {
                if entry.offset > position {
                    let gap = entry.offset - position;
                    io::copy(&mut (&mut source).take(gap), &mut io::sink())?;
                    position = entry.offset;
                }
                if unpacker.wants(entry) {
                    unpacker.unpack(entry, &mut source)?;
                } else {
                    io::copy(
                        &mut (&mut source).take(entry.uncompressed_size),
                        &mut io::sink(),
                    )?;
                }
                position += entry.uncompressed_size;
                done += 1;
                unpacker.meter.file_done(&entry.path);
            }
            Ok(())
        })();
        // Beyond repair: this block's remaining files are lost, but the
        // other blocks may be fine.
        if let Err(e) = result.map_err(|e| reader.explain(e)) {
            if e.is_cancel() {
                return Err(e);
            }
            for entry in &members[done..] {
                if unpacker.wants(entry) {
                    unpacker.damaged.push(entry.path.clone());
                }
            }
        }
    }
    if unpacker.damaged.is_empty() {
        Ok(())
    } else {
        Err(AppError::partly_damaged(&unpacker.damaged))
    }
}

fn total_size(manifest: &NineZManifest) -> u64 {
    manifest
        .entries
        .iter()
        .filter(|e| !e.is_dir)
        .map(|e| e.uncompressed_size)
        .sum()
}

pub fn extract_nine_z(
    options: &ExtractOptions,
    dest_dir: &Path,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<NineZHealth, AppError> {
    let archive_path = Path::new(&options.archive_path);
    if is_version_1(archive_path)? {
        v1::extract(options, dest_dir, cancel_flag, progress)?;
        return Ok(NineZHealth {
            format_version: 1,
            ..Default::default()
        });
    }

    let mut reader = ProtectedReader::open(archive_path)?;
    let manifest = read_manifest_protected(&mut reader)?;
    let mut unpacker = Unpacker {
        destination: Destination::Folder {
            dir: dest_dir,
            overwrite: options.overwrite,
        },
        selection: Some(options),
        cancel_flag,
        meter: Meter::new(
            progress,
            &options.task_id,
            "extracting",
            manifest.entries.len(),
            total_size(&manifest),
        ),
        chunk: vec![0u8; COPY_CHUNK],
        files_verified: 0,
        damaged: Vec::new(),
    };
    unpack_all(&mut reader, &manifest, &mut unpacker, MAX_THREADS.min(4))?;

    reader.health.files_verified = unpacker.files_verified;
    if let Some(message) = reader.health.repair_notice() {
        unpacker.meter.notice("repaired", message);
    }
    Ok(reader.health)
}

/// Reads every block and every file of the archive without writing anything,
/// like 7-Zip's Test command.
pub fn test_nine_z(
    archive_path: &Path,
    task_id: &str,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<NineZHealth, AppError> {
    if is_version_1(archive_path)? {
        return Err(AppError::new(
            "This .9z archive was made by an early test version of 9-zip and has no block-level \
             self-healing. Extract it and add the files to a new .9z archive to protect them.",
        ));
    }

    let mut reader = ProtectedReader::open(archive_path)?;
    let manifest = read_manifest_protected(&mut reader)?;
    let mut unpacker = Unpacker {
        destination: Destination::Check,
        selection: None,
        cancel_flag,
        meter: Meter::new(
            progress,
            task_id,
            "testing",
            manifest.entries.len(),
            total_size(&manifest),
        ),
        chunk: vec![0u8; COPY_CHUNK],
        files_verified: 0,
        damaged: Vec::new(),
    };
    unpack_all(&mut reader, &manifest, &mut unpacker, MAX_THREADS.min(4))?;
    reader.health.files_verified = unpacker.files_verified;
    Ok(reader.health)
}

// ---------------------------------------------------------------------------
// Version 1 (read-only)
// ---------------------------------------------------------------------------

mod v1 {
    use super::*;

    fn open(archive_path: &Path) -> Result<(File, NineZManifest), AppError> {
        let mut file = File::open(archive_path)?;
        let mut magic = [0u8; 4];
        file.read_exact(&mut magic)?;
        if &magic != NINE_Z_MAGIC_V1 {
            return Err(AppError::new("Not a valid .9z archive header"));
        }
        let mut len_buf = [0u8; 4];
        file.read_exact(&mut len_buf)?;
        let manifest_len = u32::from_le_bytes(len_buf) as usize;
        let mut manifest_bytes = vec![0u8; manifest_len];
        file.read_exact(&mut manifest_bytes)?;
        let manifest = serde_json::from_slice(&manifest_bytes)?;
        Ok((file, manifest))
    }

    pub fn read_manifest(archive_path: &Path) -> Result<NineZManifest, AppError> {
        Ok(open(archive_path)?.1)
    }

    pub fn extract(
        options: &ExtractOptions,
        dest_dir: &Path,
        cancel_flag: &Arc<AtomicBool>,
        progress: &Channel<ProgressPayload>,
    ) -> Result<(), AppError> {
        let (file, manifest) = open(Path::new(&options.archive_path))?;
        let total_bytes = manifest.entries.iter().map(|e| e.uncompressed_size).sum();
        let mut meter = Meter::new(
            progress,
            &options.task_id,
            "extracting",
            manifest.entries.len(),
            total_bytes,
        );
        let mut file = io::BufReader::new(file);

        for entry in &manifest.entries {
            if TaskManager::is_cancelled(cancel_flag) {
                return Err(AppError::new("Extraction cancelled by user"));
            }
            let target = safe_join(dest_dir, &entry.path)?;
            if entry.is_dir {
                fs::create_dir_all(&target)?;
            } else {
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut packed = Vec::new();
                (&mut file)
                    .take(entry.compressed_size)
                    .read_to_end(&mut packed)?;
                let data = if entry.method == "zstd" {
                    zstd::decode_all(&packed[..]).map_err(|e| {
                        AppError::new(format!("Zstandard decompression error: {}", e))
                    })?
                } else {
                    packed
                };
                let sha = format!("{:x}", Sha256::digest(&data));
                if !entry.sha256.is_empty() && sha != entry.sha256 {
                    return Err(AppError::new(format!(
                        "VSA Integrity Seal failed: corrupted or altered file: {}",
                        entry.path
                    )));
                }
                if !target.exists() || options.overwrite {
                    fs::write(&target, &data)?;
                }
                meter.add_bytes(entry.uncompressed_size, &entry.path);
            }
            meter.file_done(&entry.path);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compress::scan_inputs;
    use crate::models::CompressionFormat;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("9z_selfheal_{}", name));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn channel() -> Channel<ProgressPayload> {
        Channel::new(|_| Ok(()))
    }

    fn no_cancel() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// Deterministic incompressible bytes.
    fn noise(len: usize, mut seed: u64) -> Vec<u8> {
        (0..len)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect()
    }

    fn pack(src: &Path, archive: &Path, recovery: RecoveryLevel, threads: usize) {
        let options = CompressOptions {
            input_paths: vec![src.to_string_lossy().to_string()],
            output_path: archive.to_string_lossy().to_string(),
            format: CompressionFormat::NineZ,
            level: CompressionLevel::Normal,
            password: None,
            threads,
            task_id: "t".to_string(),
            recovery,
            encrypt_names: true,
        };
        let (items, total) = scan_inputs(&options.input_paths).expect("scan");
        compress_nine_z(&items, total, &options, &no_cancel(), &channel()).expect("compress");
    }

    const FILES: [&str; 6] = [
        "random.bin",
        "photo.jpg",
        "notes.txt",
        "nested/small.json",
        "nested/code.rs",
        "empty.txt",
    ];

    /// A source folder with incompressible data (stored), compressible text
    /// (solid), a nested folder and an empty file, packed into `archive.9z`.
    fn make_archive(dir: &Path, recovery: RecoveryLevel) -> (PathBuf, PathBuf) {
        let src = dir.join("src");
        fs::create_dir_all(src.join("nested")).expect("mkdir");
        fs::create_dir_all(src.join("empty_folder")).expect("mkdir");
        // Over 10 MB, so the archive spans more than one group.
        fs::write(src.join("random.bin"), noise(11_000_000, 42)).expect("write");
        fs::write(src.join("photo.jpg"), noise(400_000, 7)).expect("write");
        fs::write(
            src.join("notes.txt"),
            "Vidyasagar Academy 9-zip self-healing test. ".repeat(60_000),
        )
        .expect("write");
        fs::write(src.join("nested").join("small.json"), b"{\"ok\":true}").expect("write");
        fs::write(
            src.join("nested").join("code.rs"),
            "fn main() { println!(\"hello\"); }\n".repeat(2_000),
        )
        .expect("write");
        fs::write(src.join("empty.txt"), b"").expect("write");

        let archive = dir.join("archive.9z");
        pack(&src, &archive, recovery, 1);
        (archive, src)
    }

    fn extract_to(archive: &Path, dest: &Path) -> Result<NineZHealth, AppError> {
        let options = ExtractOptions {
            archive_path: archive.to_string_lossy().to_string(),
            destination: dest.to_string_lossy().to_string(),
            password: None,
            overwrite: true,
            task_id: "t".to_string(),
            entries: None,
        };
        extract_nine_z(&options, dest, &no_cancel(), &channel())
    }

    fn assert_same_tree(src: &Path, extracted_root: &Path) {
        for name in FILES {
            let a = fs::read(src.join(name)).expect("read source");
            let b = fs::read(extracted_root.join("src").join(name)).expect("read extracted");
            assert!(a == b, "{} differs after extraction", name);
        }
        assert!(extracted_root.join("src").join("empty_folder").is_dir());
    }

    fn damage(path: &Path, offset: u64, len: usize) {
        let mut bytes = fs::read(path).expect("read archive");
        for b in &mut bytes[offset as usize..offset as usize + len] {
            *b = !*b;
        }
        fs::write(path, bytes).expect("write archive");
    }

    fn layout(path: &Path) -> Layout {
        let bytes = fs::read(path).expect("read archive");
        let mut block = [0u8; SUPERBLOCK_LEN];
        block.copy_from_slice(&bytes[..SUPERBLOCK_LEN]);
        Layout::decode(&block).expect("valid superblock")
    }

    /// File position of shard `shard` of stripe `stripe` in group `group`.
    fn shard_offset(l: &Layout, group: u64, stripe: usize, shard: usize) -> u64 {
        let (stripes, shard_len) = l.group_geometry(group);
        l.group_offset(group) + ((shard * stripes + stripe) * shard_len) as u64
    }

    #[test]
    fn round_trip_is_exact_and_solid() {
        let tmp = TempDir::new("roundtrip");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let l = layout(&archive);
        assert_eq!(l.version, 3);
        assert_eq!(l.parity_shards, 4);
        assert!(l.group_count >= 2, "test data should span several groups");

        let manifest = read_manifest_nine_z(&archive).expect("manifest");
        let method = |name: &str| {
            manifest
                .entries
                .iter()
                .find(|e| e.path.ends_with(name))
                .map(|e| e.method.clone())
                .expect("entry")
        };
        assert_eq!(
            method("random.bin"),
            "store",
            "random data is stored as it is"
        );
        assert_eq!(
            method("photo.jpg"),
            "store",
            "photos are stored as they are"
        );
        assert_eq!(method("notes.txt"), "solid");
        assert_eq!(method("code.rs"), "solid");
        assert_eq!(
            manifest.blocks.len(),
            1,
            "small text files share one solid block"
        );
        assert_eq!(manifest.recovery_percent, 10);

        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract");
        assert_same_tree(&src, &dest);
        assert_eq!(health.blocks_repaired, 0);
        assert_eq!(health.files_verified, 6);
        assert!(!health.header_repaired && !health.index_repaired && !health.index_lost);
    }

    #[test]
    fn multithreaded_round_trip() {
        let tmp = TempDir::new("mt");
        let src = tmp.0.join("src");
        fs::create_dir_all(&src).expect("mkdir");
        let text: String = (0..60_000)
            .map(|i| format!("line {} of the 9-zip multithread test\n", i))
            .collect();
        fs::write(src.join("big.txt"), &text).expect("write");
        let archive = tmp.0.join("mt.9z");
        pack(&src, &archive, RecoveryLevel::Normal, 4);
        let dest = tmp.0.join("out");
        extract_to(&archive, &dest).expect("extract");
        assert_eq!(
            fs::read_to_string(dest.join("src").join("big.txt")).expect("read"),
            text
        );
    }

    #[test]
    fn recovery_levels_change_size_as_promised() {
        let tmp = TempDir::new("levels");
        let src = tmp.0.join("src");
        fs::create_dir_all(&src).expect("mkdir");
        fs::write(src.join("data.bin"), noise(3_000_000, 3)).expect("write");
        let mut sizes = Vec::new();
        for (level, name) in [
            (RecoveryLevel::Low, "low.9z"),
            (RecoveryLevel::Normal, "normal.9z"),
            (RecoveryLevel::High, "high.9z"),
        ] {
            let archive = tmp.0.join(name);
            pack(&src, &archive, level, 1);
            sizes.push(fs::metadata(&archive).expect("meta").len() as f64);
        }
        let data = 3_000_000f64;
        for (size, percent) in sizes.iter().zip([5.0, 10.0, 20.0]) {
            let overhead = (size - data) / data * 100.0;
            assert!(
                (overhead - percent).abs() < 1.0,
                "expected about {percent}% extra, got {overhead:.2}%"
            );
        }
    }

    #[test]
    fn repairs_scattered_damage() {
        let tmp = TempDir::new("repair");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let l = layout(&archive);

        // Four bad blocks in one stripe (the most Normal can repair), plus
        // damage in other stripes and groups, including a parity block.
        for shard in [0, 9, 17, 41] {
            damage(&archive, shard_offset(&l, 0, 3, shard) + 100, 512);
        }
        damage(&archive, shard_offset(&l, 0, 7, 5), 10);
        damage(&archive, shard_offset(&l, 1, 0, 2) + 500, 1);

        let tested = test_nine_z(&archive, "t", &no_cancel(), &channel()).expect("test");
        assert_eq!(tested.blocks_repaired, 6);
        assert_eq!(tested.files_verified, 6);

        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract after damage");
        assert_same_tree(&src, &dest);
        assert_eq!(health.blocks_repaired, 6);
        assert!(health.repair_notice().is_some());
    }

    #[test]
    fn repairs_one_long_damaged_stretch() {
        let tmp = TempDir::new("burst");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let l = layout(&archive);

        // Wipe 960 KB in one go, as a bad sector run or a broken download
        // would. Interleaving spreads it over all 16 stripes of the group.
        let start = l.group_offset(0) + 200_000;
        damage(&archive, start, 960 * 1024);

        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract after a long burst");
        assert_same_tree(&src, &dest);
        assert!(health.blocks_repaired >= 60);
    }

    #[test]
    fn survives_damaged_header_and_index() {
        let tmp = TempDir::new("header");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let l = layout(&archive);

        damage(&archive, 0, 20);
        damage(&archive, l.crc_table_offset + 8, 4);

        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract");
        assert_same_tree(&src, &dest);
        assert!(health.header_repaired);
        assert!(health.index_repaired);
    }

    #[test]
    fn repairs_damage_in_the_file_list() {
        let tmp = TempDir::new("manifest");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let l = layout(&archive);

        let group = l.payload_len / l.group_capacity();
        let (stripes, shard_len) = l.group_geometry(group);
        let within = (l.payload_len % l.group_capacity()) as usize;
        let stripe = within / (l.data_shards * shard_len);
        let shard = (within % (l.data_shards * shard_len)) / shard_len;
        assert!(stripe < stripes);
        damage(&archive, shard_offset(&l, group, stripe, shard), 8);

        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract");
        assert_same_tree(&src, &dest);
        assert_eq!(health.blocks_repaired, 1);
    }

    #[test]
    fn reports_damage_beyond_repair_clearly() {
        let tmp = TempDir::new("beyond");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Low);
        let l = layout(&archive);

        // Three bad blocks in one stripe at the start, inside random.bin:
        // more than Low (2) can rebuild.
        for shard in [0, 4, 8] {
            damage(&archive, shard_offset(&l, 0, 1, shard), 16);
        }

        let out = tmp.0.join("out");
        let err = extract_to(&archive, &out).expect_err("must report the damage");
        assert_eq!(err.code, Some("partly_damaged"), "message: {}", err.message);
        assert!(err.message.contains("random.bin"), "message: {}", err.message);
        // Like 7-Zip, the undamaged files still come out, complete.
        assert_eq!(
            fs::read(out.join("src").join("notes.txt")).expect("notes.txt extracted"),
            fs::read(src.join("notes.txt")).expect("source")
        );
        assert!(!out.join("src").join("random.bin").exists(), "no broken file is left");
        let leftovers: Vec<_> = walkdir::WalkDir::new(tmp.0.join("out"))
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().ends_with(".9z-part"))
            .collect();
        assert!(leftovers.is_empty(), "temporary files must be cleaned up");
    }

    #[test]
    fn small_archive_has_little_overhead() {
        let tmp = TempDir::new("small");
        let src = tmp.0.join("hello.txt");
        fs::write(&src, b"hello").expect("write");
        let archive = tmp.0.join("small.9z");
        pack(&src, &archive, RecoveryLevel::Normal, 1);
        let size = fs::metadata(&archive).expect("meta").len();
        assert!(
            size < 1024,
            "a 5-byte file should give a small archive, got {size} bytes"
        );
        let dest = tmp.0.join("out");
        extract_to(&archive, &dest).expect("extract");
        assert_eq!(fs::read(dest.join("hello.txt")).expect("read"), b"hello");
    }

    #[test]
    fn keeps_existing_files_without_overwrite() {
        let tmp = TempDir::new("keep");
        let (archive, src) = make_archive(&tmp.0, RecoveryLevel::Normal);
        let dest = tmp.0.join("out");
        fs::create_dir_all(dest.join("src")).expect("mkdir");
        fs::write(dest.join("src").join("notes.txt"), b"mine").expect("write");
        let options = ExtractOptions {
            archive_path: archive.to_string_lossy().to_string(),
            destination: dest.to_string_lossy().to_string(),
            password: None,
            overwrite: false,
            task_id: "t".to_string(),
            entries: None,
        };
        extract_nine_z(&options, &dest, &no_cancel(), &channel()).expect("extract");
        assert_eq!(
            fs::read(dest.join("src").join("notes.txt")).expect("read"),
            b"mine"
        );
        assert_eq!(
            fs::read(dest.join("src").join("nested").join("code.rs")).expect("read"),
            fs::read(src.join("nested").join("code.rs")).expect("read")
        );
    }

    /// Run with `cargo test --release -- --ignored large_file` while watching
    /// memory: it should stay far below the file size.
    #[test]
    #[ignore]
    fn large_file_streams() {
        let tmp = TempDir::new("large");
        let src = tmp.0.join("big.bin");
        {
            let mut out = BufWriter::new(File::create(&src).expect("create"));
            for i in 0..400u64 {
                out.write_all(&noise(1024 * 1024, i + 1)).expect("write");
            }
        }
        let archive = tmp.0.join("big.9z");
        pack(&src, &archive, RecoveryLevel::Normal, 1);
        let dest = tmp.0.join("out");
        let health = extract_to(&archive, &dest).expect("extract");
        assert_eq!(health.files_verified, 1);
        assert_eq!(
            fs::metadata(dest.join("big.bin")).expect("meta").len(),
            400 * 1024 * 1024
        );
    }

    #[test]
    fn blocks_path_traversal() {
        let dest = Path::new("C:/dest");
        assert!(safe_join(dest, "../evil.txt").is_err());
        assert!(safe_join(dest, "a/../../evil.txt").is_err());
        assert!(safe_join(dest, "C:/Windows/evil.txt").is_err());
        assert!(safe_join(dest, "/evil.txt").is_err());
        assert!(safe_join(dest, "ok..name.txt").is_ok());
        assert!(safe_join(dest, "folder\\file.txt").is_ok());
    }
}
