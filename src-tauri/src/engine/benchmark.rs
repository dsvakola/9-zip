use crate::error::AppError;
use crate::models::{BenchmarkResult, ProgressPayload};
use crate::task_manager::TaskManager;
use std::io::{Cursor, Read, Write};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tauri::ipc::Channel;

pub fn run_cpu_benchmark(
    threads_count: usize,
    task_id: &str,
    cancel_flag: Arc<AtomicBool>,
    progress: &Channel<ProgressPayload>,
) -> Result<BenchmarkResult, AppError> {
    let threads = if threads_count == 0 {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else {
        threads_count
    };

    // Create 16 MB of semi-compressible benchmark data
    let block_size = 16 * 1024 * 1024;
    let mut data = Vec::with_capacity(block_size);
    for i in 0..block_size {
        data.push(((i * 7) ^ (i / 13) ^ (i % 251)) as u8);
    }

    let _ = progress.send(ProgressPayload {
        task_id: task_id.to_string(),
        phase: "benchmark_compressing".to_string(),
        current_file: format!("Compressing with {} threads...", threads),
        files_processed: 0,
        total_files: 2,
        bytes_processed: 0,
        total_bytes: block_size as u64,
        percent: 10.0,
        speed_mb_per_sec: 0.0,
    });

    if TaskManager::is_cancelled(&cancel_flag) {
        return Err(AppError::new("Benchmark cancelled"));
    }

    // Benchmark Compression (using ZSTD Level 3 for standardized test)
    let compress_start = Instant::now();
    let mut compressed = Vec::new();
    {
        let mut encoder = zstd::stream::Encoder::new(&mut compressed, 3)
            .map_err(|e| AppError::new(format!("Benchmark encoder error: {}", e)))?;
        encoder
            .write_all(&data)
            .map_err(|e| AppError::new(format!("Benchmark write error: {}", e)))?;
        encoder
            .finish()
            .map_err(|e| AppError::new(format!("Benchmark finish error: {}", e)))?;
    }
    let compress_dur = compress_start.elapsed().as_secs_f32().max(0.001);
    let compress_mb_sec = (block_size as f32 / (1024.0 * 1024.0)) / compress_dur;
    let compress_mips = (compress_mb_sec * 31.5) as u64;

    let _ = progress.send(ProgressPayload {
        task_id: task_id.to_string(),
        phase: "benchmark_decompressing".to_string(),
        current_file: format!("Decompressing with {} threads...", threads),
        files_processed: 1,
        total_files: 2,
        bytes_processed: block_size as u64,
        total_bytes: block_size as u64,
        percent: 60.0,
        speed_mb_per_sec: compress_mb_sec,
    });

    if TaskManager::is_cancelled(&cancel_flag) {
        return Err(AppError::new("Benchmark cancelled"));
    }

    // Benchmark Decompression
    let decompress_start = Instant::now();
    let mut decompressed = Vec::with_capacity(block_size);
    {
        let mut decoder = zstd::stream::Decoder::new(Cursor::new(&compressed))
            .map_err(|e| AppError::new(format!("Benchmark decoder error: {}", e)))?;
        decoder
            .read_to_end(&mut decompressed)
            .map_err(|e| AppError::new(format!("Benchmark read error: {}", e)))?;
    }
    let decompress_dur = decompress_start.elapsed().as_secs_f32().max(0.001);
    let decompress_mb_sec = (block_size as f32 / (1024.0 * 1024.0)) / decompress_dur;
    let decompress_mips = (decompress_mb_sec * 8.5) as u64;

    let total_rating = (compress_mips + decompress_mips) / 2;

    let _ = progress.send(ProgressPayload {
        task_id: task_id.to_string(),
        phase: "done".to_string(),
        current_file: "Benchmark completed".to_string(),
        files_processed: 2,
        total_files: 2,
        bytes_processed: block_size as u64,
        total_bytes: block_size as u64,
        percent: 100.0,
        speed_mb_per_sec: decompress_mb_sec,
    });

    Ok(BenchmarkResult {
        threads,
        compress_speed_mb_per_sec: compress_mb_sec,
        compress_mips,
        decompress_speed_mb_per_sec: decompress_mb_sec,
        decompress_mips,
        total_rating_mips: total_rating,
    })
}
