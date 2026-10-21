# Desktop Software Stack (Windows): Tauri v2 + Rust + Vanilla TypeScript

Use this stack for all new desktop software and for converting existing Electron apps. Goals: small installer, low memory, fast start-up, reliable behaviour.

## 1. Technology choices
- **Shell:** Tauri v2 (latest stable). Uses Windows' built-in WebView2, so no Chromium is bundled.
- **Backend:** Rust only (stable toolchain, MSVC target). Do not add Zig or C unless an existing C library is truly unavoidable.
- **Frontend:** vanilla TypeScript with `strict` mode, built with Vite (Tauri's `vanilla-ts` template). Plain HTML and CSS. No React, Vue or Angular.
- **SolidJS:** only if a screen becomes too complex for vanilla TypeScript. Ask first.
- **Node.js:** only for build tools (Tauri CLI, Vite, TypeScript). Never at runtime.

## 2. Architecture rules
- All file system, OS, process and heavy work lives in Rust `#[tauri::command]` functions. The UI only displays data and sends user actions.
- Long tasks (scanning, copying, hashing, compression) run on background threads. Send progress through `tauri::ipc::Channel`, and support pause and cancel with atomic flags. The window must never freeze.
- One typed `api.ts` file wraps every `invoke` call. TypeScript types mirror the Rust structs (`#[serde(rename_all = "camelCase")]`).
- Commands return `Result<T, AppError>` with clear, user-readable messages. No `unwrap()` or `panic!` in command code.
- Call Windows directly through the `windows` crate (drives, volumes, Recycle Bin, file copy) instead of running PowerShell or cmd.
- Big lists and grids use virtual scrolling, rendering only the visible rows.
- Save settings and data with `serde_json` in the app data folder, using atomic writes (temporary file, then rename).
- Security: least-privilege capabilities, a strict CSP, no remote content, and the `opener` plugin limited to the app's own website and e-mail address.
- Plugins: add only what is needed (`dialog`, `opener`, and `fs` only if required).

## 3. Converting an Electron app
- `ipcMain.handle` becomes `#[tauri::command]`.
- The `preload.js` / `contextBridge` API becomes `api.ts`, keeping the same function names so the UI code changes as little as possible.
- `webContents.send` events become a `Channel` or `app.emit`.
- `dialog.showOpenDialog` becomes `tauri-plugin-dialog`. `shell.openPath` and `openExternal` become `tauri-plugin-opener`.
- `app.getPath('userData')` becomes the Tauri app data folder. On first run, migrate the user's existing data from the old Electron folder (`%APPDATA%\<old-app-name>`).
- Keep the existing HTML and CSS unless told otherwise. Convert renderer JavaScript to TypeScript.
- Remove `electron`, `electron-builder` and any Electron-only scripts.
- Study the old app first and report a migration plan plus a list of bugs and suggested improvements. Wait for approval before changing behaviour.

## 4. Size and performance
- Release profile in `Cargo.toml`: `lto = true`, `codegen-units = 1`, `opt-level = "s"` (or `3` for compute-heavy apps), `strip = true`, `panic = "abort"`.
- Keep dependencies few. Check with `cargo tree` and use features to turn off unused parts of crates.
- After every conversion, measure and report against the Electron version: installer size, EXE size, idle RAM, and start-up time.

## 5. Quality
- `cargo fmt`, `cargo clippy -- -D warnings`, and `tsc --noEmit` must be clean.
- Core logic is tested with `cargo test` on real temporary folders and files, before the UI is connected.
- Run the actual app and check every screen before reporting a step as done.

## 6. Packaging and branding
- NSIS installer (with the WebView2 bootstrapper) plus a portable EXE (the release `.exe`). Add MSI only when asked.
- The single source of the version number is `tauri.conf.json`. The About box reads it from Rust.
- Use the Vidyasagar Robotics Academy details in the About box, installer, NOTICE and README: Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Robotics Academy · https://vsa.edu.in/software/ · softinquiry@vsa.edu.in · logo at E:\Claude\logo.png.

## 7. Release Folder
- Always create folder named "Release" in the root of related software's or project's folder and build the .exe there only, instead of creating dist or dist-installer folders. I am much comfortable with "Release" name.

**Sharp graphics on every screen (Windows high-DPI):** All text, logos and icons in my software (app windows, dialogs, About boxes, installers, uninstallers) must look crisp and vivid at every display scaling from 100% to 300%.
- Make apps per-monitor DPI-aware.
- Use vector graphics (SVG) for logos and icons in the UI. Where a bitmap is unavoidable, supply it at 2x to 3x resolution.
- Never let Windows stretch small bitmaps. This especially applies to installer sidebar and header images (NSIS / Inno Setup): supply high-resolution images and scale them to the exact pixel size at run time with a high-quality filter, or supply one image per DPI.
- App and file icons (.ico) must contain 16, 24, 32, 48, 64, 128 and 256 px sizes.
- Before saying it is done, capture the real windows (app, every installer page, About, dialogs) at the actual screen scaling, zoom in, and check them. Report what was checked.

## 8. Working style
- Address me as Professor. Work one step at a time (3 or 4 if the steps are small), and wait for my go-ahead before each major step.
- Never build features that delete or overwrite user data without a preview and a safe fallback (Recycle Bin or a versions folder).
