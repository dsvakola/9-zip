#[cfg(test)]
mod tests {
    use crate::engine::{benchmark, compress, extract, hasher, list};
    use crate::models::{
        CompressOptions, CompressionFormat, CompressionLevel, ExtractOptions, ProgressPayload,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use tauri::ipc::Channel;

    fn setup_test_files(dir: &Path) -> (Vec<String>, PathBuf) {
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("file1.txt"),
            b"Hello from 9z! This is file 1 with text content.",
        )
        .unwrap();
        fs::write(
            src.join("file2.json"),
            b"{\"app\": \"9z\", \"version\": \"1.0.0\", \"popular\": true}",
        )
        .unwrap();

        let sub = src.join("subfolder");
        fs::create_dir_all(&sub).unwrap();
        fs::write(
            sub.join("nested.txt"),
            b"Nested content inside 9z test archive.",
        )
        .unwrap();

        (vec![src.to_string_lossy().to_string()], src)
    }

    fn dummy_channel() -> Channel<ProgressPayload> {
        Channel::new(|_| Ok(()))
    }

    #[test]
    fn test_all_9_formats() {
        let base_temp = std::env::temp_dir().join("9z_test_suite");
        let _ = fs::remove_dir_all(&base_temp);
        fs::create_dir_all(&base_temp).unwrap();

        let (inputs, _src) = setup_test_files(&base_temp);

        let formats = [
            (CompressionFormat::NineZ, "test.9z"),
            (CompressionFormat::Zip, "test.zip"),
            (CompressionFormat::SevenZ, "test.7z"),
            (CompressionFormat::Tar, "test.tar"),
            (CompressionFormat::Gzip, "test.tar.gz"),
            (CompressionFormat::Bzip2, "test.tar.bz2"),
            (CompressionFormat::Xz, "test.tar.xz"),
            (CompressionFormat::Zstd, "test.tar.zst"),
            (CompressionFormat::Lz4, "test.tar.lz4"),
            (CompressionFormat::Brotli, "test.tar.br"),
        ];

        let channel = dummy_channel();

        for (fmt, filename) in formats {
            let archive_path = base_temp.join(filename);
            let cancel_flag = Arc::new(AtomicBool::new(false));

            // 1. Compress
            let comp_opts = CompressOptions {
                input_paths: inputs.clone(),
                output_path: archive_path.to_string_lossy().to_string(),
                format: fmt,
                level: CompressionLevel::Normal,
                password: None,
                threads: 2,
                task_id: format!("test_comp_{:?}", fmt),
                recovery: Default::default(),
                encrypt_names: true,
            };

            compress::compress_archive(comp_opts, cancel_flag.clone(), &channel)
                .unwrap_or_else(|e| panic!("Compression failed for {:?}: {}", fmt, e));
            assert!(
                archive_path.exists(),
                "Archive file must exist for {:?}",
                fmt
            );
            assert!(
                archive_path.metadata().unwrap().len() > 0,
                "Archive cannot be empty for {:?}",
                fmt
            );

            // 2. List
            let entries = list::list_archive_contents(&archive_path.to_string_lossy(), None)
                .unwrap_or_else(|e| panic!("Listing failed for {:?}: {}", fmt, e));
            assert!(
                !entries.is_empty(),
                "Archive entries must not be empty for {:?}",
                fmt
            );

            // 3. Extract
            let extract_dest = base_temp.join(format!("extracted_{:?}", fmt));
            let ext_opts = ExtractOptions {
                archive_path: archive_path.to_string_lossy().to_string(),
                destination: extract_dest.to_string_lossy().to_string(),
                password: None,
                overwrite: true,
                task_id: format!("test_ext_{:?}", fmt),
                entries: None,
            };

            extract::extract_archive(ext_opts, cancel_flag, &channel)
                .unwrap_or_else(|e| panic!("Extraction failed for {:?}: {}", fmt, e));

            // Verify extracted files
            assert!(
                extract_dest.exists(),
                "Extract dest must exist for {:?}",
                fmt
            );
        }

        // Test Hasher
        let test_file = base_temp.join("hash_target.txt");
        fs::write(&test_file, b"Vidyasagar Academy - 9z Archiver").unwrap();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let hash_result = hasher::calculate_file_hashes(
            &test_file.to_string_lossy(),
            "hash_task",
            cancel_flag.clone(),
            &channel,
        )
        .expect("Hash calculation failed");

        assert_eq!(
            hash_result.file_size,
            b"Vidyasagar Academy - 9z Archiver".len() as u64
        );
        assert!(!hash_result.crc32.is_empty());
        assert!(!hash_result.sha256.is_empty());

        // Test Benchmark
        let bench_result = benchmark::run_cpu_benchmark(2, "bench_task", cancel_flag, &channel)
            .expect("Benchmark failed");
        assert!(bench_result.compress_speed_mb_per_sec > 0.0);
        assert!(bench_result.total_rating_mips > 0);

        let _ = fs::remove_dir_all(&base_temp);
    }
}
