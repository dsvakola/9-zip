//! The Test command: reads every file of an archive to the end without
//! writing anything, so each format's own checksums are verified (CRC32 for
//! ZIP, 7Z, GZIP and XZ, block CRCs for BZIP2, SHA-256 and Reed-Solomon
//! for 9Z). Like 7-Zip, it reports folders, files and sizes when all is well.

use crate::engine::extract::{open_7z, open_tar_stream};
use crate::engine::nine_z_format::{self, NineZHealth};
use crate::error::AppError;
use crate::models::{CompressionFormat, ProgressPayload};
use crate::task_manager::TaskManager;
use serde::Serialize;
use std::cell::Cell;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use zip::ZipArchive;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub folders: usize,
    pub files: usize,
    pub size: u64,
    pub packed_size: u64,
    /// Present for .9z archives only.
    pub nine_z: Option<NineZHealth>,
}

/// Counts how many bytes of the archive file have been read, for progress.
struct Counting<R> {
    inner: R,
    count: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.set(self.count.get() + n as u64);
        Ok(n)
    }
}

struct Meter<'a> {
    progress: &'a Channel<ProgressPayload>,
    task_id: &'a str,
    start: Instant,
    last: Option<Instant>,
}

impl Meter<'_> {
    fn emit(&mut self, name: &str, report: &TestReport, done: u64, total: u64) {
        if self
            .last
            .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        self.last = Some(Instant::now());
        let elapsed = self.start.elapsed().as_secs_f32().max(0.001);
        let _ = self.progress.send(ProgressPayload {
            task_id: self.task_id.to_string(),
            phase: "testing".to_string(),
            current_file: name.to_string(),
            files_processed: report.files,
            total_files: 0,
            bytes_processed: done,
            total_bytes: total,
            percent: if total > 0 {
                (done as f32 / total as f32 * 100.0).min(100.0)
            } else {
                0.0
            },
            speed_mb_per_sec: (done as f32 / (1024.0 * 1024.0)) / elapsed,
        });
    }
}

fn check_cancel(cancel_flag: &Arc<AtomicBool>) -> Result<(), AppError> {
    if TaskManager::is_cancelled(cancel_flag) {
        Err(AppError::new("Test cancelled by user"))
    } else {
        Ok(())
    }
}

fn damaged(name: &str, err: impl std::fmt::Display) -> AppError {
    AppError::new(format!("Data error in '{}': {}", name, err))
}

pub fn test_archive(
    archive_path: &str,
    password: Option<&str>,
    task_id: &str,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<TestReport, AppError> {
    let path = Path::new(archive_path);
    let format = CompressionFormat::from_path(archive_path)
        .ok_or_else(|| AppError::new(format!("Unsupported archive format: {}", archive_path)))?;
    let packed_size = path.metadata()?.len();
    let mut meter = Meter {
        progress,
        task_id,
        start: Instant::now(),
        last: None,
    };

    let mut report = match format {
        CompressionFormat::NineZ => {
            let health = nine_z_format::test_nine_z(path, task_id, cancel_flag, progress)?;
            let mut report = TestReport::default();
            for entry in nine_z_format::read_manifest_nine_z(path)?.entries {
                if entry.is_dir {
                    report.folders += 1;
                } else {
                    report.files += 1;
                    report.size += entry.uncompressed_size;
                }
            }
            report.nine_z = Some(health);
            report
        }
        CompressionFormat::Zip => test_zip(path, password, cancel_flag, &mut meter, packed_size)?,
        CompressionFormat::SevenZ => test_7z(archive_path, password, cancel_flag, &mut meter)?,
        tar_family => test_tar(path, tar_family, cancel_flag, &mut meter, packed_size)?,
    };
    report.packed_size = packed_size;
    Ok(report)
}

fn test_zip(
    path: &Path,
    password: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
    meter: &mut Meter,
    packed_size: u64,
) -> Result<TestReport, AppError> {
    let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
    let mut report = TestReport::default();
    for i in 0..archive.len() {
        check_cancel(cancel_flag)?;
        let mut entry = match password {
            Some(p) if !p.is_empty() => archive.by_index_decrypt(i, p.as_bytes())?,
            _ => archive.by_index(i)?,
        };
        let name = entry.name().to_string();
        if entry.is_dir() {
            report.folders += 1;
            continue;
        }
        // Reading to the end makes the zip crate compare the CRC32.
        report.size += io::copy(&mut entry, &mut io::sink()).map_err(|e| damaged(&name, e))?;
        report.files += 1;
        let done = entry.data_start() + entry.compressed_size();
        meter.emit(&name, &report, done, packed_size);
    }
    Ok(report)
}

fn test_7z(
    archive_path: &str,
    password: Option<&str>,
    cancel_flag: &Arc<AtomicBool>,
    meter: &mut Meter,
) -> Result<TestReport, AppError> {
    let mut reader = open_7z(archive_path, password)?;
    let total: u64 = reader.archive().files.iter().map(|f| f.size()).sum();
    let mut report = TestReport::default();
    let mut failure: Option<AppError> = None;

    let result = reader.for_each_entries(|entry, data| {
        if let Err(e) = check_cancel(cancel_flag) {
            failure = Some(e);
            return Ok(false);
        }
        if entry.is_directory() {
            report.folders += 1;
            return Ok(true);
        }
        match io::copy(data, &mut io::sink()) {
            Ok(n) => report.size += n,
            Err(e) => {
                failure = Some(damaged(entry.name(), e));
                return Ok(false);
            }
        }
        report.files += 1;
        meter.emit(entry.name(), &report, report.size, total);
        Ok(true)
    });

    if let Some(e) = failure {
        // With a wrong password, AES gives garbage that the decoder rejects.
        let with_password = password.is_some_and(|p| !p.is_empty());
        let cancelled = e.message.contains("cancelled");
        return Err(if with_password && e.code.is_none() && !cancelled {
            AppError::wrong_password()
        } else {
            e
        });
    }
    result.map_err(AppError::from)?;
    Ok(report)
}

fn test_tar(
    path: &Path,
    format: CompressionFormat,
    cancel_flag: &Arc<AtomicBool>,
    meter: &mut Meter,
    packed_size: u64,
) -> Result<TestReport, AppError> {
    let count = Rc::new(Cell::new(0u64));
    let source = Counting {
        inner: File::open(path)?,
        count: Rc::clone(&count),
    };
    let mut archive = tar::Archive::new(open_tar_stream(format, source)?);
    let mut report = TestReport::default();

    for entry in archive.entries().map_err(|e| damaged("archive", e))? {
        check_cancel(cancel_flag)?;
        let mut entry = entry.map_err(|e| damaged("archive", e))?;
        let name = entry
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            report.folders += 1;
            continue;
        }
        report.size += io::copy(&mut entry, &mut io::sink()).map_err(|e| damaged(&name, e))?;
        if kind.is_file() {
            report.files += 1;
        }
        meter.emit(&name, &report, count.get(), packed_size);
    }

    // TAR stops at its end marker; read on so the compression layer reaches
    // its trailer and checks it (the CRC32 of GZIP, for example).
    let mut rest = archive.into_inner();
    io::copy(&mut rest, &mut io::sink()).map_err(|e| damaged("archive end", e))?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compress::compress_archive;
    use crate::models::{CompressOptions, CompressionLevel};
    use std::fs;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn channel() -> Channel<ProgressPayload> {
        Channel::new(|_| Ok(()))
    }

    const FORMATS: [(CompressionFormat, &str); 10] = [
        (CompressionFormat::NineZ, "t.9z"),
        (CompressionFormat::Zip, "t.zip"),
        (CompressionFormat::SevenZ, "t.7z"),
        (CompressionFormat::Tar, "t.tar"),
        (CompressionFormat::Gzip, "t.tar.gz"),
        (CompressionFormat::Bzip2, "t.tar.bz2"),
        (CompressionFormat::Xz, "t.tar.xz"),
        (CompressionFormat::Zstd, "t.tar.zst"),
        (CompressionFormat::Lz4, "t.tar.lz4"),
        (CompressionFormat::Brotli, "t.tar.br"),
    ];

    fn build(dir: &Path, format: CompressionFormat, name: &str) -> PathBuf {
        let src = dir.join("src");
        if !src.exists() {
            fs::create_dir_all(src.join("sub")).expect("mkdir");
            fs::write(src.join("a.txt"), "alpha line\n".repeat(5000)).expect("write");
            fs::write(src.join("sub").join("b.txt"), "beta line\n".repeat(3000)).expect("write");
        }
        let out = dir.join(name);
        compress_archive(
            CompressOptions {
                input_paths: vec![src.to_string_lossy().to_string()],
                output_path: out.to_string_lossy().to_string(),
                format,
                level: CompressionLevel::Normal,
                password: None,
                threads: 1,
                // Unique per test: the 7z writer stages files in a folder
                // named after the task id.
                task_id: format!("{}", out.display()).replace(['\\', ':', '/', '.'], "_"),
                recovery: Default::default(),
                encrypt_names: true,
            },
            Arc::new(AtomicBool::new(false)),
            &channel(),
        )
        .expect("compress");
        out
    }

    fn run(path: &Path) -> Result<TestReport, AppError> {
        test_archive(
            &path.to_string_lossy(),
            None,
            "t",
            &Arc::new(AtomicBool::new(false)),
            &channel(),
        )
    }

    #[test]
    fn every_format_passes_when_intact() {
        let dir = TempDir(std::env::temp_dir().join("9z_test_cmd_ok"));
        let _ = fs::remove_dir_all(&dir.0);
        for (format, name) in FORMATS {
            let report = run(&build(&dir.0, format, name))
                .unwrap_or_else(|e| panic!("{:?} failed: {}", format, e));
            assert_eq!(report.files, 2, "{:?} file count", format);
            assert_eq!(report.size, 11 * 5000 + 10 * 3000, "{:?} size", format);
            assert!(report.folders >= 1, "{:?} folder count", format);
            assert_eq!(report.nine_z.is_some(), format == CompressionFormat::NineZ);
        }
    }

    /// Writes one archive per format into `NINEZ_SAMPLE_DIR`, packing
    /// `NINEZ_SAMPLE_SRC`, so they can be checked with other tools such as
    /// `7z t`. Run with `cargo test -- --ignored write_samples`.
    #[test]
    #[ignore]
    fn write_samples() {
        let (Ok(out), Ok(src)) = (
            std::env::var("NINEZ_SAMPLE_DIR"),
            std::env::var("NINEZ_SAMPLE_SRC"),
        ) else {
            return;
        };
        fs::create_dir_all(&out).expect("mkdir");
        // Optional: NINEZ_SAMPLE_ONLY=9z and NINEZ_SAMPLE_THREADS=4, for timing.
        let only = std::env::var("NINEZ_SAMPLE_ONLY").ok();
        let threads = std::env::var("NINEZ_SAMPLE_THREADS")
            .ok()
            .and_then(|t| t.parse().ok())
            .unwrap_or(1);
        for (format, name) in FORMATS {
            if only.as_deref().is_some_and(|o| !name.ends_with(o)) {
                continue;
            }
            let started = std::time::Instant::now();
            let dest = Path::new(&out).join(name.replace("t.", "sample."));
            compress_archive(
                CompressOptions {
                    input_paths: vec![src.clone()],
                    output_path: dest.to_string_lossy().to_string(),
                    format,
                    level: CompressionLevel::Normal,
                    password: None,
                    threads,
                    task_id: format!("sample_{:?}", format),
                    recovery: Default::default(),
                    encrypt_names: true,
                },
                Arc::new(AtomicBool::new(false)),
                &channel(),
            )
            .expect("compress");
            println!("{:?}: {:.1} s", format, started.elapsed().as_secs_f32());
        }
    }

    #[test]
    fn extracts_only_the_selection_in_every_format() {
        let dir = TempDir(std::env::temp_dir().join("9z_test_cmd_pick"));
        let _ = fs::remove_dir_all(&dir.0);
        for (format, name) in FORMATS {
            let archive = build(&dir.0, format, name);
            let dest = dir.0.join(format!("out_{:?}", format));
            crate::engine::extract::extract_archive(
                crate::models::ExtractOptions {
                    archive_path: archive.to_string_lossy().to_string(),
                    destination: dest.to_string_lossy().to_string(),
                    password: None,
                    overwrite: true,
                    task_id: format!("pick_{:?}", format),
                    entries: Some(vec!["src/sub".to_string()]),
                },
                Arc::new(AtomicBool::new(false)),
                &channel(),
            )
            .unwrap_or_else(|e| panic!("{:?} failed: {}", format, e));
            assert!(
                dest.join("src").join("sub").join("b.txt").exists(),
                "{:?}: selected file missing",
                format
            );
            assert!(
                !dest.join("src").join("a.txt").exists(),
                "{:?}: file outside the selection was extracted",
                format
            );
        }
    }

    /// ZIP and 7Z with a password: no password or a wrong one gives an error
    /// code the window turns into a password prompt; the right one works.
    #[test]
    fn passwords_are_asked_for_and_checked() {
        let dir = TempDir(std::env::temp_dir().join("9z_test_cmd_password"));
        let _ = fs::remove_dir_all(&dir.0);
        fs::create_dir_all(dir.0.join("src")).expect("mkdir");
        fs::write(
            dir.0.join("src").join("secret.txt"),
            "top secret\n".repeat(500),
        )
        .expect("write");

        for (format, name, names_hidden) in [
            (CompressionFormat::Zip, "locked.zip", false),
            (CompressionFormat::SevenZ, "locked.7z", true),
        ] {
            let archive = dir.0.join(name);
            compress_archive(
                CompressOptions {
                    input_paths: vec![dir.0.join("src").to_string_lossy().to_string()],
                    output_path: archive.to_string_lossy().to_string(),
                    format,
                    level: CompressionLevel::Normal,
                    password: Some("Sesame-9".to_string()),
                    threads: 1,
                    task_id: format!("pw_{:?}", format),
                    recovery: Default::default(),
                    encrypt_names: true,
                },
                Arc::new(AtomicBool::new(false)),
                &channel(),
            )
            .expect("compress");
            let path = archive.to_string_lossy().to_string();

            // Listing: ZIP shows names without a password; 7Z hides them.
            let listed = crate::engine::list::list_archive_contents(&path, None);
            assert_eq!(listed.is_err(), names_hidden, "{:?} listing", format);
            if names_hidden {
                assert_eq!(listed.expect_err("hidden").code, Some("password_required"));
                crate::engine::list::list_archive_contents(&path, Some("Sesame-9".into()))
                    .expect("list with password");
            }

            let extract = |password: Option<&str>| {
                crate::engine::extract::extract_archive(
                    crate::models::ExtractOptions {
                        archive_path: path.clone(),
                        destination: dir
                            .0
                            .join(format!("out_{:?}", format))
                            .to_string_lossy()
                            .to_string(),
                        password: password.map(String::from),
                        overwrite: true,
                        task_id: format!("pwx_{:?}", format),
                        entries: None,
                    },
                    Arc::new(AtomicBool::new(false)),
                    &channel(),
                )
            };
            assert_eq!(
                extract(None).expect_err("needs a password").code,
                Some("password_required"),
                "{:?} without password",
                format
            );
            assert_eq!(
                extract(Some("wrong")).expect_err("wrong password").code,
                Some("wrong_password"),
                "{:?} with a wrong password",
                format
            );
            extract(Some("Sesame-9")).unwrap_or_else(|e| panic!("{:?}: {}", format, e));
            let text = fs::read_to_string(
                dir.0
                    .join(format!("out_{:?}", format))
                    .join("src")
                    .join("secret.txt"),
            )
            .expect("read");
            assert!(text.starts_with("top secret"));
        }
    }

    /// The Professor's test: a ZIP with one damaged file. The other files
    /// come out complete, and no half-written file is left behind.
    #[test]
    fn damaged_zip_extracts_the_good_files() {
        let dir = TempDir(std::env::temp_dir().join("9z_test_cmd_damaged_zip"));
        let _ = fs::remove_dir_all(&dir.0);
        let archive = build(&dir.0, CompressionFormat::Zip, "t.zip");
        let mut zip = ZipArchive::new(File::open(&archive).expect("open")).expect("zip");
        let a = (0..zip.len())
            .find(|&i| zip.by_index_raw(i).expect("entry").name().ends_with("a.txt"))
            .expect("a.txt");
        let at = zip.by_index_raw(a).expect("entry").data_start() as usize + 4;
        drop(zip);
        let mut bytes = fs::read(&archive).expect("read");
        for b in &mut bytes[at..at + 64] {
            *b = !*b;
        }
        fs::write(&archive, bytes).expect("write");

        let out = dir.0.join("out");
        let err = crate::engine::extract::extract_archive(
            crate::models::ExtractOptions {
                archive_path: archive.to_string_lossy().to_string(),
                destination: out.to_string_lossy().to_string(),
                password: None,
                overwrite: true,
                task_id: "damaged_zip".to_string(),
                entries: None,
            },
            Arc::new(AtomicBool::new(false)),
            &channel(),
        )
        .expect_err("must report the damage");
        assert_eq!(err.code, Some("partly_damaged"), "message: {}", err.message);
        assert!(err.message.contains("a.txt"));
        assert_eq!(
            fs::read_to_string(out.join("src").join("sub").join("b.txt")).expect("b.txt"),
            "beta line\n".repeat(3000)
        );
        assert!(!out.join("src").join("a.txt").exists(), "no broken file is left");
        assert!(!out.join("src").join("a.txt.9z-part").exists(), "no temporary file is left");
    }

    #[test]
    fn damage_is_reported_for_checksummed_formats() {
        let dir = TempDir(std::env::temp_dir().join("9z_test_cmd_bad"));
        let _ = fs::remove_dir_all(&dir.0);
        // Formats whose data carries a checksum. Plain TAR and BROTLI have
        // none, so they cannot detect every change by design.
        let checked = [
            (CompressionFormat::Zip, "t.zip"),
            (CompressionFormat::SevenZ, "t.7z"),
            (CompressionFormat::Gzip, "t.tar.gz"),
            (CompressionFormat::Bzip2, "t.tar.bz2"),
            (CompressionFormat::Xz, "t.tar.xz"),
            (CompressionFormat::Zstd, "t.tar.zst"),
            (CompressionFormat::Lz4, "t.tar.lz4"),
        ];
        for (format, name) in checked {
            let path = build(&dir.0, format, name);
            // Damage the compressed file data itself. (ZIP and 7Z keep their
            // index at the end, where the middle of a small archive may fall.)
            let at = match format {
                CompressionFormat::Zip => {
                    let mut zip = ZipArchive::new(File::open(&path).expect("open")).expect("zip");
                    let a = (0..zip.len())
                        .find(|&i| {
                            zip.by_index_raw(i)
                                .expect("entry")
                                .name()
                                .ends_with("a.txt")
                        })
                        .expect("a.txt");
                    zip.by_index_raw(a).expect("entry").data_start() as usize + 4
                }
                CompressionFormat::SevenZ => 32 + 4, // packed data follows the 32-byte signature header
                _ => fs::metadata(&path).expect("meta").len() as usize / 2,
            };
            let mut bytes = fs::read(&path).expect("read");
            for b in &mut bytes[at..at + 8] {
                *b = !*b;
            }
            fs::write(&path, bytes).expect("write");
            assert!(run(&path).is_err(), "{:?} damage went unnoticed", format);
        }
    }
}
