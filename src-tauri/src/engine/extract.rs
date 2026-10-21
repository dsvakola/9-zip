use crate::error::AppError;
use crate::models::{CompressionFormat, ExtractOptions, ProgressPayload};
use crate::task_manager::TaskManager;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tauri::ipc::Channel;
use zip::ZipArchive;

pub fn extract_archive(
    options: ExtractOptions,
    cancel_flag: Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let dest_dir = PathBuf::from(&options.destination);
    fs::create_dir_all(&dest_dir)?;

    let archive_path = Path::new(&options.archive_path);
    if !archive_path.exists() {
        return Err(AppError::new(format!(
            "Archive does not exist: {}",
            options.archive_path
        )));
    }

    let detected_format = CompressionFormat::from_path(&options.archive_path).ok_or_else(|| {
        AppError::new(format!(
            "Unsupported archive format: {}",
            options.archive_path
        ))
    })?;

    let task_id = &options.task_id;
    let _ = progress.send(ProgressPayload {
        task_id: task_id.clone(),
        phase: "extracting".to_string(),
        current_file: "Opening archive...".to_string(),
        files_processed: 0,
        total_files: 0,
        bytes_processed: 0,
        total_bytes: 0,
        percent: 0.0,
        speed_mb_per_sec: 0.0,
    });

    match detected_format {
        CompressionFormat::NineZ => {
            crate::engine::nine_z_format::extract_nine_z(
                &options,
                &dest_dir,
                &cancel_flag,
                progress,
            )?;
        }
        CompressionFormat::Zip => {
            extract_zip(&options, &dest_dir, &cancel_flag, progress)?;
        }
        CompressionFormat::SevenZ => {
            extract_7z(&options, &dest_dir, &cancel_flag, progress)?;
        }
        tar_family => {
            let reader = open_tar_stream(tar_family, File::open(archive_path)?)?;
            extract_tar(reader, &options, &dest_dir, &cancel_flag, progress)?;
        }
    }

    let _ = progress.send(ProgressPayload {
        task_id: options.task_id.clone(),
        phase: "done".to_string(),
        current_file: "Extraction completed successfully".to_string(),
        files_processed: 1,
        total_files: 1,
        bytes_processed: 0,
        total_bytes: 0,
        percent: 100.0,
        speed_mb_per_sec: 0.0,
    });

    Ok(())
}

/// Opens the decompressed TAR stream of a TAR-based format.
pub(crate) fn open_tar_stream<'a>(
    format: CompressionFormat,
    source: impl Read + 'a,
) -> Result<Box<dyn Read + 'a>, AppError> {
    let source = BufReader::new(source);
    Ok(match format {
        CompressionFormat::Tar => Box::new(source),
        CompressionFormat::Gzip => Box::new(flate2::read::GzDecoder::new(source)),
        CompressionFormat::Bzip2 => Box::new(bzip2::read::BzDecoder::new(source)),
        CompressionFormat::Xz => Box::new(lzma_rust2::XzReader::new(source, true)),
        CompressionFormat::Zstd => Box::new(
            zstd::stream::Decoder::new(source)
                .map_err(|e| AppError::new(format!("Zstandard decoder error: {}", e)))?,
        ),
        CompressionFormat::Lz4 => Box::new(lz4_flex::frame::FrameDecoder::new(source)),
        CompressionFormat::Brotli => Box::new(brotli::Decompressor::new(source, 4096)),
        CompressionFormat::NineZ | CompressionFormat::Zip | CompressionFormat::SevenZ => {
            return Err(AppError::new("Not a TAR-based archive format"));
        }
    })
}

pub(crate) fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, AppError> {
    let clean_rel = rel.replace('\\', "/");
    let mut target = base.to_path_buf();
    for part in clean_rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(AppError::new("Illegal path traversal detected in archive"));
        }
        target.push(part);
    }
    Ok(target)
}

fn extract_zip(
    options: &ExtractOptions,
    dest_dir: &Path,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let file = File::open(&options.archive_path)?;
    let mut archive = ZipArchive::new(BufReader::new(file))?;
    let total_files = archive.len();
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_processed = 0u64;

    for i in 0..total_files {
        if TaskManager::is_cancelled(cancel_flag) {
            return Err(AppError::new("Extraction cancelled by user"));
        }

        let mut entry = match &options.password {
            Some(pass) if !pass.is_empty() => archive.by_index_decrypt(i, pass.as_bytes())?,
            _ => archive.by_index(i)?,
        };

        let raw_name = entry.name().to_string();
        if !options.wants(&raw_name) {
            continue;
        }
        let target_path = safe_join(dest_dir, &raw_name)?;

        if entry.is_dir() {
            fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if target_path.exists() && !options.overwrite {
                continue;
            }
            let mut out_file = File::create(&target_path)?;
            let copied = std::io::copy(&mut entry, &mut out_file)?;
            bytes_processed += copied;
        }

        if last_emit.elapsed().as_millis() >= 100 || i == total_files - 1 {
            let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
            let speed = (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed;
            let percent = ((i + 1) as f32 / total_files as f32) * 100.0;

            let _ = progress.send(ProgressPayload {
                task_id: options.task_id.clone(),
                phase: "extracting".to_string(),
                current_file: raw_name,
                files_processed: i + 1,
                total_files,
                bytes_processed,
                total_bytes: bytes_processed,
                percent,
                speed_mb_per_sec: speed,
            });
            last_emit = Instant::now();
        }
    }

    Ok(())
}

fn extract_7z(
    options: &ExtractOptions,
    dest_dir: &Path,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let mut reader = open_7z(&options.archive_path, options.password.as_deref())?;
    let total_files = reader.archive().files.len();
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_processed = 0u64;
    let mut count = 0usize;
    // Errors from inside the callback, kept with their own message.
    let mut failure: Option<AppError> = None;

    let result = reader.for_each_entries(|entry, data| {
        if TaskManager::is_cancelled(cancel_flag) {
            failure = Some(AppError::new("Extraction cancelled by user"));
            return Ok(false);
        }
        let step = (|| -> Result<(), AppError> {
            if !options.wants(entry.name()) {
                // Solid 7z data is read in order, so pass over it.
                std::io::copy(data, &mut std::io::sink())?;
                return Ok(());
            }
            let target_path = safe_join(dest_dir, entry.name())?;
            if entry.is_directory() {
                fs::create_dir_all(&target_path)?;
                return Ok(());
            }
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if target_path.exists() && !options.overwrite {
                std::io::copy(data, &mut std::io::sink())?;
                return Ok(());
            }
            let mut out = File::create(&target_path)?;
            bytes_processed += std::io::copy(data, &mut out)?;
            Ok(())
        })();
        if let Err(e) = step {
            failure = Some(e);
            return Ok(false);
        }

        count += 1;
        if last_emit.elapsed().as_millis() >= 100 || count == total_files {
            let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
            let _ = progress.send(ProgressPayload {
                task_id: options.task_id.clone(),
                phase: "extracting".to_string(),
                current_file: entry.name().to_string(),
                files_processed: count,
                total_files,
                bytes_processed,
                total_bytes: bytes_processed,
                percent: count as f32 / total_files.max(1) as f32 * 100.0,
                speed_mb_per_sec: (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed,
            });
            last_emit = Instant::now();
        }
        Ok(true)
    });

    if let Some(e) = failure {
        // With a wrong password, AES gives garbage that the decoder rejects.
        let with_password = options.password.as_deref().is_some_and(|p| !p.is_empty());
        let cancelled = e.message.contains("cancelled");
        return Err(if with_password && e.code.is_none() && !cancelled {
            AppError::wrong_password()
        } else {
            e
        });
    }
    result.map_err(AppError::from)
}

pub(crate) fn open_7z(
    archive_path: &str,
    password: Option<&str>,
) -> Result<sevenz_rust2::ArchiveReader<File>, AppError> {
    let password = match password {
        Some(p) if !p.is_empty() => p.into(),
        _ => sevenz_rust2::Password::empty(),
    };
    sevenz_rust2::ArchiveReader::open(archive_path, password).map_err(AppError::from)
}

fn extract_tar<R: Read>(
    reader: R,
    options: &ExtractOptions,
    dest_dir: &Path,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let mut archive = tar::Archive::new(reader);
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_processed = 0u64;
    let mut count = 0;

    for entry_res in archive.entries()? {
        if TaskManager::is_cancelled(cancel_flag) {
            return Err(AppError::new("Extraction cancelled by user"));
        }

        let mut entry = entry_res?;
        let path = entry.path()?.to_string_lossy().to_string();
        if !options.wants(&path) {
            continue;
        }
        let target_path = safe_join(dest_dir, &path)?;

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if target_path.exists() && !options.overwrite {
                continue;
            }
            let mut out = File::create(&target_path)?;
            let copied = std::io::copy(&mut entry, &mut out)?;
            bytes_processed += copied;
        }

        count += 1;
        if last_emit.elapsed().as_millis() >= 100 {
            let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
            let speed = (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed;

            let _ = progress.send(ProgressPayload {
                task_id: options.task_id.clone(),
                phase: "extracting".to_string(),
                current_file: path,
                files_processed: count,
                total_files: count + 1,
                bytes_processed,
                total_bytes: bytes_processed,
                percent: 50.0,
                speed_mb_per_sec: speed,
            });
            last_emit = Instant::now();
        }
    }

    Ok(())
}
