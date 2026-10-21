use crate::error::AppError;
use crate::models::{ArchiveEntry, CompressionFormat};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use zip::ZipArchive;

pub fn list_archive_contents(
    archive_path: &str,
    password: Option<String>,
) -> Result<Vec<ArchiveEntry>, AppError> {
    let path = Path::new(archive_path);
    if !path.exists() {
        return Err(AppError::new(format!(
            "File does not exist: {}",
            archive_path
        )));
    }

    let format = CompressionFormat::from_path(archive_path)
        .ok_or_else(|| AppError::new(format!("Unsupported archive format: {}", archive_path)))?;

    match format {
        CompressionFormat::NineZ => crate::engine::nine_z_format::list_nine_z(path),
        CompressionFormat::Zip => list_zip(path),
        CompressionFormat::SevenZ => list_7z(path, password),
        tar_family => list_tar(crate::engine::extract::open_tar_stream(
            tar_family,
            File::open(path)?,
        )?),
    }
}

fn list_zip(path: &Path) -> Result<Vec<ArchiveEntry>, AppError> {
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(BufReader::new(file))?;
    let mut entries = Vec::new();

    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i)?;
        let name = entry.name().to_string();
        let is_dir = entry.is_dir();
        let size = entry.size();
        let compressed_size = entry.compressed_size();
        let crc = format!("{:08X}", entry.crc32());
        let method = format!("{:?}", entry.compression());
        let encrypted = entry.encrypted();

        let last_modified = entry.last_modified().and_then(|t| {
            time::OffsetDateTime::try_from(t)
                .ok()
                .map(|odt| odt.unix_timestamp() as u64)
        });

        let file_name = Path::new(&name)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| name.clone());

        entries.push(ArchiveEntry {
            name: file_name,
            path: name,
            size,
            compressed_size,
            is_dir,
            modified: last_modified,
            crc32: Some(crc),
            method: (!is_dir).then_some(method),
            encrypted,
            packed_known: true,
            sha256: None,
        });
    }

    Ok(entries)
}

fn list_7z(path: &Path, password: Option<String>) -> Result<Vec<ArchiveEntry>, AppError> {
    let mut file = BufReader::new(File::open(path)?);
    let password = match password.as_deref() {
        Some(p) if !p.is_empty() => p.into(),
        _ => sevenz_rust2::Password::empty(),
    };

    let archive = sevenz_rust2::Archive::read(&mut file, &password).map_err(AppError::from)?;

    let mut entries = Vec::new();
    for (index, entry) in archive.files.iter().enumerate() {
        let full_path = entry.name().to_string();
        let is_dir = entry.is_directory();
        let size = entry.size();
        let compressed_size = entry.compressed_size;
        let crc = if entry.has_crc {
            Some(format!("{:08X}", entry.crc))
        } else {
            None
        };

        let file_name = Path::new(&full_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| full_path.clone());

        let block = archive
            .stream_map
            .file_block_index
            .get(index)
            .copied()
            .flatten();
        // In a solid block only the first file carries the packed size.
        let shares_block = block.is_some_and(|b| {
            archive
                .stream_map
                .block_first_file_index
                .get(b)
                .is_some_and(|&first| first != index)
                || archive.files.iter().enumerate().any(|(i, f)| {
                    i != index
                        && f.has_stream
                        && archive
                            .stream_map
                            .file_block_index
                            .get(i)
                            .copied()
                            .flatten()
                            == Some(b)
                })
        });
        let method = block
            .and_then(|b| archive.blocks.get(b))
            .map(seven_z_method);
        entries.push(ArchiveEntry {
            name: file_name,
            path: full_path,
            size,
            compressed_size,
            is_dir,
            modified: entry
                .has_last_modified_date
                .then(|| nt_time_to_unix(u64::from(entry.last_modified_date))),
            crc32: crc,
            encrypted: method.as_deref().is_some_and(|m| m.contains("AES")),
            method,
            packed_known: !shares_block,
            sha256: None,
        });
    }

    Ok(entries)
}

fn list_tar<R: Read>(reader: R) -> Result<Vec<ArchiveEntry>, AppError> {
    let mut archive = tar::Archive::new(reader);
    let mut entries = Vec::new();

    for entry_res in archive.entries()? {
        let entry = entry_res?;
        let full_path = entry.path()?.to_string_lossy().to_string();
        let is_dir = entry.header().entry_type().is_dir();
        let size = entry.size();
        let modified = entry.header().mtime().ok();

        let file_name = Path::new(&full_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| full_path.clone());

        entries.push(ArchiveEntry {
            name: file_name,
            path: full_path,
            size,
            compressed_size: 0,
            is_dir,
            modified,
            crc32: None,
            method: None,
            encrypted: false,
            // The whole TAR stream is compressed as one.
            packed_known: false,
            sha256: None,
        });
    }

    Ok(entries)
}

/// 7z coder chain as 7-Zip names it, such as "LZMA2:16m" or "AES + LZMA2:16m".
pub(crate) fn seven_z_method(block: &sevenz_rust2::Block) -> String {
    let names: Vec<String> = block
        .coders
        .iter()
        .rev()
        .map(|coder| {
            let name = sevenz_rust2::EncoderMethod::by_id(coder.encoder_method_id())
                .map(|m| m.name().to_string())
                .unwrap_or_else(|| "Unknown".to_string());
            let props = coder.properties();
            match name.as_str() {
                "LZMA2" if !props.is_empty() => {
                    let p = u32::from(props[0]);
                    let dict = if p >= 40 {
                        u32::MAX
                    } else {
                        (2 | (p & 1)) << (p / 2 + 11)
                    };
                    format!("LZMA2:{}", human_dict(dict))
                }
                "LZMA" if props.len() >= 5 => {
                    let dict = u32::from_le_bytes([props[1], props[2], props[3], props[4]]);
                    format!("LZMA:{}", human_dict(dict))
                }
                other => other.to_string(),
            }
        })
        .collect();
    names.join(" + ")
}

fn human_dict(bytes: u32) -> String {
    if bytes >= 1 << 20 {
        format!("{}m", bytes >> 20)
    } else {
        format!("{}k", bytes >> 10)
    }
}

/// Windows FILETIME (100 ns since 1601) to Unix seconds.
fn nt_time_to_unix(time: u64) -> u64 {
    (time / 10_000_000).saturating_sub(11_644_473_600)
}
