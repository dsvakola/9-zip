# 9-zip - Archive Manager for Windows

**9-zip** is a free archive manager for Windows that works like 7-Zip and adds its own self-healing **.9z** format.

Developed by **Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar**, for **Vidyasagar Academy**.
Official Website: [https://vsa.edu.in/](https://vsa.edu.in/)  
Software Catalog: [https://github.com/dsvakola/software](https://github.com/dsvakola/software)  
Repository: [https://github.com/dsvakola/9-zip](https://github.com/dsvakola/9-zip)  
Email: [softinquiry@vsa.edu.in](mailto:softinquiry@vsa.edu.in)

---

## Formats

9-zip creates, opens, extracts and tests these 10 formats:

| # | Format | Extension | Compression | Damage detection (Test) |
|---|---|---|---|---|
| 1 | **9Z** | `.9z` | Solid LZMA2, smart-adaptive | SHA-256 per file, CRC32 per block, **self-repair** |
| 2 | ZIP | `.zip` | Deflate, optional AES-256 password | CRC32 |
| 3 | 7Z | `.7z` | Solid LZMA2, optional AES-256 password | CRC32 |
| 4 | TAR | `.tar` | None (container) | Header checksums only |
| 5 | GZIP | `.tar.gz` | Deflate | CRC32 |
| 6 | BZIP2 | `.tar.bz2` | Burrows-Wheeler | Block CRCs |
| 7 | XZ | `.tar.xz` | LZMA2 | CRC64 |
| 8 | ZSTD | `.tar.zst` | Zstandard | Content checksum |
| 9 | LZ4 | `.tar.lz4` | LZ4 frame | Content checksum |
| 10 | BROTLI | `.tar.br` | Brotli | None (format has no checksum) |

Archives made by 9-zip in 7z, zip, tar, gz, bz2, xz and zst open in 7-Zip. Standard 7-Zip cannot open `.lz4` or `.br`.

---

## The .9z format

The .9z format brings three features together:

1. **Self-healing.** Reed-Solomon recovery data is stored inside the archive. When blocks are damaged (bad disk sectors, a broken download), 9-zip rebuilds them automatically while extracting. The amount is chosen in the Add to Archive window:
   - Low: 5% extra size
   - Normal: 10% extra size (default)
   - High: 20% extra size

   Recovery blocks are spread out, so one long damaged stretch (up to about 1 MB in every 10 MB at Normal) can be repaired. The header and the block index are each stored twice.
2. **Smart-adaptive compression.** Text, code and documents are packed together in solid LZMA2 blocks, sorted by type, as 7-Zip does. Photos, videos and other already-compressed files are stored as they are, so no time is wasted on them.
3. **VSA Integrity Seal.** Every file carries a SHA-256 fingerprint, checked whenever the archive is tested or extracted.

Compared with 7-Zip on the same folder (74 MB, 516 files), a Normal .9z was about 13% larger than 7-Zip's .7z: 10% of that is the recovery data. With 4 threads, 9-zip packed it in 12.5 s, against 15.2 s for 7-Zip.

Recipients need 9-zip to open .9z archives.

---

## Technology

- **Shell:** Tauri v2, using the WebView2 runtime built into Windows.
- **Backend:** Rust (stable, MSVC). All file and archive work runs on background threads with progress and cancel.
- **Frontend:** TypeScript (strict), Vite, plain HTML and CSS. Light theme.

---

## Building

```bash
npm ci
npx tauri build --bundles nsis
```

Then copy the results into `Release/`:

- `Release/9-zip.exe`: portable program
- `Release/9-zip_1.2.0_x64-setup.exe`: installer

---

## Licence and copyright

Copyright © 2026 Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

This program is free software: you can redistribute it and/or modify it under the terms of the **GNU General Public License** as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

See the [`LICENSE`](LICENSE) file for the full license text.
