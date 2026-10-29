# 9-zip: Microsoft Store submission kit

Everything to paste into Partner Center, in the order Partner Center asks for it.

---

## 1. Product identity (Partner Center gives these values)

Reserved name: **9-zip Archiver** ("9-zip" was taken). Store ID 9N7VCJRQJQ3W, PFN VidyasagarAcademy.9-zipArchiver_z3tqm701my9nc. After reserving, open *Product management → Product identity* and copy these three values into `msix/store-identity.json`:

| Partner Center field | Goes into |
|---|---|
| Package/Identity/Name | `identityName` |
| Package/Identity/Publisher | `publisher` |
| Package/Properties/PublisherDisplayName | `publisherDisplayName` |

Then rebuild the package:

```
npx tauri build --bundles nsis
powershell -ExecutionPolicy Bypass -File msix\build-msix.ps1
```

Upload `Release\9-zip-Archiver_<version>.0_x64.msix` (1.2.0.0 was submitted on 29 October 2026). The values are already filled in `msix/store-identity.json`.

---

## 2. Pricing and availability

- **Price:** Free
- **Markets:** all markets
- **Visibility:** Public

## 3. Properties

- **Category:** Utilities & tools
- **Privacy policy URL:** https://github.com/dsvakola/9-zip/blob/main/PRIVACY.md
- **Website:** https://vsa.edu.in/software/
- **Support contact:** softinquiry@vsa.edu.in
- **Product declarations:**
  - This app does not collect, store or transmit personal information: **yes**.
  - The app depends on non-Microsoft drivers or NT services: **no**.
  - The app has been tested to meet accessibility guidelines: leave **unticked** until it has been tested.

## 4. Age ratings (IARC questionnaire)

Answer **No** to every content question: no violence, no sexual content, no gambling, no user-to-user communication, no sharing of user location, no purchases. 9-zip only reads and writes files the user chooses. The expected rating is **3+ / Everyone**.

## 5. Submission options: notes for certification

Paste into *Notes for certification*:

> 9-zip is an archive manager, like 7-Zip. It needs **runFullTrust** because it is a desktop (Win32) app that must read and write files and folders anywhere the user chooses, such as when creating and extracting archives. It does not run any background services, does not connect to the internet, and does not collect data.
>
> The package includes `ninezip_shell.dll`, a COM `IExplorerCommand` handler declared in the manifest (`windows.fileExplorerContextMenus` and `windows.comServer`). It adds a "9-zip" submenu to the Explorer right-click menu with Add to archive, Extract here, Extract to folder and Test archive. Each command only starts 9-zip.exe with the selected paths.
>
> To test: open 9-zip, select a folder, click **Add**, keep the format **9z**, and click **OK**. Then select the new .9z file and click **Test**. To see self-repair, open the .9z in a hex editor, change a few bytes in the middle, save it, and click **Test** again. 9-zip reports the repaired blocks.

**Restricted capability justification** (Partner Center asks for this separately for runFullTrust):

> 9-zip is a Win32 desktop archive manager. Full trust is required for unrestricted access to user-selected files and folders when creating, testing and extracting archives, and for the Explorer context menu handler.

---

## 6. Store listing (English)

### Product name
9-zip Archiver

### Short description (up to 270 characters)
Free archive manager for Windows that works like 7-Zip, with its own self-healing .9z format. Create, open, test and extract 9Z, 7Z, ZIP, TAR, GZIP, BZIP2, XZ, ZSTD, LZ4 and BROTLI archives.

### Description

9-zip is a free archive manager for Windows. If you know 7-Zip, you already know 9-zip: the same file manager window, the same menus and keyboard shortcuts, and the same right-click menu in File Explorer.

9-zip also introduces .9z, an archive format that brings three features together:

**Self-healing.** A .9z archive carries Reed-Solomon recovery data. When parts of the archive are damaged, for example by a bad disk sector or a broken download, 9-zip rebuilds them automatically while extracting. You choose how much recovery data to add: Low (5%), Normal (10%) or High (20%).

**Smart-adaptive compression.** Text, code and documents are packed together with LZMA2, the method 7z uses, for a small archive. Photos, videos and other already-compressed files are stored as they are, so no time is spent trying to shrink them.

**VSA Integrity Seal.** Every file in a .9z archive carries a SHA-256 fingerprint. Testing or extracting checks every file against it, so damage never goes unnoticed.

9-zip creates, opens, tests and extracts ten formats: 9Z, 7Z, ZIP, TAR, GZIP, BZIP2, XZ, ZSTD, LZ4 and BROTLI. ZIP and 7Z archives can be protected with an AES-256 password.

Recipients need 9-zip to open .9z archives. Archives in the other formats open in other archive programs too.

Developed by Prof. Dattaraj S. Vidyasagar and Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

### Product features (one per line, up to 20)
- Works like 7-Zip: file manager window, menus and keyboard shortcuts
- Self-healing .9z format with Reed-Solomon recovery data
- Choose recovery data: Low 5%, Normal 10% or High 20%
- Solid LZMA2 compression, the method 7z uses
- Photos and videos are stored as they are, saving time
- SHA-256 fingerprint for every file in a .9z archive
- Creates and extracts 9Z, 7Z, ZIP, TAR, GZIP, BZIP2, XZ, ZSTD, LZ4 and BROTLI
- AES-256 passwords for ZIP and 7Z archives
- Test any archive for damage
- Extract only the files you select
- Right-click menu in File Explorer: Add to archive, Extract here, Test
- Multithreaded compression
- Checksums: CRC32, MD5, SHA-1 and SHA-256
- Built-in benchmark
- Free, with no ads and no data collection

### Search terms (up to 7)
zip, 7z, archive, extract, compress, unzip, self-healing archive

> Do not use "7-Zip" as a search term: Store policy does not allow other products' names as keywords.

### Copyright and trademark info
© 2026 Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy

### Screenshots
At least 1, up to 10, PNG, 1366×768 or larger. Suggested set:
1. The main window showing a folder, with a .9z file selected
2. The Add to Archive window with format 9z and Recovery "Normal"
3. Inside a .9z archive, with the self-healing banner
4. The Test result for a .9z archive
5. The 9-zip submenu in the File Explorer right-click menu

The 4 screenshots submitted are in `store/screenshots/`. Capture at 100% display scaling, maximised to 1920×1080 if possible.

### Store logos
The package already contains every required tile and icon (`msix/Assets`). Partner Center may also ask for a 300×300 Store logo: a ready 300×300 file is at `store/9-zip-300.png`.

---

## 7. Privacy policy (published at https://github.com/dsvakola/9-zip/blob/main/PRIVACY.md)

**9-zip Privacy Policy**

Last updated: 21 October 2026

9-zip is developed by Prof. Dattaraj S. Vidyasagar and Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

9-zip does not collect, store, share or transmit any personal information. It has no accounts, no advertising, no analytics and no telemetry, and it does not connect to the internet.

9-zip reads and writes only the files and folders you choose when you create, open, test or extract archives. Those files stay on your computer. 9-zip keeps one small settings file on your computer, holding your favorite folders, which you can delete at any time by uninstalling the app.

The website and email links in the About window open in your own browser or email program.

Questions: softinquiry@vsa.edu.in
