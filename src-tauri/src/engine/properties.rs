//! The archive part of the Properties window, like 7-Zip's: type, sizes,
//! method, solid blocks, and for .9z its recovery data and seal.

use crate::engine::list::{list_archive_contents, seven_z_method};
use crate::engine::nine_z_format::{self, NineZDetails};
use crate::error::AppError;
use crate::models::CompressionFormat;
use serde::Serialize;
use std::collections::BTreeSet;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveInfo {
    pub path: String,
    /// "9z", "7z", "zip", "tar.gz", ...
    pub kind: String,
    pub physical_size: u64,
    pub size: u64,
    /// Sum of the packed data; None when the format does not record it.
    pub packed_size: Option<u64>,
    pub files: usize,
    pub folders: usize,
    pub method: String,
    pub solid: Option<bool>,
    pub blocks: Option<usize>,
    pub headers_size: Option<u64>,
    pub encrypted: bool,
    pub nine_z: Option<NineZDetails>,
}

fn kind_of(path: &str) -> String {
    let lower = path.to_lowercase();
    for kind in [
        "tar.gz", "tar.bz2", "tar.xz", "tar.zst", "tar.lz4", "tar.br",
    ] {
        if lower.ends_with(&format!(".{}", kind)) {
            return kind.to_string();
        }
    }
    Path::new(&lower)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn archive_properties(path: &str, password: Option<String>) -> Result<ArchiveInfo, AppError> {
    let format = CompressionFormat::from_path(path)
        .ok_or_else(|| AppError::new(format!("Unsupported archive format: {}", path)))?;
    let entries = list_archive_contents(path, password.clone())?;

    // Folders: those listed, plus any only implied by a file's path.
    let mut folders = BTreeSet::new();
    for entry in &entries {
        let clean = entry.path.replace('\\', "/");
        let clean = clean.trim_end_matches('/');
        if entry.is_dir {
            folders.insert(clean.to_string());
        }
        let mut parts: Vec<&str> = clean.split('/').collect();
        parts.pop();
        for i in 1..=parts.len() {
            folders.insert(parts[..i].join("/"));
        }
    }
    let files: Vec<_> = entries.iter().filter(|e| !e.is_dir).collect();
    let mut info = ArchiveInfo {
        path: path.to_string(),
        kind: kind_of(path),
        physical_size: Path::new(path).metadata()?.len(),
        size: files.iter().map(|e| e.size).sum(),
        files: files.len(),
        folders: folders.len(),
        encrypted: files.iter().any(|e| e.encrypted),
        ..Default::default()
    };

    match format {
        CompressionFormat::NineZ => {
            let details = nine_z_format::nine_z_details(Path::new(path))?;
            info.packed_size = Some(
                info.physical_size
                    .saturating_sub(details.recovery_size + details.headers_size),
            );
            info.method = match details.dictionary {
                Some(dict) if details.stored_files > 0 => {
                    format!("LZMA2:{}m (solid) + Stored", dict >> 20)
                }
                Some(dict) => format!("LZMA2:{}m (solid)", dict >> 20),
                None => "Stored".to_string(),
            };
            info.solid = Some(details.solid_blocks > 0);
            info.blocks = Some(details.solid_blocks);
            info.headers_size = Some(details.headers_size);
            info.nine_z = Some(details);
        }
        CompressionFormat::SevenZ => {
            let pass = match password.as_deref() {
                Some(p) if !p.is_empty() => p.into(),
                _ => sevenz_rust2::Password::empty(),
            };
            let mut file = BufReader::new(File::open(path)?);
            let archive = sevenz_rust2::Archive::read(&mut file, &pass)?;
            let mut methods: Vec<String> = archive.blocks.iter().map(seven_z_method).collect();
            methods.dedup();
            info.method = methods.join(", ");
            info.solid = Some(archive.is_solid);
            info.blocks = Some(archive.blocks.len());
            let packed: u64 = archive.pack_sizes().iter().sum();
            info.packed_size = Some(packed);
            // 32-byte signature header, then packed data, then the headers.
            info.headers_size = Some(info.physical_size.saturating_sub(packed + 32));
            info.encrypted = info.encrypted || info.method.contains("AES");
        }
        CompressionFormat::Zip => {
            let mut methods: Vec<String> = files.iter().filter_map(|e| e.method.clone()).collect();
            methods.sort();
            methods.dedup();
            info.method = methods.join(", ");
            info.packed_size = Some(files.iter().map(|e| e.compressed_size).sum());
            info.solid = Some(false);
        }
        CompressionFormat::Tar => {
            info.method = "None (container)".to_string();
            info.packed_size = Some(info.physical_size);
        }
        other => {
            info.method = match other {
                CompressionFormat::Gzip => "Deflate",
                CompressionFormat::Bzip2 => "BZip2",
                CompressionFormat::Xz => "LZMA2 (XZ)",
                CompressionFormat::Zstd => "Zstandard",
                CompressionFormat::Lz4 => "LZ4",
                CompressionFormat::Brotli => "Brotli",
                _ => "",
            }
            .to_string();
            info.packed_size = Some(info.physical_size);
            // The whole TAR is compressed as one stream.
            info.solid = Some(true);
            info.blocks = Some(1);
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compress::compress_archive;
    use crate::models::{CompressOptions, CompressionLevel, ProgressPayload, RecoveryLevel};
    use std::fs;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tauri::ipc::Channel;

    #[test]
    fn reports_9z_recovery_and_counts() {
        let dir = std::env::temp_dir().join("9z_props_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src").join("inner")).expect("mkdir");
        fs::write(dir.join("src").join("a.txt"), "alpha\n".repeat(20_000)).expect("write");
        fs::write(
            dir.join("src").join("inner").join("b.jpg"),
            vec![7u8; 50_000],
        )
        .expect("write");
        let archive = dir.join("p.9z");
        compress_archive(
            CompressOptions {
                input_paths: vec![dir.join("src").to_string_lossy().to_string()],
                output_path: archive.to_string_lossy().to_string(),
                format: CompressionFormat::NineZ,
                level: CompressionLevel::Normal,
                password: None,
                threads: 1,
                task_id: "props".to_string(),
                recovery: RecoveryLevel::High,
                encrypt_names: true,
            },
            Arc::new(AtomicBool::new(false)),
            &Channel::<ProgressPayload>::new(|_| Ok(())),
        )
        .expect("compress");

        let info = archive_properties(&archive.to_string_lossy(), None).expect("properties");
        assert_eq!(info.kind, "9z");
        assert_eq!(info.files, 2);
        assert_eq!(info.folders, 2, "src and src/inner");
        assert_eq!(info.size, 120_000 + 50_000);
        let details = info.nine_z.expect("9z details");
        assert_eq!(details.recovery_percent, 20);
        assert_eq!(details.sealed_files, 2);
        assert_eq!(details.solid_files, 1);
        assert_eq!(details.stored_files, 1);
        assert!(details.recovery_size > 0);
        let _ = fs::remove_dir_all(&dir);
    }
}
