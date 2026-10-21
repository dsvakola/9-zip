use crate::error::AppError;
use crate::models::{HashResult, ProgressPayload};
use crate::task_manager::TaskManager;
use crc32fast::Hasher as CrcHasher;
use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tauri::ipc::Channel;

pub fn calculate_file_hashes(
    file_path: &str,
    task_id: &str,
    cancel_flag: Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<HashResult, AppError> {
    let path = Path::new(file_path);
    if !path.is_file() {
        return Err(AppError::new(format!(
            "Path is not a valid file: {}",
            file_path
        )));
    }

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.to_string());

    let mut file = File::open(path)
        .map_err(|e| AppError::new(format!("Cannot open file for hashing: {}", e)))?;
    let total_bytes = file.metadata().map(|m| m.len()).unwrap_or(0);

    let mut crc = CrcHasher::new();
    let mut md5 = Md5::new();
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();

    let mut buffer = [0u8; 128 * 1024]; // 128 KB buffer
    let mut bytes_processed = 0u64;
    let start_time = Instant::now();
    let mut last_emit = Instant::now();

    loop {
        if TaskManager::is_cancelled(&cancel_flag) {
            let _ = progress.send(ProgressPayload {
                task_id: task_id.to_string(),
                phase: "cancelled".to_string(),
                current_file: file_name.clone(),
                files_processed: 0,
                total_files: 1,
                bytes_processed,
                total_bytes,
                percent: 0.0,
                speed_mb_per_sec: 0.0,
            });
            return Err(AppError::new("Hash calculation was cancelled by user"));
        }

        let n = file
            .read(&mut buffer)
            .map_err(|e| AppError::new(format!("Read error while hashing: {}", e)))?;
        if n == 0 {
            break;
        }

        let chunk = &buffer[..n];
        crc.update(chunk);
        md5.update(chunk);
        sha1.update(chunk);
        sha256.update(chunk);

        bytes_processed += n as u64;

        if last_emit.elapsed().as_millis() >= 100 || bytes_processed == total_bytes {
            let elapsed_sec = start_time.elapsed().as_secs_f32().max(0.001);
            let speed_mb = (bytes_processed as f32 / (1024.0 * 1024.0)) / elapsed_sec;
            let percent = if total_bytes > 0 {
                (bytes_processed as f32 / total_bytes as f32) * 100.0
            } else {
                100.0
            };

            let _ = progress.send(ProgressPayload {
                task_id: task_id.to_string(),
                phase: "hashing".to_string(),
                current_file: file_name.clone(),
                files_processed: if bytes_processed == total_bytes { 1 } else { 0 },
                total_files: 1,
                bytes_processed,
                total_bytes,
                percent,
                speed_mb_per_sec: speed_mb,
            });
            last_emit = Instant::now();
        }
    }

    let crc_val = format!("{:08X}", crc.finalize());
    let md5_val = format!("{:x}", md5.finalize());
    let sha1_val = format!("{:x}", sha1.finalize());
    let sha256_val = format!("{:x}", sha256.finalize());

    let _ = progress.send(ProgressPayload {
        task_id: task_id.to_string(),
        phase: "done".to_string(),
        current_file: file_name.clone(),
        files_processed: 1,
        total_files: 1,
        bytes_processed,
        total_bytes,
        percent: 100.0,
        speed_mb_per_sec: 0.0,
    });

    Ok(HashResult {
        path: file_path.to_string(),
        file_name,
        file_size: total_bytes,
        crc32: crc_val,
        md5: md5_val,
        sha1: sha1_val,
        sha256: sha256_val,
    })
}
