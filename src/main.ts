/**
 * 9-zip - Archive Manager for Windows
 * Copyright (C) 2026 Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar
 * Licensed under the GNU General Public License v3.0 (GPL-3.0-only).
 */
import { open, save, message, ask } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import {
  getDrives,
  listDirectory,
  listArchive,
  getNineZManifest,
  testArchive,
  compressFiles,
  extractArchiveFiles,
  calculateHashes,
  runBenchmark,
  cancelTask,
  deleteToRecycleBin,
  getLaunchAction,
  getAppVersion,
  openPath,
  renameItem,
  transferItems,
  getKnownFolders,
  getFavorites,
  setFavorites,
  openArchiveEntry,
  showProperties,
  getSettings,
  setSettings,
  extractForDrag,
  startDragOut,
  pathExists,
  archiveProperties,
  getStartFolder,
  type ArchiveInfo,
  type KnownFolders,
  registerShellMenu,
  isPackaged,
  type CompressionFormat,
  type CompressionLevel,
  type RecoveryLevel,
  type DriveItem,
  type FileItem,
  type ArchiveEntry,
  type ProgressPayload,
} from "./api";

// STATE
let currentPath: string = "C:\\";
let isInsideArchive: boolean = false;
let activeArchivePath: string | null = null;
let currentFiles: FileItem[] = [];
let currentArchiveEntries: ArchiveEntry[] = [];
// Inside an archive: the folder being shown ("" = top level) and its rows.
let archiveFolder = "";
let archiveRows: ArchiveEntry[] = [];
let selectedPaths = new Set<string>();
let activeTaskId: string | null = null;
let repairNotice: string | null = null;

// DOM - ADDRESS & CONTROLS
const selectDrives = document.getElementById("select-drives") as HTMLSelectElement;
const inputPath = document.getElementById("input-path") as HTMLInputElement;
const btnNavUp = document.getElementById("btn-nav-up") as HTMLButtonElement;
const addressTypeIcon = document.getElementById("address-type-icon") as unknown as SVGElement;
const gridBody = document.getElementById("grid-body") as HTMLTableSectionElement;
const emptyState = document.getElementById("empty-state") as HTMLDivElement;
const emptyStateText = document.getElementById("empty-state-text") as HTMLParagraphElement;
const archiveBanner = document.getElementById("archive-banner") as HTMLDivElement;
const archiveBannerName = document.getElementById("archive-banner-name") as HTMLSpanElement;
const nineZSealBadge = document.getElementById("nine-z-seal-badge") as HTMLSpanElement;
const btnExitArchive = document.getElementById("btn-exit-archive") as HTMLButtonElement;

// STATUS BAR
const statusSelection = document.getElementById("status-selection") as HTMLDivElement;
const statusSize = document.getElementById("status-size") as HTMLDivElement;
const statusTotal = document.getElementById("status-total") as HTMLDivElement;

// TOOLBAR BUTTONS
const btnAdd = document.getElementById("btn-add") as HTMLButtonElement;
const btnExtract = document.getElementById("btn-extract") as HTMLButtonElement;
const btnTest = document.getElementById("btn-test") as HTMLButtonElement;
const btnCopy = document.getElementById("btn-copy") as HTMLButtonElement;
const btnMove = document.getElementById("btn-move") as HTMLButtonElement;
const btnDelete = document.getElementById("btn-delete") as HTMLButtonElement;
const btnInfo = document.getElementById("btn-info") as HTMLButtonElement;

// MODALS
const modalCompress = document.getElementById("modal-compress") as HTMLDivElement;
const compressOutputName = document.getElementById("compress-output-name") as HTMLInputElement;
const btnBrowseDest = document.getElementById("btn-browse-dest") as HTMLButtonElement;
const selectFormat = document.getElementById("select-format") as HTMLSelectElement;
const selectLevel = document.getElementById("select-level") as HTMLSelectElement;
const selectThreads = document.getElementById("select-threads") as HTMLSelectElement;
const compressPassword = document.getElementById("compress-password") as HTMLInputElement;
const compressSummary = document.getElementById("compress-summary") as HTMLDivElement;
const nineZAdvisory = document.getElementById("nine-z-advisory") as HTMLDivElement;
const selectRecovery = document.getElementById("select-recovery") as HTMLSelectElement;
const btnStartCompress = document.getElementById("btn-start-compress") as HTMLButtonElement;
const btnCloseCompress = document.getElementById("btn-close-compress") as HTMLButtonElement;
const btnCancelCompressDialog = document.getElementById("btn-cancel-compress-dialog") as HTMLButtonElement;

const modalExtract = document.getElementById("modal-extract") as HTMLDivElement;
const extractSource = document.getElementById("extract-source") as HTMLInputElement;
const extractDestination = document.getElementById("extract-destination") as HTMLInputElement;
const btnBrowseExtractDest = document.getElementById("btn-browse-extract-dest") as HTMLButtonElement;
const extractPassword = document.getElementById("extract-password") as HTMLInputElement;
const compressPasswordAgain = document.getElementById("compress-password-again") as HTMLInputElement;
const compressShowPassword = document.getElementById("compress-show-password") as HTMLInputElement;
const compressEncryptNames = document.getElementById("compress-encrypt-names") as HTMLInputElement;
const encryptionGroup = document.getElementById("encryption-group") as HTMLFieldSetElement;
const encryptionLegend = document.getElementById("encryption-legend") as HTMLLegendElement;
const modalPassword = document.getElementById("modal-password") as HTMLDivElement;
const passwordInput = document.getElementById("password-input") as HTMLInputElement;
const passwordShow = document.getElementById("password-show") as HTMLInputElement;
const passwordArchive = document.getElementById("password-archive") as HTMLParagraphElement;
const passwordWrong = document.getElementById("password-wrong") as HTMLParagraphElement;
const btnPasswordOk = document.getElementById("btn-password-ok") as HTMLButtonElement;
const btnPasswordCancel = document.getElementById("btn-password-cancel") as HTMLButtonElement;
const btnPasswordClose = document.getElementById("btn-password-close") as HTMLButtonElement;

// PASSWORDS: asked for when an archive needs one, remembered until 9-zip closes.
const archivePasswords = new Map<string, string>();

/** Shows the "Enter password" window; resolves to null when cancelled. */
function askPassword(archivePath: string, wrong: boolean): Promise<string | null> {
  return new Promise((resolve) => {
    passwordArchive.textContent = archivePath.split("\\").pop() || archivePath;
    passwordWrong.classList.toggle("hidden", !wrong);
    passwordInput.value = "";
    passwordShow.checked = false;
    passwordInput.type = "password";
    modalPassword.classList.remove("hidden");
    passwordInput.focus();
    const finish = (value: string | null) => {
      modalPassword.classList.add("hidden");
      btnPasswordOk.onclick = btnPasswordCancel.onclick = btnPasswordClose.onclick = null;
      passwordInput.onkeydown = null;
      resolve(value);
    };
    btnPasswordOk.onclick = () => finish(passwordInput.value);
    btnPasswordCancel.onclick = btnPasswordClose.onclick = () => finish(null);
    passwordInput.onkeydown = (e) => {
      if (e.key === "Enter") finish(passwordInput.value);
      if (e.key === "Escape") finish(null);
    };
  });
}

/**
 * Runs an archive operation, asking for the password and trying again when
 * the archive is encrypted, as 7-Zip does.
 */
async function withPassword<T>(archivePath: string, run: (password?: string) => Promise<T>): Promise<T> {
  let password = archivePasswords.get(archivePath);
  for (;;) {
    try {
      const result = await run(password);
      if (password) archivePasswords.set(archivePath, password);
      return result;
    } catch (err: unknown) {
      const code = (err as { code?: string }).code;
      if (code !== "password_required" && code !== "wrong_password") throw err;
      const entered = await askPassword(archivePath, code === "wrong_password");
      if (entered === null) throw new Error("No password was entered.");
      password = entered;
    }
  }
}
const extractOverwrite = document.getElementById("extract-overwrite") as HTMLInputElement;
const btnStartExtract = document.getElementById("btn-start-extract") as HTMLButtonElement;
const btnCloseExtract = document.getElementById("btn-close-extract") as HTMLButtonElement;
const btnCancelExtractDialog = document.getElementById("btn-cancel-extract-dialog") as HTMLButtonElement;

const modalProgress = document.getElementById("modal-progress") as HTMLDivElement;
const modalRename = document.getElementById("modal-rename") as HTMLDivElement;
const renameInput = document.getElementById("rename-input") as HTMLInputElement;
const btnRenameOk = document.getElementById("btn-rename-ok") as HTMLButtonElement;
const btnRenameCancel = document.getElementById("btn-rename-cancel") as HTMLButtonElement;
const btnRenameClose = document.getElementById("btn-rename-close") as HTMLButtonElement;
const favoritesList = document.getElementById("favorites-list") as HTMLDivElement;
const modalProperties = document.getElementById("modal-properties") as HTMLDivElement;
const propertiesTable = document.getElementById("properties-table") as HTMLTableSectionElement;
const progressTitle = document.getElementById("progress-title") as HTMLSpanElement;
const progressFile = document.getElementById("progress-file") as HTMLDivElement;
const progressSpeed = document.getElementById("progress-speed") as HTMLSpanElement;
const progressBarFill = document.getElementById("progress-bar-fill") as HTMLDivElement;
const progressPercent = document.getElementById("progress-percent") as HTMLSpanElement;
const progressCounts = document.getElementById("progress-counts") as HTMLSpanElement;
const progressBytes = document.getElementById("progress-bytes") as HTMLDivElement;
const btnProgressCancel = document.getElementById("btn-progress-cancel") as HTMLButtonElement;

const modalBenchmark = document.getElementById("modal-benchmark") as HTMLDivElement;
const btnCloseBenchmark = document.getElementById("btn-close-benchmark") as HTMLButtonElement;
const btnCloseBenchmarkBtn = document.getElementById("btn-close-benchmark-btn") as HTMLButtonElement;
const btnRunBenchmark = document.getElementById("btn-run-benchmark") as HTMLButtonElement;
const benchCompSpeed = document.getElementById("bench-comp-speed") as HTMLElement;
const benchCompMips = document.getElementById("bench-comp-mips") as HTMLElement;
const benchDecompSpeed = document.getElementById("bench-decomp-speed") as HTMLElement;
const benchDecompMips = document.getElementById("bench-decomp-mips") as HTMLElement;
const benchTotalRating = document.getElementById("bench-total-rating") as HTMLElement;
const benchThreads = document.getElementById("bench-threads") as HTMLElement;

const modalHash = document.getElementById("modal-hash") as HTMLDivElement;
const btnCloseHash = document.getElementById("btn-close-hash") as HTMLButtonElement;
const btnCloseHashBtn = document.getElementById("btn-close-hash-btn") as HTMLButtonElement;
const hashFilename = document.getElementById("hash-filename") as HTMLDivElement;
const hashFilesize = document.getElementById("hash-filesize") as HTMLDivElement;
const hashCrc32 = document.getElementById("hash-crc32") as HTMLElement;
const hashMd5 = document.getElementById("hash-md5") as HTMLElement;
const hashSha1 = document.getElementById("hash-sha1") as HTMLElement;
const hashSha256 = document.getElementById("hash-sha256") as HTMLElement;

const modalAbout = document.getElementById("modal-about") as HTMLDivElement;
const btnCloseAbout = document.getElementById("btn-close-about") as HTMLButtonElement;
const btnCloseAboutBtn = document.getElementById("btn-close-about-btn") as HTMLButtonElement;

// DIALOGS: native Windows message boxes titled "9-zip".
const openMessages: Promise<unknown>[] = [];
async function notify(text: string): Promise<void> {
  const kind = /error|failed|cannot|unable/i.test(text) ? "error" : "info";
  const shown = message(text, { title: "9-zip", kind });
  openMessages.push(shown);
  await shown;
}

// FORMATTING
function formatBytes(bytes: number): string {
  if (bytes === 0) return "0";
  return bytes.toLocaleString("en-US");
}

function formatDisplayBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + " " + sizes[i];
}

function formatDate(timestampSec?: number): string {
  if (!timestampSec) return "";
  const d = new Date(timestampSec * 1000);
  const pad = (n: number) => (n < 10 ? "0" + n : n);
  const year = d.getFullYear();
  const month = pad(d.getMonth() + 1);
  const day = pad(d.getDate());
  const hours = pad(d.getHours());
  const minutes = pad(d.getMinutes());
  return `${year}-${month}-${day} ${hours}:${minutes}`;
}

// 1. INITIALIZATION & LAUNCH ACTION
async function initApp() {
  // The window starts hidden (tauri.conf.json) so Explorer commands can show
  // just their dialog. Never leave it hidden if something goes wrong.
  const safety = setTimeout(() => getCurrentWindow().show(), 4000);
  await loadDrives();
  setupEventListeners();

  // Check if launched with a context menu argument or file association
  try {
    const launchAction = await getLaunchAction();
    if (launchAction) {
      if (EXPLORER_COMMANDS.includes(launchAction.action)) {
        clearTimeout(safety);
        await runExplorerCommand(launchAction.action, launchAction.targetPaths);
        return;
      }
      await getCurrentWindow().show();
      clearTimeout(safety);
      await handleLaunchAction(launchAction.action, launchAction.targetPaths);
      return;
    }
  } catch (err) {
    console.warn("No launch action:", err);
  }

  // Default start: the folder of the shortcut that opened 9-zip (a shortcut
  // on the Desktop shows the Desktop), otherwise the first drive.
  try {
    const startFolder = await getStartFolder();
    if (startFolder) currentPath = startFolder;
  } catch {
    // Keep the drive.
  }
  await getCurrentWindow().show();
  clearTimeout(safety);
  await navigateTo(currentPath);
}

// DIALOG MODE: Explorer commands show only their dialog, like 7-Zip, and
// 9-zip closes when the work is done.
const EXPLORER_COMMANDS = [
  "add",
  "add_9z",
  "add_zip",
  "add_7z",
  "quick_9z",
  "quick_zip",
  "quick_7z",
  "extract_here",
  "extract_to",
  "test",
  "hash",
];
let dialogMode = false;

async function runExplorerCommand(action: string, paths: string[]) {
  dialogMode = true;
  document.body.classList.add("dialog-mode");
  try {
    await handleLaunchAction(action, paths);
  } finally {
    // The Add, Extract and Checksum dialogs close the window themselves.
    if (!["add", "add_9z", "add_zip", "add_7z", "hash"].includes(action)) await closeDialogWindow();
  }
}

/** Sizes the hidden window to one dialog and shows it, with the dialog's title. */
async function showAsDialog(modal: HTMLElement, title: string) {
  if (!dialogMode) return;
  const box = modal.querySelector(".modal-window") as HTMLElement | null;
  // Let the dialog lay out at its natural size (animation frames do not run
  // while the window is hidden, so wait with a timer).
  await new Promise((done) => setTimeout(done, 30));
  const win = getCurrentWindow();
  await win.setTitle(`${title} - 9-zip`);
  // Like a 7-Zip dialog: fixed size, close button only.
  await win.setResizable(false);
  await win.setMaximizable(false);
  await win.setMinimizable(false);
  if (box) {
    const rect = box.getBoundingClientRect();
    // A dialog that closed straight away (after an error) has no size.
    if (rect.width > 0 && rect.height > 0) {
      await win.setSize(new LogicalSize(Math.ceil(rect.width), Math.ceil(rect.height)));
    }
    // Whenever the dialog grows or shrinks (for example the .9z notice
    // appears or disappears with the format), the window follows it.
    dialogResizeWatch?.disconnect();
    dialogResizeWatch = new ResizeObserver(() => {
      const now = box.getBoundingClientRect();
      if (now.width > 0 && now.height > 0) {
        win.setSize(new LogicalSize(Math.ceil(now.width), Math.ceil(now.height)));
      }
    });
    dialogResizeWatch.observe(box);
  }
  await win.center();
  await win.show();
  await win.setFocus();
}
let dialogResizeWatch: ResizeObserver | null = null;

async function closeDialogWindow() {
  if (!dialogMode) return;
  // Let the user read any message first.
  await Promise.allSettled(openMessages);
  await getCurrentWindow().close();
}

function parentFolder(path: string): string {
  const trimmed = path.replace(/\\$/, "");
  const cut = trimmed.lastIndexOf("\\");
  if (cut <= 0) return "C:\\";
  const parent = trimmed.substring(0, cut);
  return /^[a-zA-Z]:$/.test(parent) ? parent + "\\" : parent;
}

/** "photos.tar.gz" -> "photos", "data.9z" -> "data" */
function archiveStem(path: string): string {
  const name = path.split("\\").pop() || path;
  const multi = name.match(/^(.*)\.tar\.(gz|bz2|xz|zst|lz4|br)$/i);
  if (multi) return multi[1];
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.substring(0, dot) : name;
}

/** Commands from Explorer's right-click menu and from file associations. */
async function handleLaunchAction(action: string, paths: string[]) {
  const first = paths[0];
  if (action === "open") {
    await navigateTo(first);
  } else if (action === "extract_here" || action === "extract_to") {
    await navigateTo(parentFolder(first));
    for (const archive of paths) {
      const destination =
        action === "extract_here"
          ? parentFolder(archive)
          : `${parentFolder(archive).replace(/\\$/, "")}\\${archiveStem(archive)}`;
      await executeDirectExtraction(archive, destination);
    }
    await navigateTo(parentFolder(first));
  } else if (action === "extract") {
    await navigateTo(first);
    openExtractModal();
  } else if (action.startsWith("add")) {
    // The archive goes next to the selected items, like 7-Zip.
    await navigateTo(parentFolder(first));
    paths.forEach((p) => selectedPaths.add(p));
    updateRowSelections();
    openCompressModal();
    const format = action.replace("add_", "");
    if (["9z", "zip", "7z"].includes(format)) {
      selectFormat.value = format;
      selectFormat.dispatchEvent(new Event("change"));
    }
  } else if (action === "quick_9z" || action === "quick_zip") {
    await navigateTo(parentFolder(first));
    await quickAdd(paths, action === "quick_9z" ? "9z" : "zip");
  } else if (action === "hash") {
    await navigateTo(parentFolder(first));
    selectedPaths.add(first);
    executeHasher();
  } else if (action === "test") {
    await navigateTo(parentFolder(first));
    for (const archive of paths) await runArchiveTest(archive);
  }
}

/** Explorer's "Add to <name>.9z / .zip": no dialog, default settings, never overwrites. */
async function quickAdd(paths: string[], format: "9z" | "zip") {
  const folder = parentFolder(paths[0]).replace(/\\$/, "");
  const base =
    paths.length === 1
      ? archiveStem(paths[0])
      : folder.split("\\").pop() || "archive";
  const taken = new Set(currentFiles.map((f) => f.name.toLowerCase()));
  let name = `${base}.${format}`;
  for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${base} (${n}).${format}`;

  const taskId = "quick_" + Date.now();
  activeTaskId = taskId;
  showProgressModal("Compressing...");
  try {
    await compressFiles(
      {
        inputPaths: paths,
        outputPath: `${folder}\\${name}`,
        format,
        level: "normal",
        threads: Math.min(8, navigator.hardwareConcurrency || 4),
        taskId,
        recovery: "normal",
      },
      handleProgressUpdate
    );
    hideProgressModal();
    await navigateTo(parentFolder(paths[0]));
    selectedPaths.add(`${folder}\\${name}`);
    updateRowSelections();
  } catch (err: unknown) {
    hideProgressModal();
    notify("Compression error: " + (err as Error).message);
  } finally {
    activeTaskId = null;
  }
}

async function loadDrives() {
  try {
    const drives: DriveItem[] = await getDrives();
    selectDrives.innerHTML = "";

    // Find C: or default fixed drive
    let preferredDrive = "C:\\";
    for (const drive of drives) {
      const val = drive.letter + "\\";
      const opt = document.createElement("option");
      opt.value = val;
      opt.textContent = `${drive.letter} (${drive.label})`;
      selectDrives.appendChild(opt);
      if (drive.letter.toUpperCase() === "C:" || drive.letter.toUpperCase() === "C") {
        preferredDrive = val;
      }
    }

    currentPath = preferredDrive;
    selectDrives.value = preferredDrive;
  } catch (err) {
    console.error("Failed to load drives:", err);
    currentPath = "C:\\";
  }
}

// 2. NAVIGATION
async function navigateTo(target: string) {
  selectedPaths.clear();
  const isArchive = isArchiveFilename(target);

  if (isArchive) {
    await openArchive(target);
    return;
  }

  isInsideArchive = false;
  activeArchivePath = null;
  archiveBanner.classList.add("hidden");
  addressTypeIcon.innerHTML = '<path d="M2 3h4l2 2h6v8H2z" fill="#f5c542"/>';

  // Ensure trailing slash for drive roots
  let normalized = target;
  if (/^[a-zA-Z]:$/.test(normalized)) {
    normalized += "\\";
  }

  try {
    currentFiles = await listDirectory(normalized);
    currentPath = normalized;
    inputPath.value = currentPath;
    renderGrid();
  } catch {
    // If target directory failed, fallback gracefully to C:\
    try {
      currentFiles = await listDirectory("C:\\");
      currentPath = "C:\\";
      inputPath.value = currentPath;
      renderGrid();
    } catch {
      emptyState.classList.remove("hidden");
      emptyStateText.textContent = "Unable to read directory";
    }
  }
}

async function openArchive(archivePath: string) {
  try {
    isInsideArchive = true;
    activeArchivePath = archivePath;
    inputPath.value = archivePath + "\\";
    archiveBanner.classList.remove("hidden");
    archiveBannerName.textContent = archivePath.split("\\").pop() || archivePath;
    addressTypeIcon.innerHTML = '<path d="M2 2h12v12H2zm5 2v2h2V4zm0 3v2h2V7zm-2 5h6v1H5z" fill="#e67e22"/>';

    // Check if .9z
    if (archivePath.toLowerCase().endsWith(".9z")) {
      try {
        await getNineZManifest(archivePath);
        nineZSealBadge.classList.remove("hidden");
      } catch {
        nineZSealBadge.classList.add("hidden");
      }
    } else {
      nineZSealBadge.classList.add("hidden");
    }

    currentArchiveEntries = await withPassword(archivePath, (pw) => listArchive(archivePath, pw));
    archiveFolder = "";
    renderArchiveGrid();
  } catch (err: unknown) {
    notify("Cannot open archive: " + (err as Error).message);
    isInsideArchive = false;
    archiveBanner.classList.add("hidden");
    await navigateTo(parentFolder(archivePath));
  }
}

function renderGrid() {
  gridBody.innerHTML = "";
  if (currentFiles.length === 0) {
    emptyState.classList.remove("hidden");
    emptyStateText.textContent = "Folder is empty";
  } else {
    emptyState.classList.add("hidden");
  }

  for (const item of currentFiles) {
    const tr = document.createElement("tr");
    tr.dataset.path = item.path;
    if (selectedPaths.has(item.path)) {
      tr.classList.add("selected");
    }

    tr.innerHTML = `
      <td class="col-name">
        <div class="grid-name-cell">
          <svg class="grid-icon ${item.isDir ? "folder" : item.isArchive ? "archive" : "file"}" viewBox="0 0 16 16">
            ${
              item.isDir
                ? '<path d="M2 3h4l2 2h6v8H2z"/>'
                : item.isArchive
                ? '<path d="M2 2h12v12H2zm5 2v2h2V4zm0 3v2h2V7zm-2 5h6v1H5z"/>'
                : '<path d="M3 2h7l4 4v8H3z"/>'
            }
          </svg>
          <span>${escapeHtml(item.name)}</span>
        </div>
      </td>
      <td class="col-size">${item.isDir ? "" : formatBytes(item.size)}</td>
      <td class="col-packed"></td>
      <td class="col-modified">${formatDate(item.modified)}</td>
      <td class="col-attributes">${item.isDir ? "D" : "A"}</td>
      <td class="col-crc"></td>
    `;

    tr.addEventListener("click", (e) => selectRow(item.path, e, currentFiles.map((f) => f.path)));
    tr.draggable = true;
    tr.addEventListener("dragstart", (e) => dragOut(e, item.path));

    tr.addEventListener("dblclick", () => {
      if (item.isDir || item.isArchive) {
        navigateTo(item.path);
      } else {
        openPath(item.path).catch((err: unknown) => notify((err as Error).message));
      }
    });

    gridBody.appendChild(tr);
  }

  updateStatusBar();
}

/** Archive paths use "/"; some programs write "\\" or a trailing "/". */
function archivePathKey(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

/**
 * The rows of one folder inside the archive, like 7-Zip: its sub-folders
 * (also those only implied by file paths) first, then its files.
 */
function buildArchiveRows(): ArchiveEntry[] {
  const prefix = archiveFolder ? archiveFolder + "/" : "";
  const folders = new Map<string, ArchiveEntry>();
  const files: ArchiveEntry[] = [];
  for (const entry of currentArchiveEntries) {
    const key = archivePathKey(entry.path);
    if (!key.startsWith(prefix) || key === archiveFolder) continue;
    const rest = key.slice(prefix.length);
    const slash = rest.indexOf("/");
    if (slash < 0 && !entry.isDir) {
      files.push({ ...entry, name: rest });
      continue;
    }
    const name = slash >= 0 ? rest.slice(0, slash) : rest;
    const folderPath = prefix + name;
    const row: ArchiveEntry = folders.get(folderPath) ?? {
      name,
      path: folderPath,
      size: 0,
      compressedSize: 0,
      isDir: true,
      packedKnown: false,
    };
    if (slash < 0 && entry.modified) row.modified = entry.modified;
    if (!entry.isDir) row.size += entry.size;
    folders.set(folderPath, row);
  }
  const byName = (a: ArchiveEntry, b: ArchiveEntry) =>
    a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: "base" });
  return [...[...folders.values()].sort(byName), ...files.sort(byName)];
}

function enterArchiveFolder(folder: string) {
  archiveFolder = folder;
  selectedPaths.clear();
  renderArchiveGrid();
}

function renderArchiveGrid() {
  archiveRows = buildArchiveRows();
  if (activeArchivePath) {
    inputPath.value = activeArchivePath + "\\" + (archiveFolder ? archiveFolder.replace(/\//g, "\\") + "\\" : "");
  }
  gridBody.innerHTML = "";
  if (archiveRows.length === 0) {
    emptyState.classList.remove("hidden");
    emptyStateText.textContent = archiveFolder ? "Folder is empty" : "Archive is empty";
  } else {
    emptyState.classList.add("hidden");
  }

  for (const entry of archiveRows) {
    const tr = document.createElement("tr");
    tr.dataset.path = entry.path;
    if (selectedPaths.has(entry.path)) {
      tr.classList.add("selected");
    }

    tr.innerHTML = `
      <td class="col-name">
        <div class="grid-name-cell">
          <svg class="grid-icon ${entry.isDir ? "folder" : "file"}" viewBox="0 0 16 16">
            ${
              entry.isDir
                ? '<path d="M2 3h4l2 2h6v8H2z"/>'
                : '<path d="M3 2h7l4 4v8H3z"/>'
            }
          </svg>
          <span>${escapeHtml(entry.name || entry.path)}</span>
        </div>
      </td>
      <td class="col-size">${entry.isDir ? "" : formatBytes(entry.size)}</td>
      <td class="col-packed">${entry.isDir || entry.packedKnown === false ? "" : formatBytes(entry.compressedSize)}</td>
      <td class="col-modified">${formatDate(entry.modified)}</td>
      <td class="col-attributes">${entry.isDir ? "D" : "A"}</td>
      <td class="col-crc">${entry.crc32 || ""}</td>
    `;

    tr.addEventListener("click", (e) => selectRow(entry.path, e, archiveRows.map((a) => a.path)));
    tr.draggable = true;
    tr.addEventListener("dragstart", (e) => dragOut(e, entry.path));

    // Like 7-Zip: a file inside an archive opens in its usual program.
    tr.addEventListener("dblclick", () => {
      if (entry.isDir) {
        enterArchiveFolder(entry.path);
      } else {
        selectedPaths.clear();
        selectedPaths.add(entry.path);
        updateRowSelections();
        openSelection("open");
      }
    });

    gridBody.appendChild(tr);
  }

  updateStatusBar();
}

/**
 * Dragging rows out of 9-zip into Explorer copies them there, as in 7-Zip.
 * Items inside an archive are extracted to a temporary folder first.
 */
async function dragOut(e: DragEvent, path: string) {
  e.preventDefault();
  if (!selectedPaths.has(path)) {
    selectedPaths.clear();
    selectedPaths.add(path);
    updateRowSelections();
  }
  const list = selectionList();
  try {
    if (isInsideArchive && activeArchivePath) {
      const archive = activeArchivePath;
      const files = await withPassword(archive, (pw) => extractForDrag(archive, list, pw));
      await startDragOut(files);
    } else {
      await startDragOut(list);
    }
  } catch (err: unknown) {
    notify("Cannot drag: " + (err as Error).message);
  }
}

/** Windows list selection: click, Ctrl+click to toggle, Shift+click for a range. */
let selectionAnchor: string | null = null;
function selectRow(path: string, e: MouseEvent, order: string[]) {
  if (e.shiftKey && selectionAnchor && order.includes(selectionAnchor)) {
    const a = order.indexOf(selectionAnchor);
    const b = order.indexOf(path);
    if (!e.ctrlKey) selectedPaths.clear();
    order.slice(Math.min(a, b), Math.max(a, b) + 1).forEach((p) => selectedPaths.add(p));
  } else if (e.ctrlKey) {
    if (selectedPaths.has(path)) selectedPaths.delete(path);
    else selectedPaths.add(path);
    selectionAnchor = path;
  } else {
    selectedPaths.clear();
    selectedPaths.add(path);
    selectionAnchor = path;
  }
  updateRowSelections();
}

function updateRowSelections() {
  const rows = gridBody.querySelectorAll("tr");
  rows.forEach((r) => {
    const p = r.dataset.path;
    if (p && selectedPaths.has(p)) {
      r.classList.add("selected");
    } else {
      r.classList.remove("selected");
    }
  });
  updateStatusBar();
}

function updateStatusBar() {
  const totalItems = isInsideArchive ? archiveRows.length : currentFiles.length;
  const count = selectedPaths.size;

  let totalSelectedBytes = 0;
  if (isInsideArchive) {
    for (const e of archiveRows) {
      if (selectedPaths.has(e.path)) totalSelectedBytes += e.size;
    }
  } else {
    for (const f of currentFiles) {
      if (selectedPaths.has(f.path)) totalSelectedBytes += f.size;
    }
  }

  statusSelection.textContent = `${count} / ${totalItems} object(s) selected`;
  statusSize.textContent = formatDisplayBytes(totalSelectedBytes);
  statusTotal.textContent = `${totalItems} items`;
}

function escapeHtml(text: string): string {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}

function isArchiveFilename(name: string): boolean {
  const lower = name.toLowerCase();
  return (
    lower.endsWith(".9z") ||
    lower.endsWith(".zip") ||
    lower.endsWith(".7z") ||
    lower.endsWith(".tar") ||
    lower.endsWith(".tar.gz") ||
    lower.endsWith(".tgz") ||
    lower.endsWith(".tar.bz2") ||
    lower.endsWith(".tar.xz") ||
    lower.endsWith(".tar.zst") ||
    lower.endsWith(".tar.lz4") ||
    lower.endsWith(".tar.br") ||
    lower.endsWith(".rar") ||
    lower.endsWith(".iso") ||
    lower.endsWith(".cab")
  );
}

// 3. EVENT LISTENERS
function setupEventListeners() {
  // Drives
  selectDrives.addEventListener("change", () => {
    if (selectDrives.value) navigateTo(selectDrives.value);
  });

  // Up Button
  btnNavUp.addEventListener("click", () => {
    if (isInsideArchive && archiveFolder) {
      const cut = archiveFolder.lastIndexOf("/");
      enterArchiveFolder(cut > 0 ? archiveFolder.slice(0, cut) : "");
      return;
    }
    if (isInsideArchive && activeArchivePath) {
      const parent = activeArchivePath.substring(0, activeArchivePath.lastIndexOf("\\"));
      navigateTo(parent || "C:\\");
      return;
    }
    const trimmed = currentPath.replace(/\\$/, "");
    const lastSlash = trimmed.lastIndexOf("\\");
    if (lastSlash >= 0) {
      const parent = trimmed.substring(0, lastSlash + 1);
      navigateTo(parent || "C:\\");
    }
  });

  // Path input Enter
  inputPath.addEventListener("keydown", (e) => {
    if (e.key === "Enter") navigateTo(inputPath.value);
  });

  // Exit Archive
  btnExitArchive.addEventListener("click", () => {
    if (activeArchivePath) {
      const parent = activeArchivePath.substring(0, activeArchivePath.lastIndexOf("\\"));
      navigateTo(parent || "C:\\");
    }
  });

  // TOOLBAR: Add
  btnAdd.addEventListener("click", openCompressModal);
  const closeCompress = () => {
    modalCompress.classList.add("hidden");
    closeDialogWindow();
  };
  btnCloseCompress.addEventListener("click", closeCompress);
  btnCancelCompressDialog.addEventListener("click", closeCompress);
  btnBrowseDest.addEventListener("click", async () => {
    const ext = selectFormat.value;
    const selected = await save({
      defaultPath: compressOutputName.value,
      filters: [{ name: "Archive", extensions: [ext] }],
    });
    if (selected) compressOutputName.value = selected;
  });
  selectFormat.addEventListener("change", () => {
    const val = selectFormat.value;
    if (val === "9z") nineZAdvisory.classList.remove("hidden");
    else nineZAdvisory.classList.add("hidden");
    updatePasswordAvailability();

    let out = compressOutputName.value;
    out = out.replace(/\.(9z|zip|7z|tar\.gz|tar\.bz2|tar\.xz|tar\.zst|tar\.lz4|tar\.br|tar)$/i, "");
    compressOutputName.value = `${out}.${val === "gzip" ? "tar.gz" : val === "bzip2" ? "tar.bz2" : val === "xz" ? "tar.xz" : val === "zstd" ? "tar.zst" : val === "lz4" ? "tar.lz4" : val === "brotli" ? "tar.br" : val}`;
  });
  btnStartCompress.addEventListener("click", executeCompression);
  compressShowPassword.addEventListener("change", () => {
    // Show or hide what is typed in both boxes; nothing is cleared.
    const type = compressShowPassword.checked ? "text" : "password";
    compressPassword.type = type;
    compressPasswordAgain.type = type;
  });
  passwordShow.addEventListener("change", () => {
    passwordInput.type = passwordShow.checked ? "text" : "password";
  });

  // TOOLBAR: Extract
  btnExtract.addEventListener("click", openExtractModal);
  btnCloseExtract.addEventListener("click", () => modalExtract.classList.add("hidden"));
  btnCancelExtractDialog.addEventListener("click", () => modalExtract.classList.add("hidden"));
  btnBrowseExtractDest.addEventListener("click", async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: extractDestination.value,
    });
    if (selected) extractDestination.value = selected as string;
  });
  btnStartExtract.addEventListener("click", executeExtraction);

  // TOOLBAR: Test
  btnTest.addEventListener("click", async () => {
    let target = activeArchivePath;
    if (!target && selectedPaths.size === 1) {
      const sel = Array.from(selectedPaths)[0];
      if (isArchiveFilename(sel)) target = sel;
    }
    if (!target) {
      notify("Please select or open an archive to test.");
      return;
    }
    await runArchiveTest(target);
  });

  // TOOLBAR: Copy & Move
  btnCopy.addEventListener("click", () => copyOrMoveSelection(false));
  btnMove.addEventListener("click", () => copyOrMoveSelection(true));

  // TOOLBAR: Delete (Recycle Bin)
  btnDelete.addEventListener("click", async () => {
    if (selectedPaths.size === 0) {
      notify("Please select item(s) to delete.");
      return;
    }
    if (isInsideArchive) {
      notify(
        "9-zip cannot delete files from inside an archive yet. Extract the archive, delete the files, and create a new archive."
      );
      return;
    }
    const paths = Array.from(selectedPaths);
    const ok = await ask(`Send ${paths.length} item(s) to the Recycle Bin?`, { title: "9-zip", kind: "warning" });
    if (!ok) return;

    try {
      await deleteToRecycleBin(paths);
      selectedPaths.clear();
      await navigateTo(currentPath);
    } catch (err: unknown) {
      notify("Delete failed: " + (err as Error).message);
    }
  });

  // TOOLBAR: Info
  btnInfo.addEventListener("click", () => {
    if (isInsideArchive && activeArchivePath) {
      // Like 7-Zip: properties of the selection and of the archive.
      showArchiveProperties();
    } else if (selectedPaths.size === 1) {
      // Like 7-Zip: the Windows Properties window.
      const p = Array.from(selectedPaths)[0];
      showProperties(p).catch((err: unknown) => notify((err as Error).message));
    } else {
      modalAbout.classList.remove("hidden");
    }
  });

  // MENU BAR HOOKS: File
  const onMenu = (id: string, action: () => void) =>
    document.getElementById(id)?.addEventListener("click", action);
  onMenu("m-open", () => openSelection("open"));
  onMenu("m-open-inside", openSelectionInside);
  onMenu("m-view", () => openSelection("open"));
  onMenu("m-edit", () => openSelection("edit"));
  onMenu("m-rename", openRenameModal);
  onMenu("m-copy", () => copyOrMoveSelection(false));
  onMenu("m-move", () => copyOrMoveSelection(true));
  onMenu("m-delete", () => btnDelete.click());
  onMenu("m-properties", () => btnInfo.click());
  btnRenameOk.addEventListener("click", executeRename);
  const closeProperties = () => modalProperties.classList.add("hidden");
  document.getElementById("btn-properties-ok")?.addEventListener("click", closeProperties);
  document.getElementById("btn-properties-close")?.addEventListener("click", closeProperties);
  btnRenameCancel.addEventListener("click", () => modalRename.classList.add("hidden"));
  btnRenameClose.addEventListener("click", () => modalRename.classList.add("hidden"));
  renameInput.addEventListener("keydown", (e) => {
    if (e.key === "Enter") executeRename();
  });

  // COLUMNS: drag the header edges; widths are remembered, as in 7-Zip.
  setupColumnResizing();

  // MENU BAR HOOKS: Favorites
  onMenu("m-fav-add", addCurrentToFavorites);
  renderFavoritesMenu();

  // MENU BAR HOOKS: Edit, View, Tools, Help
  document.getElementById("m-select-all")?.addEventListener("click", () => {
    if (isInsideArchive) archiveRows.forEach((e) => selectedPaths.add(e.path));
    else currentFiles.forEach((f) => selectedPaths.add(f.path));
    updateRowSelections();
  });
  document.getElementById("m-deselect-all")?.addEventListener("click", () => {
    selectedPaths.clear();
    updateRowSelections();
  });
  document.getElementById("m-invert-selection")?.addEventListener("click", () => {
    if (isInsideArchive) {
      archiveRows.forEach((e) => {
        if (selectedPaths.has(e.path)) selectedPaths.delete(e.path);
        else selectedPaths.add(e.path);
      });
    } else {
      currentFiles.forEach((f) => {
        if (selectedPaths.has(f.path)) selectedPaths.delete(f.path);
        else selectedPaths.add(f.path);
      });
    }
    updateRowSelections();
  });
  document.getElementById("m-refresh")?.addEventListener("click", () => navigateTo(currentPath));
  document.getElementById("m-tools-benchmark")?.addEventListener("click", () => modalBenchmark.classList.remove("hidden"));
  document.getElementById("m-tools-hash")?.addEventListener("click", executeHasher);
  // A Store install has its Explorer menu declared by the package.
  isPackaged()
    .then((packaged) => {
      if (packaged) document.getElementById("m-tools-register-menu")?.classList.add("hidden");
    })
    .catch(() => {});
  document.getElementById("m-tools-register-menu")?.addEventListener("click", async () => {
    try {
      await registerShellMenu();
      notify("9-zip context menu successfully registered in Windows Explorer!");
    } catch (err: unknown) {
      notify("Context menu registration: " + (err as Error).message);
    }
  });
  const showAbout = async () => {
    const versionLine = document.getElementById("about-version");
    if (versionLine) {
      try {
        versionLine.textContent = "Version " + (await getAppVersion());
      } catch {
        versionLine.textContent = "";
      }
    }
    modalAbout.classList.remove("hidden");
  };
  document.getElementById("m-help-about")?.addEventListener("click", showAbout);
  // Website and email open in the user's browser / mail app, never inside 9-zip.
  document.querySelectorAll<HTMLAnchorElement>("a.external-link").forEach((a) =>
    a.addEventListener("click", (e) => {
      e.preventDefault();
      const url = a.dataset.url;
      if (url) openUrl(url).catch((err: unknown) => notify("Cannot open link: " + String(err)));
    })
  );
  document.getElementById("m-help-inventions")?.addEventListener("click", () => showHelp("nine-z"));
  document.getElementById("m-help-contents")?.addEventListener("click", () => showHelp("start"));
  document.getElementById("btn-help-close")?.addEventListener("click", () =>
    document.getElementById("modal-help")?.classList.add("hidden")
  );
  document.querySelectorAll<HTMLAnchorElement>("#modal-help a[data-page], #modal-help a[data-goto]").forEach((a) =>
    a.addEventListener("click", (e) => {
      e.preventDefault();
      showHelp(a.dataset.page || a.dataset.goto || "start");
    })
  );
  document.getElementById("m-exit")?.addEventListener("click", () => window.close());

  // BENCHMARK
  btnCloseBenchmark.addEventListener("click", () => modalBenchmark.classList.add("hidden"));
  btnCloseBenchmarkBtn.addEventListener("click", () => modalBenchmark.classList.add("hidden"));
  btnRunBenchmark.addEventListener("click", executeBenchmark);

  // HASH
  const closeHash = () => {
    modalHash.classList.add("hidden");
    closeDialogWindow();
  };
  btnCloseHash.addEventListener("click", closeHash);
  btnCloseHashBtn.addEventListener("click", closeHash);
  document.querySelectorAll(".btn-copy").forEach((b) => {
    b.addEventListener("click", () => {
      const id = (b as HTMLElement).dataset.copy;
      if (id) {
        const el = document.getElementById(id);
        if (el && el.textContent) {
          navigator.clipboard.writeText(el.textContent);
          const orig = b.textContent;
          b.textContent = "Copied";
          setTimeout(() => (b.textContent = orig), 1500);
        }
      }
    });
  });

  // ABOUT
  btnCloseAbout.addEventListener("click", () => modalAbout.classList.add("hidden"));
  btnCloseAboutBtn.addEventListener("click", () => modalAbout.classList.add("hidden"));

  // PROGRESS CANCEL
  btnProgressCancel.addEventListener("click", async () => {
    if (activeTaskId) {
      btnProgressCancel.textContent = "Cancelling...";
      await cancelTask(activeTaskId);
    }
  });

  // KEYBOARD SHORTCUTS
  // The same keys as 7-Zip File Manager.
  window.addEventListener("keydown", (e) => {
    const openModal = document.querySelector(".modal-backdrop:not(.hidden)");
    if (e.key === "Escape" && openModal && openModal !== modalProgress && openModal !== modalPassword) {
      openModal.classList.add("hidden");
      return;
    }
    // Typing in a text box or working in a dialog is not a file command.
    const target = e.target as HTMLElement;
    if (openModal || ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName)) return;

    const key = e.key.toLowerCase();
    const run = (action: () => void) => {
      e.preventDefault();
      action();
    };
    if (e.key === "F1") run(() => showHelp("start"));
    else if (e.key === "Enter" && e.altKey) run(() => btnInfo.click());
    else if (e.key === "Enter") run(() => openSelection("open"));
    else if (e.key === "PageDown" && e.ctrlKey) run(openSelectionInside);
    else if (e.key === "Backspace") run(() => btnNavUp.click());
    else if (e.key === "F2") run(openRenameModal);
    else if (e.key === "F3") run(() => openSelection("open"));
    else if (e.key === "F4") run(() => openSelection("edit"));
    else if (e.key === "F5") run(() => copyOrMoveSelection(false));
    else if (e.key === "F6") run(() => copyOrMoveSelection(true));
    else if (e.key === "Delete") run(() => btnDelete.click());
    else if (e.ctrlKey && key === "r") run(() => navigateTo(currentPath));
    else if (e.ctrlKey && key === "a") run(() => document.getElementById("m-select-all")?.click());
    else if (e.ctrlKey && key === "i") run(() => document.getElementById("m-invert-selection")?.click());
    else if (e.ctrlKey && key === "b") run(() => modalBenchmark.classList.remove("hidden"));
  });
}

// HELP (F1): contents on the left, one page at a time on the right.
function showHelp(page: string) {
  const modal = document.getElementById("modal-help");
  if (!modal) return;
  modal.querySelectorAll("section[data-page]").forEach((section) =>
    section.classList.toggle("active", (section as HTMLElement).dataset.page === page)
  );
  modal.querySelectorAll(".help-contents a").forEach((link) =>
    link.classList.toggle("active", (link as HTMLElement).dataset.page === page)
  );
  (document.getElementById("help-page") as HTMLElement).scrollTop = 0;
  modal.classList.remove("hidden");
}

// PROPERTIES inside an archive, laid out like 7-Zip's: the item, the
// selection, then the archive itself.
async function showArchiveProperties() {
  if (!activeArchivePath) return;
  const archive = activeArchivePath;
  const lines: [string, string][] = [];
  const add = (label: string, value: string | number | null | undefined) => {
    if (value !== null && value !== undefined && value !== "") lines.push([label, String(value)]);
  };
  const separator = () => lines.push(["", ""]);
  const bytes = (n: number | null | undefined) => (n === null || n === undefined ? undefined : formatBytes(n));

  // Every archive entry below a folder row, for its counts.
  const under = (folder: string) =>
    currentArchiveEntries.filter((e) => archivePathKey(e.path).startsWith(folder + "/"));
  const selected = archiveRows.filter((r) => selectedPaths.has(r.path));

  if (selected.length === 1) {
    const item = selected[0];
    add("Name", item.name);
    add("Size", bytes(item.size));
    add("Packed Size", item.isDir || item.packedKnown === false ? "" : bytes(item.compressedSize));
    add("Modified", item.modified ? formatDate(item.modified) : "");
    add("Attributes", item.isDir ? "D" : "A");
    if (item.isDir) {
      const inside = under(archivePathKey(item.path));
      add("Folders", new Set(inside.filter((e) => e.isDir).map((e) => archivePathKey(e.path))).size);
      add("Files", inside.filter((e) => !e.isDir).length);
    } else {
      add(item.sha256 ? "SHA-256 (VSA Seal)" : "CRC", item.sha256 || item.crc32 || "");
      add("Encrypted", item.encrypted ? "+" : "-");
      add("Method", item.method || "");
    }
    separator();
  }
  if (selected.length > 1) {
    const files = selected.flatMap((r) => (r.isDir ? under(archivePathKey(r.path)).filter((e) => !e.isDir) : [r]));
    add("Selected", `${selected.length} items`);
    add("Size", bytes(files.reduce((sum, f) => sum + f.size, 0)));
    add("Files", files.length);
    separator();
  }

  try {
    const info: ArchiveInfo = await withPassword(archive, (pw) => archiveProperties(archive, pw));
    add("Path", info.path);
    add("Type", info.kind);
    add("Physical Size", bytes(info.physicalSize));
    add("Size", bytes(info.size));
    add("Packed Size", bytes(info.packedSize));
    add("Headers Size", bytes(info.headersSize));
    add("Folders", info.folders);
    add("Files", info.files);
    add("Method", info.method);
    add("Solid", info.solid === null || info.solid === undefined ? "" : info.solid ? "+" : "-");
    add("Blocks", info.blocks);
    add("Encrypted", info.encrypted ? "+" : "-");
    const nz = info.nineZ;
    if (nz) {
      separator();
      add("9z Format Version", nz.formatVersion);
      if (nz.formatVersion >= 2) {
        add("Recovery Data", `${nz.recoveryPercent}% (${formatBytes(nz.recoverySize)} bytes)`);
        add("Data Blocks", formatBytes(nz.dataBlocks));
        add("Recovery Blocks", formatBytes(nz.recoveryBlocks));
        add("Block Size", bytes(nz.blockSize));
        add("Solid Files", nz.solidFiles);
        add("Stored Files", nz.storedFiles);
      }
      add("VSA Integrity Seal", `${nz.sealedFiles} file(s) with SHA-256 fingerprints`);
      add("Created By", nz.createdBy);
    }
  } catch (err: unknown) {
    add("Archive", "Cannot read: " + (err as Error).message);
  }

  propertiesTable.innerHTML = lines
    .map(([label, value]) =>
      label ? `<tr><td>${escapeHtml(label)}</td><td>${escapeHtml(value)}</td></tr>` : `<tr class="sep"><td colspan="2"></td></tr>`
    )
    .join("");
  modalProperties.classList.remove("hidden");
}

// COLUMN WIDTHS
async function setupColumnResizing() {
  const table = document.getElementById("grid-table") as HTMLTableElement;
  const headers = Array.from(table.querySelectorAll("th")) as HTMLTableCellElement[];
  const container = document.getElementById("grid-container") as HTMLElement;
  let current: Record<string, number> | null = null;
  // Like 7-Zip: the last column takes up any space left, so a scroll bar
  // appears only when the columns really are wider than the window.
  const apply = (widths: Record<string, number>) => {
    current = widths;
    const last = headers[headers.length - 1];
    let others = 0;
    for (const th of headers.slice(0, -1)) {
      const width = widths[th.dataset.col || ""] ?? th.offsetWidth;
      th.style.width = `${width}px`;
      others += width;
    }
    let lastWidth = Math.max(widths[last.dataset.col || ""] ?? 60, container.clientWidth - others);
    last.style.width = `${lastWidth}px`;
    table.style.width = `${others + lastWidth}px`;
    // Borders can add a few pixels; take them off the last column.
    const extra = table.offsetWidth - container.clientWidth;
    if (extra > 0 && lastWidth - extra >= (widths[last.dataset.col || ""] ?? 60)) {
      lastWidth -= extra;
      last.style.width = `${lastWidth}px`;
      table.style.width = `${others + lastWidth}px`;
    }
  };
  window.addEventListener("resize", () => {
    if (current) apply(current);
  });
  try {
    const saved = (await getSettings()).columnWidths as Record<string, number> | undefined;
    if (saved) apply(saved);
  } catch {
    // Defaults are fine.
  }
  for (const th of headers) {
    const grip = document.createElement("span");
    grip.className = "col-resizer";
    grip.addEventListener("click", (e) => e.stopPropagation());
    grip.addEventListener("mousedown", (e) => {
      e.preventDefault();
      e.stopPropagation();
      const startX = e.clientX;
      const startWidth = th.offsetWidth;
      const widths: Record<string, number> = {};
      headers.forEach((h) => (widths[h.dataset.col || ""] = h.offsetWidth));
      const move = (ev: MouseEvent) => {
        widths[th.dataset.col || ""] = Math.max(40, startWidth + ev.clientX - startX);
        apply(widths);
      };
      const up = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
        setSettings({ columnWidths: widths }).catch(() => {});
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    });
    th.appendChild(grip);
  }
}

// 4. FILE MANAGER ACTIONS (7-Zip File menu)
function selectionList(): string[] {
  return Array.from(selectedPaths);
}

/** Enter / F3 / F4: folders and archives open in 9-zip, files in their usual program. */
async function openSelection(verb: "open" | "edit") {
  const list = selectionList();
  if (list.length !== 1) {
    notify(list.length === 0 ? "Please select an item to open." : "Please select only one item to open.");
    return;
  }
  const path = list[0];
  if (isInsideArchive && activeArchivePath) {
    const entry = archiveRows.find((e) => e.path === path);
    if (!entry) return;
    if (entry.isDir) {
      enterArchiveFolder(entry.path);
      return;
    }
    showProgressModal("Opening...");
    try {
      const archive = activeArchivePath;
      await withPassword(archive, (pw) => openArchiveEntry(archive, entry.path, pw, handleProgressUpdate));
    } catch (err: unknown) {
      notify("Cannot open " + entry.name + ": " + (err as Error).message);
    } finally {
      hideProgressModal();
    }
    return;
  }
  const item = currentFiles.find((f) => f.path === path);
  if (!item) return;
  if (item.isDir || (item.isArchive && verb === "open")) {
    await navigateTo(item.path);
    return;
  }
  try {
    await openPath(item.path, verb);
  } catch (err: unknown) {
    notify((err as Error).message);
  }
}

/** Ctrl+PgDn: open the selected archive inside 9-zip. */
async function openSelectionInside() {
  const list = selectionList();
  const item = list.length === 1 ? currentFiles.find((f) => f.path === list[0]) : undefined;
  if (isInsideArchive || !item || !item.isArchive) {
    notify("Please select one archive file to open inside 9-zip.");
    return;
  }
  await navigateTo(item.path);
}

function openRenameModal() {
  if (isInsideArchive) {
    notify("9-zip cannot rename files inside an archive yet.");
    return;
  }
  const list = selectionList();
  if (list.length !== 1) {
    notify("Please select one item to rename.");
    return;
  }
  const item = currentFiles.find((f) => f.path === list[0]);
  if (!item) return;
  renameInput.value = item.name;
  renameInput.dataset.path = item.path;
  modalRename.classList.remove("hidden");
  renameInput.focus();
  // Like Windows: select the name without its extension.
  const dot = item.isDir ? -1 : item.name.lastIndexOf(".");
  renameInput.setSelectionRange(0, dot > 0 ? dot : item.name.length);
}

async function executeRename() {
  const path = renameInput.dataset.path;
  const newName = renameInput.value.trim();
  if (!path) return;
  try {
    const renamed = await renameItem(path, newName);
    modalRename.classList.add("hidden");
    await navigateTo(currentPath);
    selectedPaths.add(renamed);
    updateRowSelections();
  } catch (err: unknown) {
    notify((err as Error).message);
  }
}

/** F5 / F6: copy or move to a folder. Inside an archive, Copy To extracts the selection. */
async function copyOrMoveSelection(move: boolean) {
  const list = selectionList();
  if (list.length === 0) {
    notify(`Please select item(s) to ${move ? "move" : "copy"}.`);
    return;
  }
  if (isInsideArchive && move) {
    notify("Items cannot be moved out of an archive. Use Copy To to extract them.");
    return;
  }
  const destination = await open({
    directory: true,
    multiple: false,
    title: move ? "Move to folder" : isInsideArchive ? "Extract selected items to folder" : "Copy to folder",
    defaultPath: isInsideArchive ? undefined : currentPath,
  });
  if (!destination || typeof destination !== "string") return;

  if (isInsideArchive && activeArchivePath) {
    const taskId = "copy_" + Date.now();
    activeTaskId = taskId;
    showProgressModal("Extracting...");
    try {
      const archive = activeArchivePath;
      await withPassword(archive, (password) =>
        extractArchiveFiles(
          { archivePath: archive, destination, password, overwrite: false, taskId, entries: list },
          handleProgressUpdate
        )
      );
      hideProgressModal();
      showRepairNotice();
      notify(`Extracted ${list.length} item(s) to ${destination}.\nExisting files with the same names were left unchanged.`);
    } catch (err: unknown) {
      hideProgressModal();
      notify("Extraction error: " + (err as Error).message);
    } finally {
      activeTaskId = null;
    }
    return;
  }

  showProgressModal(move ? "Moving..." : "Copying...");
  try {
    const report = await transferItems(list, destination, move);
    hideProgressModal();
    if (report.skipped.length > 0) {
      const shown = report.skipped.slice(0, 10).join("\n");
      const more = report.skipped.length > 10 ? `\n...and ${report.skipped.length - 10} more` : "";
      notify(
        `${move ? "Moved" : "Copied"} ${report.done} item(s). These already existed and were left unchanged:\n${shown}${more}`
      );
    }
    selectedPaths.clear();
    await navigateTo(currentPath);
  } catch (err: unknown) {
    hideProgressModal();
    notify((move ? "Move" : "Copy") + " failed: " + (err as Error).message);
  }
}

// FAVORITES: Desktop, Documents, Downloads, C:\ and folders the user adds.
let favorites: string[] = [];

async function renderFavoritesMenu() {
  try {
    favorites = await getFavorites();
  } catch {
    favorites = [];
  }
  let known: KnownFolders = {};
  try {
    known = await getKnownFolders();
  } catch {
    known = {};
  }
  favoritesList.innerHTML = "";
  const add = (label: string, path: string) => {
    const entry = document.createElement("div");
    entry.className = "menu-entry";
    entry.textContent = label;
    entry.title = path;
    entry.addEventListener("click", () => navigateTo(path));
    favoritesList.appendChild(entry);
  };
  if (known.desktop) add("Desktop", known.desktop);
  if (known.documents) add("Documents", known.documents);
  if (known.downloads) add("Downloads", known.downloads);
  add("Drive C:\\", "C:\\");
  if (favorites.length > 0) {
    const sep = document.createElement("div");
    sep.className = "menu-separator";
    favoritesList.appendChild(sep);
    favorites.forEach((f) => add(f, f));
  }
}

async function addCurrentToFavorites() {
  const folder = isInsideArchive && activeArchivePath ? activeArchivePath : currentPath;
  const existing = favorites.findIndex((f) => f.toLowerCase() === folder.toLowerCase());
  if (existing >= 0) {
    const remove = await ask(`${folder} is already a favorite. Remove it from Favorites?`, {
      title: "9-zip",
      kind: "info",
    });
    if (!remove) return;
    favorites.splice(existing, 1);
  } else {
    favorites.push(folder);
  }
  try {
    await setFavorites(favorites);
  } catch (err: unknown) {
    notify("Cannot save favorites: " + (err as Error).message);
  }
  await renderFavoritesMenu();
}

// 5. MODAL OPERATIONS
function openCompressModal() {
  if (selectedPaths.size === 0) {
    notify("Please select at least one file or folder to add to archive.");
    return;
  }
  const list = Array.from(selectedPaths);
  const first = list[0].split("\\").pop() || "archive";
  const base = first.replace(/\.[^/.]+$/, "");
  compressOutputName.value = `${currentPath.replace(/\\$/, "")}\\${base}.9z`;
  compressSummary.textContent = `Selected: ${list.length} item(s)`;
  selectFormat.value = "9z";
  selectRecovery.value = "normal";
  nineZAdvisory.classList.remove("hidden");
  updatePasswordAvailability();
  modalCompress.classList.remove("hidden");
  showAsDialog(modalCompress, "Add to Archive");
}

/** Like 7-Zip, offer encryption only for formats that support it. */
function updatePasswordAvailability() {
  const supported = selectFormat.value === "zip" || selectFormat.value === "7z";
  for (const box of [compressPassword, compressPasswordAgain]) {
    box.disabled = !supported;
    if (!supported) box.value = "";
    box.placeholder = supported ? "" : "Not available";
  }
  compressShowPassword.disabled = !supported;
  // ZIP always shows file names; only 7z can hide them (ticked by default).
  compressEncryptNames.disabled = selectFormat.value !== "7z";
  compressEncryptNames.checked = selectFormat.value === "7z";
  encryptionGroup.classList.toggle("unavailable", !supported);
  encryptionLegend.textContent = supported
    ? "Encryption (AES-256)"
    : "Encryption (AES-256): available for 7z and zip";
}

async function executeCompression() {
  const outPath = compressOutputName.value.trim();
  if (!outPath) {
    notify("Please enter a valid archive name.");
    return;
  }
  const fmt = selectFormat.value as CompressionFormat;
  const level = selectLevel.value as CompressionLevel;
  const threads = parseInt(selectThreads.value, 10) || 4;
  const password = compressPassword.value || undefined;
  if (password && compressPasswordAgain.value !== compressPassword.value) {
    notify("The passwords do not match. Please type the same password in both boxes.");
    return;
  }
  if (await pathExists(outPath)) {
    const replace = await ask(`${outPath} already exists. Replace it?`, { title: "9-zip", kind: "warning" });
    if (!replace) return;
  }
  const inputPaths = Array.from(selectedPaths);
  const taskId = "comp_" + Date.now();
  activeTaskId = taskId;

  modalCompress.classList.add("hidden");
  showProgressModal("Compressing...");

  try {
    await compressFiles(
      {
        inputPaths,
        outputPath: outPath,
        format: fmt,
        level,
        threads,
        password,
        taskId,
        recovery: selectRecovery.value as RecoveryLevel,
        encryptNames: compressEncryptNames.checked,
      },
      handleProgressUpdate
    );
    hideProgressModal();
    selectedPaths.clear();
    if (!dialogMode) await navigateTo(currentPath);
  } catch (err: unknown) {
    hideProgressModal();
    notify("Compression error: " + (err as Error).message);
  } finally {
    activeTaskId = null;
    await closeDialogWindow();
  }
}

function openExtractModal() {
  let target = activeArchivePath;
  if (!target && selectedPaths.size === 1) {
    const sel = Array.from(selectedPaths)[0];
    if (isArchiveFilename(sel)) target = sel;
  }
  if (!target) {
    notify("Please select an archive to extract.");
    return;
  }
  extractSource.value = target;
  const defaultDest = target.replace(/\.(9z|zip|7z|tar\.gz|tgz|tar\.bz2|tar\.xz|tar\.zst|tar\.lz4|tar\.br|tar)$/i, "");
  extractDestination.value = defaultDest;
  modalExtract.classList.remove("hidden");
}

async function executeExtraction() {
  const archivePath = extractSource.value.trim();
  const destination = extractDestination.value.trim();
  const password = extractPassword.value || undefined;
  const overwrite = extractOverwrite.checked;
  const taskId = "ext_" + Date.now();
  activeTaskId = taskId;

  if (!destination) {
    notify("Please select an extraction destination.");
    return;
  }

  modalExtract.classList.add("hidden");
  showProgressModal("Extracting...");

  if (password) archivePasswords.set(archivePath, password);
  try {
    await withPassword(archivePath, (pw) =>
      extractArchiveFiles({ archivePath, destination, password: pw, overwrite, taskId }, handleProgressUpdate)
    );
    hideProgressModal();
    showRepairNotice();
    await navigateTo(destination);
  } catch (err: unknown) {
    hideProgressModal();
    notify("Extraction error: " + (err as Error).message);
  } finally {
    activeTaskId = null;
  }
}

async function executeDirectExtraction(archivePath: string, destination: string) {
  const taskId = "ext_dir_" + Date.now();
  activeTaskId = taskId;
  showProgressModal("Extracting...");
  try {
    // Like 7-Zip, never replace files that are already there without asking.
    await withPassword(archivePath, (password) =>
      extractArchiveFiles({ archivePath, destination, password, overwrite: false, taskId }, handleProgressUpdate)
    );
    hideProgressModal();
    showRepairNotice();
    await navigateTo(destination);
  } catch (err: unknown) {
    hideProgressModal();
    notify("Extraction error: " + (err as Error).message);
  } finally {
    activeTaskId = null;
  }
}

async function executeBenchmark() {
  const taskId = "bench_" + Date.now();
  activeTaskId = taskId;
  btnRunBenchmark.disabled = true;
  benchCompSpeed.textContent = "Testing...";
  benchDecompSpeed.textContent = "Testing...";
  benchTotalRating.textContent = "Running...";

  try {
    const res = await runBenchmark(4, taskId, (p) => {
      if (p.phase === "benchmark_compressing") {
        benchCompSpeed.textContent = `${p.speedMbPerSec.toFixed(1)} MB/s`;
      } else if (p.phase === "benchmark_decompressing") {
        benchDecompSpeed.textContent = `${p.speedMbPerSec.toFixed(1)} MB/s`;
      }
    });

    benchCompSpeed.textContent = `${res.compressSpeedMbPerSec.toFixed(1)} MB/s`;
    benchCompMips.textContent = `${res.compressMips} MIPS`;
    benchDecompSpeed.textContent = `${res.decompressSpeedMbPerSec.toFixed(1)} MB/s`;
    benchDecompMips.textContent = `${res.decompressMips} MIPS`;
    benchTotalRating.textContent = `${res.totalRatingMips} MIPS`;
    benchThreads.textContent = `Threads: ${res.threads}`;
  } catch (err: unknown) {
    notify("Benchmark failed: " + (err as Error).message);
  } finally {
    btnRunBenchmark.disabled = false;
    activeTaskId = null;
  }
}

async function executeHasher() {
  if (selectedPaths.size !== 1) {
    notify("Please select one file to compute checksums.");
    return;
  }
  const filePath = Array.from(selectedPaths)[0];
  const fileObj = currentFiles.find((f) => f.path === filePath);
  if (fileObj && fileObj.isDir) {
    notify("Please select a file, not a folder.");
    return;
  }

  modalHash.classList.remove("hidden");
  showAsDialog(modalHash, "Checksum");
  hashFilename.textContent = "File: " + (filePath.split("\\").pop() || filePath);
  hashFilesize.textContent = fileObj ? "Size: " + formatDisplayBytes(fileObj.size) : "";
  hashCrc32.textContent = "Calculating...";
  hashMd5.textContent = "Calculating...";
  hashSha1.textContent = "Calculating...";
  hashSha256.textContent = "Calculating...";

  const taskId = "hash_" + Date.now();
  activeTaskId = taskId;

  try {
    const res = await calculateHashes(filePath, taskId);
    hashCrc32.textContent = res.crc32;
    hashMd5.textContent = res.md5;
    hashSha1.textContent = res.sha1;
    hashSha256.textContent = res.sha256;
  } catch (err: unknown) {
    notify("Checksum error: " + (err as Error).message);
    modalHash.classList.add("hidden");
  } finally {
    activeTaskId = null;
  }
}

// 5. PROGRESS
function showProgressModal(title: string) {
  progressTitle.textContent = title;
  repairNotice = null;
  progressFile.textContent = "Starting...";
  progressSpeed.textContent = "0.0 MB/s";
  progressBarFill.style.width = "0%";
  progressPercent.textContent = "0%";
  progressCounts.textContent = "0 / 0";
  progressBytes.textContent = "0 B / 0 B";
  btnProgressCancel.textContent = "Cancel";
  modalProgress.classList.remove("hidden");
  showAsDialog(modalProgress, title.replace(/\.+$/, ""));
}

function hideProgressModal() {
  modalProgress.classList.add("hidden");
}

function showRepairNotice() {
  if (repairNotice) {
    notify(repairNotice + "\n\nAll files were restored and match their original SHA-256 fingerprints.");
    repairNotice = null;
  }
}

async function runArchiveTest(archivePath: string) {
  const taskId = "test_" + Date.now();
  activeTaskId = taskId;
  showProgressModal("Testing...");
  try {
    const r = await withPassword(archivePath, (pw) => testArchive(archivePath, taskId, pw, handleProgressUpdate));
    hideProgressModal();
    const lines = [
      "There are no errors.",
      "",
      `Archive: ${archivePath.split("\\").pop()}`,
      `Folders: ${formatBytes(r.folders)}`,
      `Files: ${formatBytes(r.files)}`,
      `Size: ${formatBytes(r.size)} bytes`,
      `Packed Size: ${formatBytes(r.packedSize)} bytes`,
    ];
    const h = r.nineZ;
    if (h) {
      const repairs: string[] = [];
      if (h.blocksRepaired > 0) repairs.push(`${h.blocksRepaired} damaged block(s) can be rebuilt from parity`);
      if (h.headerRepaired) repairs.push("the header is damaged, but its backup copy is intact");
      if (h.indexRepaired) repairs.push("the block index is damaged, but its backup copy is intact");
      lines.push(
        "",
        `9-zip self-healing (${h.recoveryPercent}% recovery data): ${h.blocksChecked} blocks checked, every file matches its SHA-256 fingerprint.`,
        repairs.length > 0
          ? "Repairable damage found: " + repairs.join("; ") + ". Extracting repairs it automatically."
          : "No damage found."
      );
      if (h.indexLost) {
        lines.push("Warning: the block index is unreadable, so damage was checked with file fingerprints only.");
      }
    }
    notify(lines.join("\n"));
  } catch (err: unknown) {
    hideProgressModal();
    notify("Test failed: " + (err as Error).message);
  } finally {
    activeTaskId = null;
    repairNotice = null;
  }
}

function handleProgressUpdate(p: ProgressPayload) {
  if (p.phase === "repaired") {
    repairNotice = p.currentFile;
  }
  progressFile.textContent = p.currentFile;
  progressSpeed.textContent = `${p.speedMbPerSec.toFixed(1)} MB/s`;
  const pct = Math.min(100, Math.max(0, p.percent));
  progressBarFill.style.width = `${pct}%`;
  progressPercent.textContent = `${Math.round(pct)}%`;
  progressCounts.textContent = `${p.filesProcessed} / ${p.totalFiles}`;
  progressBytes.textContent = `${formatDisplayBytes(p.bytesProcessed)} / ${formatDisplayBytes(p.totalBytes)}`;
}

window.addEventListener("DOMContentLoaded", initApp);
