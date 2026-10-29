use crate::error::AppError;
use crate::models::{CompressOptions, CompressionFormat, CompressionLevel, ProgressPayload};
use crate::task_manager::TaskManager;
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tauri::ipc::Channel;
use walkdir::WalkDir;
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

#[derive(Debug, Clone)]
pub struct InputFileItem {
    pub disk_path: PathBuf,
    pub archive_path: String,
    pub size: u64,
    pub is_dir: bool,
}

pub fn scan_inputs(input_paths: &[String]) -> Result<(Vec<InputFileItem>, u64), AppError> {
    let mut items = Vec::new();
    let mut total_bytes = 0u64;

    for input_str in input_paths {
        let p = Path::new(input_str);
        if !p.exists() {
            return Err(AppError::new(format!(
                "Input path does not exist: {}",
                input_str
            )));
        }

        let base_name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".to_string());

        if p.is_file() {
            let meta = p.metadata()?;
            let size = meta.len();
            total_bytes += size;
            items.push(InputFileItem {
                disk_path: p.to_path_buf(),
                archive_path: base_name,
                size,
                is_dir: false,
            });
        } else if p.is_dir() {
            let prefix_parent = p.parent().unwrap_or(p);
            for entry in WalkDir::new(p).into_iter().filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                let rel = match entry_path.strip_prefix(prefix_parent) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => entry_path.to_string_lossy().replace('\\', "/"),
                };

                let is_dir = entry.file_type().is_dir();
                let size = if is_dir {
                    0
                } else {
                    entry.metadata().map(|m| m.len()).unwrap_or(0)
                };

                if !is_dir {
                    total_bytes += size;
                }

                items.push(InputFileItem {
                    disk_path: entry_path.to_path_buf(),
                    archive_path: rel,
                    size,
                    is_dir,
                });
            }
        }
    }

    Ok((items, total_bytes))
}

/// Message for a folder in which 9-zip may not save the archive.
fn output_folder_error(folder: &Path, err: &std::io::Error) -> AppError {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::new(format!(
            "Windows does not allow 9-zip to save in {}.

Please choose another folder (for example Documents or Desktop).",
            folder.display()
        ))
    } else {
        AppError::new(format!("Cannot save in {}: {}", folder.display(), err))
    }
}

/// Makes sure the archive's folder exists and 9-zip may create files in it,
/// by creating and removing a small test file.
pub(crate) fn check_output_folder(output: &Path) -> Result<(), AppError> {
    let folder = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(folder).map_err(|e| output_folder_error(folder, &e))?;
    let probe = folder.join(format!(".9-zip-write-test-{}.tmp", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(output_folder_error(folder, &e)),
    }
}

/// Creates the archive file; the error names the file.
fn create_output(path: &Path) -> Result<File, AppError> {
    File::create(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::new(format!(
                "Windows does not allow 9-zip to write {}.

The file may be read-only or open in another program, or the folder may be protected. Please choose another name or folder.",
                path.display()
            ))
        } else {
            AppError::new(format!("Cannot create {}: {}", path.display(), e))
        }
    })
}

/// Message for an input file that cannot be read; it names the file.
pub(crate) fn source_error(path: &Path, err: &std::io::Error) -> AppError {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::new(format!(
            "Windows does not allow 9-zip to read {}.

The file may be open in another program or protected. Close it, or leave it out, and try again.",
            path.display()
        ))
    } else {
        AppError::new(format!("Cannot read {}: {}", path.display(), err))
    }
}

/// Opens an input file; the error names the file.
fn open_source(path: &Path) -> Result<File, AppError> {
    File::open(path).map_err(|e| source_error(path, &e))
}

pub fn compress_archive(
    options: CompressOptions,
    cancel_flag: Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let task_id = &options.task_id;

    let _ = progress.send(ProgressPayload {
        task_id: task_id.clone(),
        phase: "scanning".to_string(),
        current_file: "Scanning input files...".to_string(),
        files_processed: 0,
        total_files: 0,
        bytes_processed: 0,
        total_bytes: 0,
        percent: 0.0,
        speed_mb_per_sec: 0.0,
    });

    // Find out at once if Windows lets 9-zip save in the chosen folder.
    check_output_folder(Path::new(&options.output_path))?;

    let (items, total_bytes) = scan_inputs(&options.input_paths)?;
    let total_files = items.len();

    match options.format {
        CompressionFormat::NineZ => {
            crate::engine::nine_z_format::compress_nine_z(
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
        }
        CompressionFormat::Zip => {
            compress_zip(&items, total_bytes, &options, &cancel_flag, progress)?;
        }
        CompressionFormat::SevenZ => {
            compress_7z(&items, total_bytes, &options, &cancel_flag, progress)?;
        }
        CompressionFormat::Tar => {
            let out_file = create_output(Path::new(&options.output_path))?;
            let mut tar = tar::Builder::new(BufWriter::new(out_file));
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let mut writer = tar.into_inner()?;
            writer.flush()?;
        }
        CompressionFormat::Gzip => {
            let out_file = create_output(Path::new(&options.output_path))?;
            let gz_level = match options.level {
                CompressionLevel::Store => flate2::Compression::none(),
                CompressionLevel::Fastest => flate2::Compression::fast(),
                CompressionLevel::Ultra => flate2::Compression::best(),
                _ => flate2::Compression::new(options.level.as_u32()),
            };
            let encoder = flate2::write::GzEncoder::new(BufWriter::new(out_file), gz_level);
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let encoder = tar.into_inner()?;
            encoder.finish()?;
        }
        CompressionFormat::Bzip2 => {
            let out_file = create_output(Path::new(&options.output_path))?;
            let bz_level = match options.level {
                CompressionLevel::Fastest => bzip2::Compression::fast(),
                CompressionLevel::Ultra => bzip2::Compression::best(),
                _ => bzip2::Compression::new(options.level.as_u32().clamp(1, 9)),
            };
            let encoder = bzip2::write::BzEncoder::new(BufWriter::new(out_file), bz_level);
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let encoder = tar.into_inner()?;
            encoder.finish()?;
        }
        CompressionFormat::Xz => {
            let out_file = create_output(Path::new(&options.output_path))?;
            // CRC64 check, like the xz tool, so Test can detect damage.
            let xz_options = lzma_rust2::XzOptions::with_preset(options.level.as_u32());
            let encoder = lzma_rust2::XzWriter::new(BufWriter::new(out_file), xz_options)
                .map_err(|e| AppError::new(format!("XZ encoder error: {}", e)))?;
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let encoder = tar.into_inner()?;
            encoder
                .finish()
                .map_err(|e| AppError::new(format!("XZ finish error: {}", e)))?
                .flush()?;
        }
        CompressionFormat::Zstd => {
            let out_file = create_output(Path::new(&options.output_path))?;
            let zstd_level = match options.level {
                CompressionLevel::Store => 1,
                CompressionLevel::Fastest => 1,
                CompressionLevel::Fast => 3,
                CompressionLevel::Normal => 7,
                CompressionLevel::Maximum => 14,
                CompressionLevel::Ultra => 19,
            };
            let mut encoder = zstd::stream::Encoder::new(BufWriter::new(out_file), zstd_level)
                .map_err(|e| AppError::new(format!("Zstandard encoder error: {}", e)))?;
            // Content checksum, so Test can detect damage.
            encoder
                .include_checksum(true)
                .map_err(|e| AppError::new(format!("Zstandard encoder error: {}", e)))?;
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let encoder = tar.into_inner()?;
            encoder
                .finish()
                .map_err(|e| AppError::new(format!("Zstandard finish error: {}", e)))?;
        }
        CompressionFormat::Lz4 => {
            let out_file = create_output(Path::new(&options.output_path))?;
            // Content checksum, so Test can detect damage.
            let frame_info = lz4_flex::frame::FrameInfo::new().content_checksum(true);
            let encoder = lz4_flex::frame::FrameEncoder::with_frame_info(
                frame_info,
                BufWriter::new(out_file),
            );
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let encoder = tar.into_inner()?;
            encoder
                .finish()
                .map_err(|e| AppError::new(format!("LZ4 finish error: {}", e)))?;
        }
        CompressionFormat::Brotli => {
            let out_file = create_output(Path::new(&options.output_path))?;
            let quality = match options.level {
                CompressionLevel::Store => 0,
                CompressionLevel::Fastest => 1,
                CompressionLevel::Fast => 4,
                CompressionLevel::Normal => 6,
                CompressionLevel::Maximum => 9,
                CompressionLevel::Ultra => 11,
            };
            let encoder =
                brotli::CompressorWriter::new(BufWriter::new(out_file), 4096, quality, 22);
            let mut tar = tar::Builder::new(encoder);
            write_tar_entries(
                &mut tar,
                &items,
                total_bytes,
                &options,
                &cancel_flag,
                progress,
            )?;
            let mut encoder = tar.into_inner()?;
            encoder.flush()?;
        }
    }

    let _ = progress.send(ProgressPayload {
        task_id: options.task_id.clone(),
        phase: "done".to_string(),
        current_file: "Archive created successfully".to_string(),
        files_processed: total_files,
        total_files,
        bytes_processed: total_bytes,
        total_bytes,
        percent: 100.0,
        speed_mb_per_sec: 0.0,
    });

    Ok(())
}

fn compress_zip(
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let file = create_output(Path::new(&options.output_path))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));

    let method = match options.level {
        CompressionLevel::Store => zip::CompressionMethod::Stored,
        _ => zip::CompressionMethod::Deflated,
    };

    let compression_level_opt = match options.level {
        CompressionLevel::Store => None,
        _ => Some(options.level.as_u32() as i64),
    };

    let mut base_opts = SimpleFileOptions::default()
        .compression_method(method)
        .compression_level(compression_level_opt);

    if let Some(pass) = options.password.as_deref().filter(|p| !p.is_empty()) {
        base_opts = base_opts.with_aes_encryption(zip::AesMode::Aes256, pass);
    }

    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_processed = 0u64;
    let total_files = items.len();

    let mut buf = [0u8; 64 * 1024];

    for (idx, item) in items.iter().enumerate() {
        if TaskManager::is_cancelled(cancel_flag) {
            let _ = fs::remove_file(&options.output_path);
            return Err(AppError::new("Compression cancelled by user"));
        }

        if item.is_dir {
            zip.add_directory(&item.archive_path, base_opts)?;
        } else {
            zip.start_file(&item.archive_path, base_opts)?;
            let mut f = open_source(&item.disk_path)?;
            loop {
                if TaskManager::is_cancelled(cancel_flag) {
                    let _ = fs::remove_file(&options.output_path);
                    return Err(AppError::new("Compression cancelled by user"));
                }
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buf[..n])?;
                bytes_processed += n as u64;

                if last_emit.elapsed().as_millis() >= 100 {
                    let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
                    let speed = (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed;
                    let percent = if total_bytes > 0 {
                        (bytes_processed as f32 / total_bytes as f32) * 100.0
                    } else {
                        (idx as f32 / total_files as f32) * 100.0
                    };

                    let _ = progress.send(ProgressPayload {
                        task_id: options.task_id.clone(),
                        phase: "compressing".to_string(),
                        current_file: item.archive_path.clone(),
                        files_processed: idx,
                        total_files,
                        bytes_processed,
                        total_bytes,
                        percent,
                        speed_mb_per_sec: speed,
                    });
                    last_emit = Instant::now();
                }
            }
        }
    }

    zip.finish()?;
    Ok(())
}

fn write_tar_entries<W: Write>(
    tar: &mut tar::Builder<W>,
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let start_time = Instant::now();
    let mut last_emit = Instant::now();
    let mut bytes_processed = 0u64;
    let total_files = items.len();

    for (idx, item) in items.iter().enumerate() {
        if TaskManager::is_cancelled(cancel_flag) {
            let _ = fs::remove_file(&options.output_path);
            return Err(AppError::new("Compression cancelled by user"));
        }

        if item.is_dir {
            tar.append_dir(&item.archive_path, &item.disk_path)?;
        } else {
            let mut file = open_source(&item.disk_path)?;
            tar.append_file(&item.archive_path, &mut file)?;
            bytes_processed += item.size;
        }

        if last_emit.elapsed().as_millis() >= 100 || idx == total_files - 1 {
            let elapsed = start_time.elapsed().as_secs_f32().max(0.001);
            let speed = (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed;
            let percent = if total_bytes > 0 {
                (bytes_processed as f32 / total_bytes as f32) * 100.0
            } else {
                (idx as f32 / total_files as f32) * 100.0
            };

            let _ = progress.send(ProgressPayload {
                task_id: options.task_id.clone(),
                phase: "compressing".to_string(),
                current_file: item.archive_path.clone(),
                files_processed: idx + 1,
                total_files,
                bytes_processed,
                total_bytes,
                percent,
                speed_mb_per_sec: speed,
            });
            last_emit = Instant::now();
        }
    }

    tar.finish()?;
    Ok(())
}

/// Shared progress state for the 7z writer, which pulls file data through
/// `TrackedFile` readers.
struct SevenZProgress<'a> {
    options: &'a CompressOptions,
    cancel_flag: &'a Arc<AtomicBool>,
    progress: &'a Channel<ProgressPayload>,
    total_files: usize,
    total_bytes: u64,
    files_done: usize,
    bytes_done: u64,
    start: Instant,
    last_emit: Option<Instant>,
}

impl SevenZProgress<'_> {
    fn emit(&mut self, name: &str) {
        if self
            .last_emit
            .is_some_and(|t| t.elapsed().as_millis() < 100)
        {
            return;
        }
        self.last_emit = Some(Instant::now());
        let elapsed = self.start.elapsed().as_secs_f32().max(0.001);
        let percent = if self.total_bytes > 0 {
            self.bytes_done as f32 / self.total_bytes as f32 * 100.0
        } else {
            self.files_done as f32 / self.total_files.max(1) as f32 * 100.0
        };
        let _ = self.progress.send(ProgressPayload {
            task_id: self.options.task_id.clone(),
            phase: "compressing".to_string(),
            current_file: name.to_string(),
            files_processed: self.files_done,
            total_files: self.total_files,
            bytes_processed: self.bytes_done,
            total_bytes: self.total_bytes,
            percent: percent.min(100.0),
            speed_mb_per_sec: (self.bytes_done as f32 / (1024.0 * 1024.0)) / elapsed,
        });
    }
}

/// Opens its file on first read and closes it at the end, so a solid block
/// of many files never holds more than one file handle. Reports progress and
/// stops with an error when the user cancels.
struct TrackedFile<'a> {
    path: PathBuf,
    name: String,
    file: Option<File>,
    finished: bool,
    state: Rc<RefCell<SevenZProgress<'a>>>,
}

impl Read for TrackedFile<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.finished {
            return Ok(0);
        }
        if TaskManager::is_cancelled(self.state.borrow().cancel_flag) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Compression cancelled by user",
            ));
        }
        if self.file.is_none() {
            let file = File::open(&self.path)
                .map_err(|e| std::io::Error::new(e.kind(), source_error(&self.path, &e).message))?;
            self.file = Some(file);
        }
        let n = match self.file.as_mut() {
            Some(file) => file.read(buf)?,
            None => 0,
        };
        let mut state = self.state.borrow_mut();
        if n == 0 {
            self.finished = true;
            self.file = None;
            state.files_done += 1;
        } else {
            state.bytes_done += n as u64;
        }
        state.emit(&self.name);
        Ok(n)
    }
}

/// Largest solid block, as in 7-Zip's default for LZMA2.
const SEVEN_Z_MAX_BLOCK: u64 = 4 * 1024 * 1024 * 1024;

fn compress_7z(
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    let result = write_7z(items, total_bytes, options, cancel_flag, progress);
    if result.is_err() {
        let _ = fs::remove_file(&options.output_path);
    }
    if TaskManager::is_cancelled(cancel_flag) {
        return Err(AppError::new("Compression cancelled by user"));
    }
    result
}

fn write_7z(
    items: &[InputFileItem],
    total_bytes: u64,
    options: &CompressOptions,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<(), AppError> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod};

    let err = |e: sevenz_rust2::Error| AppError::new(format!("7Z compression error: {}", e));
    let mut writer = ArchiveWriter::create(&options.output_path).map_err(err)?;

    let level = options.level.as_u32();
    let main_method: EncoderConfiguration = if options.level == CompressionLevel::Store {
        EncoderMethod::COPY.into()
    } else if options.threads > 1 {
        Lzma2Options::from_level_mt(level, options.threads as u32, 64 * 1024 * 1024).into()
    } else {
        Lzma2Options::from_level(level).into()
    };
    match options.password.as_deref().filter(|p| !p.is_empty()) {
        Some(password) => {
            writer.set_content_methods(vec![
                AesEncoderOptions::new(password.into()).into(),
                main_method,
            ]);
            // 7-Zip's "Encrypt file names".
            writer.set_encrypt_header(options.encrypt_names);
        }
        None => {
            writer.set_content_methods(vec![main_method]);
        }
    }

    let state = Rc::new(RefCell::new(SevenZProgress {
        options,
        cancel_flag,
        progress,
        total_files: items.len(),
        total_bytes,
        files_done: 0,
        bytes_done: 0,
        start: Instant::now(),
        last_emit: None,
    }));

    let mut entries = Vec::new();
    let mut readers = Vec::new();
    let mut block_size = 0u64;
    for item in items {
        if TaskManager::is_cancelled(cancel_flag) {
            return Err(AppError::new("Compression cancelled by user"));
        }
        let entry = ArchiveEntry::from_path(&item.disk_path, item.archive_path.clone());
        if item.is_dir {
            writer
                .push_archive_entry::<&[u8]>(entry, None)
                .map_err(err)?;
            state.borrow_mut().files_done += 1;
            continue;
        }
        if block_size > 0 && block_size + item.size >= SEVEN_Z_MAX_BLOCK {
            writer
                .push_archive_entries(std::mem::take(&mut entries), std::mem::take(&mut readers))
                .map_err(err)?;
            block_size = 0;
        }
        block_size += item.size;
        entries.push(entry);
        readers.push(
            TrackedFile {
                path: item.disk_path.clone(),
                name: item.archive_path.clone(),
                file: None,
                finished: false,
                state: Rc::clone(&state),
            }
            .into(),
        );
    }
    if !entries.is_empty() {
        writer.push_archive_entries(entries, readers).map_err(err)?;
    }
    writer
        .finish()
        .map_err(|e| AppError::new(format!("7Z compression error: {}", e)))?;
    Ok(())
}
