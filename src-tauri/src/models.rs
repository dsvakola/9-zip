use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompressionFormat {
    #[serde(rename = "9z")]
    NineZ,
    #[serde(rename = "7z")]
    SevenZ,
    #[serde(rename = "zip")]
    Zip,
    #[serde(rename = "tar")]
    Tar,
    #[serde(rename = "gzip")]
    Gzip,
    #[serde(rename = "bzip2")]
    Bzip2,
    #[serde(rename = "xz")]
    Xz,
    #[serde(rename = "zstd")]
    Zstd,
    #[serde(rename = "lz4")]
    Lz4,
    #[serde(rename = "brotli")]
    Brotli,
}

#[allow(dead_code)]
impl CompressionFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::NineZ => "9z",
            Self::SevenZ => "7z",
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::Gzip => "tar.gz",
            Self::Bzip2 => "tar.bz2",
            Self::Xz => "tar.xz",
            Self::Zstd => "tar.zst",
            Self::Lz4 => "tar.lz4",
            Self::Brotli => "tar.br",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::NineZ => "9Z - Self-Healing & Smart-Adaptive",
            Self::SevenZ => "7Z - High Ratio LZMA/LZMA2",
            Self::Zip => "ZIP - Universal Compatibility",
            Self::Tar => "TAR - Unix Standard Container",
            Self::Gzip => "GZIP (TAR.GZ) - Linux / Web Standard",
            Self::Bzip2 => "BZIP2 (TAR.BZ2) - High Redundancy",
            Self::Xz => "XZ (TAR.XZ) - High Compression LZMA2",
            Self::Zstd => "ZSTD (TAR.ZST) - Real-Time Ultra Fast",
            Self::Lz4 => "LZ4 (TAR.LZ4) - Extreme Speed Throughput",
            Self::Brotli => "BROTLI (TAR.BR) - Web & Text Optimized",
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        let lower = path.to_lowercase();
        if lower.ends_with(".9z") {
            Some(Self::NineZ)
        } else if lower.ends_with(".7z") {
            Some(Self::SevenZ)
        } else if lower.ends_with(".zip") {
            Some(Self::Zip)
        } else if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") || lower.ends_with(".gz") {
            Some(Self::Gzip)
        } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") || lower.ends_with(".bz2")
        {
            Some(Self::Bzip2)
        } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") || lower.ends_with(".xz") {
            Some(Self::Xz)
        } else if lower.ends_with(".tar.zst") || lower.ends_with(".tzst") || lower.ends_with(".zst")
        {
            Some(Self::Zstd)
        } else if lower.ends_with(".tar.lz4") || lower.ends_with(".lz4") {
            Some(Self::Lz4)
        } else if lower.ends_with(".tar.br") || lower.ends_with(".br") {
            Some(Self::Brotli)
        } else if lower.ends_with(".tar") {
            Some(Self::Tar)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompressionLevel {
    Store,
    Fastest,
    Fast,
    Normal,
    Maximum,
    Ultra,
}

impl CompressionLevel {
    pub fn as_u32(&self) -> u32 {
        match self {
            Self::Store => 0,
            Self::Fastest => 1,
            Self::Fast => 3,
            Self::Normal => 5,
            Self::Maximum => 7,
            Self::Ultra => 9,
        }
    }
}

/// How much Reed-Solomon recovery data a .9z archive carries, as a share of
/// its packed data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryLevel {
    /// 5%
    Low,
    /// 10%
    #[default]
    Normal,
    /// 20%
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressOptions {
    pub input_paths: Vec<String>,
    pub output_path: String,
    pub format: CompressionFormat,
    pub level: CompressionLevel,
    pub password: Option<String>,
    pub threads: usize,
    pub task_id: String,
    /// Used by .9z only.
    #[serde(default)]
    pub recovery: RecoveryLevel,
    /// 7z with a password: also hide the file names (7-Zip's
    /// "Encrypt file names").
    #[serde(default = "default_true")]
    pub encrypt_names: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractOptions {
    pub archive_path: String,
    pub destination: String,
    pub password: Option<String>,
    pub overwrite: bool,
    pub task_id: String,
    /// Archive paths to extract (a folder brings everything inside it).
    /// None extracts everything.
    #[serde(default)]
    pub entries: Option<Vec<String>>,
}

impl ExtractOptions {
    /// Whether an archive entry is part of the selection.
    pub fn wants(&self, entry_path: &str) -> bool {
        let Some(selection) = &self.entries else {
            return true;
        };
        let entry = entry_path.replace('\\', "/");
        let entry = entry.trim_end_matches('/');
        selection.iter().any(|chosen| {
            let chosen = chosen.replace('\\', "/");
            let chosen = chosen.trim_end_matches('/');
            entry == chosen
                || (entry.len() > chosen.len()
                    && entry.starts_with(chosen)
                    && entry.as_bytes()[chosen.len()] == b'/')
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressPayload {
    pub task_id: String,
    pub phase: String,
    pub current_file: String,
    pub files_processed: usize,
    pub total_files: usize,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub percent: f32,
    pub speed_mb_per_sec: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub compressed_size: u64,
    pub is_dir: bool,
    pub modified: Option<u64>,
    pub crc32: Option<String>,
    /// How the entry is stored, e.g. "Deflate", "LZMA2 (solid)", "Stored".
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub encrypted: bool,
    /// False when the entry shares a solid block with other files, so it has
    /// no packed size of its own (7-Zip then leaves Packed Size blank).
    #[serde(default)]
    pub packed_known: bool,
    /// Full SHA-256 fingerprint (.9z only).
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_archive: bool,
    pub modified: Option<u64>,
    pub extension: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveItem {
    pub letter: String,
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub drive_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HashResult {
    pub path: String,
    pub file_name: String,
    pub file_size: u64,
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchAction {
    pub action: String,
    /// The first path; `target_paths` has all of them.
    pub target_path: String,
    pub target_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkResult {
    pub threads: usize,
    pub compress_speed_mb_per_sec: f32,
    pub compress_mips: u64,
    pub decompress_speed_mb_per_sec: f32,
    pub decompress_mips: u64,
    pub total_rating_mips: u64,
}
