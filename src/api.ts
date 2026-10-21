import { invoke, Channel } from "@tauri-apps/api/core";

export type CompressionFormat =
  | "9z"
  | "7z"
  | "zip"
  | "tar"
  | "gzip"
  | "bzip2"
  | "xz"
  | "zstd"
  | "lz4"
  | "brotli";

export type CompressionLevel =
  | "store"
  | "fastest"
  | "fast"
  | "normal"
  | "maximum"
  | "ultra";

export type RecoveryLevel = "low" | "normal" | "high";

export interface CompressOptions {
  inputPaths: string[];
  outputPath: string;
  format: CompressionFormat;
  level: CompressionLevel;
  password?: string;
  threads: number;
  taskId: string;
  /** .9z only */
  recovery?: RecoveryLevel;
  /** 7z with a password: also hide the file names. */
  encryptNames?: boolean;
}

export interface ExtractOptions {
  archivePath: string;
  destination: string;
  password?: string;
  overwrite: boolean;
  taskId: string;
  /** Archive paths to extract; a folder brings everything inside it. */
  entries?: string[];
}

export interface ProgressPayload {
  taskId: string;
  phase: string;
  currentFile: string;
  filesProcessed: number;
  totalFiles: number;
  bytesProcessed: number;
  totalBytes: number;
  percent: number;
  speedMbPerSec: number;
}

export interface ArchiveEntry {
  name: string;
  path: string;
  size: number;
  compressedSize: number;
  isDir: boolean;
  modified?: number;
  crc32?: string;
  method?: string | null;
  encrypted?: boolean;
  /** False for files sharing a solid block: no packed size of their own. */
  packedKnown?: boolean;
  sha256?: string | null;
}

export interface FileItem {
  name: string;
  path: string;
  size: number;
  isDir: boolean;
  isArchive: boolean;
  modified?: number;
  extension: string;
}

export interface DriveItem {
  letter: string;
  label: string;
  totalBytes: number;
  freeBytes: number;
  driveType: string;
}

export interface HashResult {
  path: string;
  fileName: string;
  fileSize: number;
  crc32: string;
  md5: string;
  sha1: string;
  sha256: string;
}

export interface BenchmarkResult {
  threads: number;
  compressSpeedMbPerSec: number;
  compressMips: number;
  decompressSpeedMbPerSec: number;
  decompressMips: number;
  totalRatingMips: number;
}

export interface LaunchAction {
  action: string;
  targetPath: string;
  targetPaths: string[];
}

export interface NineZEntryMeta {
  path: string;
  uncompressedSize: number;
  compressedSize: number;
  isDir: boolean;
  modified?: number;
  sha256: string;
  method: string;
  offset?: number;
}

export interface NineZManifest {
  version: string;
  creator: string;
  organization: string;
  storeUrl: string;
  dataShards: number;
  parityShards: number;
  shardSize: number;
  entries: NineZEntryMeta[];
}

export async function getDrives(): Promise<DriveItem[]> {
  return await invoke<DriveItem[]>("get_drives");
}

export async function listDirectory(path: string): Promise<FileItem[]> {
  return await invoke<FileItem[]>("list_directory", { path });
}

export async function listArchive(
  path: string,
  password?: string
): Promise<ArchiveEntry[]> {
  return await invoke<ArchiveEntry[]>("list_archive", { path, password });
}

export async function getNineZManifest(path: string): Promise<NineZManifest> {
  return await invoke<NineZManifest>("get_nine_z_manifest", { path });
}

export interface NineZHealth {
  formatVersion: number;
  recoveryPercent: number;
  blocksChecked: number;
  blocksRepaired: number;
  headerRepaired: boolean;
  indexRepaired: boolean;
  indexLost: boolean;
  filesVerified: number;
}

export interface TestReport {
  folders: number;
  files: number;
  size: number;
  packedSize: number;
  nineZ?: NineZHealth | null;
}

export async function testArchive(
  path: string,
  taskId: string,
  password?: string,
  onProgress?: (p: ProgressPayload) => void
): Promise<TestReport> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<TestReport>("test_archive", {
    path,
    password,
    taskId,
    onProgress: channel,
  });
}

export async function compressFiles(
  options: CompressOptions,
  onProgress?: (p: ProgressPayload) => void
): Promise<void> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<void>("compress_files", {
    options,
    onProgress: channel,
  });
}

export async function extractArchiveFiles(
  options: ExtractOptions,
  onProgress?: (p: ProgressPayload) => void
): Promise<void> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<void>("extract_archive_files", {
    options,
    onProgress: channel,
  });
}

export async function calculateHashes(
  path: string,
  taskId: string,
  onProgress?: (p: ProgressPayload) => void
): Promise<HashResult> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<HashResult>("calculate_hashes", {
    path,
    taskId,
    onProgress: channel,
  });
}

export async function runBenchmark(
  threads: number,
  taskId: string,
  onProgress?: (p: ProgressPayload) => void
): Promise<BenchmarkResult> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<BenchmarkResult>("run_benchmark", {
    threads,
    taskId,
    onProgress: channel,
  });
}

export async function cancelTask(taskId: string): Promise<boolean> {
  return await invoke<boolean>("cancel_task", { taskId });
}

export async function deleteToRecycleBin(paths: string[]): Promise<void> {
  return await invoke<void>("delete_to_recycle_bin", { paths });
}

export async function getAppVersion(): Promise<string> {
  return await invoke<string>("get_app_version");
}

export async function getLaunchAction(): Promise<LaunchAction | null> {
  return await invoke<LaunchAction | null>("get_launch_action");
}

export async function isPackaged(): Promise<boolean> {
  return await invoke<boolean>("is_packaged");
}

export async function registerShellMenu(): Promise<void> {
  return await invoke<void>("register_shell_menu");
}

export async function showInFolder(path: string): Promise<void> {
  return await invoke<void>("show_in_folder", { path });
}

export async function openPath(path: string, verb: "open" | "edit" = "open"): Promise<void> {
  return await invoke<void>("open_path", { path, verb });
}

export async function renameItem(path: string, newName: string): Promise<string> {
  return await invoke<string>("rename_item", { path, newName });
}

export interface TransferReport {
  done: number;
  skipped: string[];
}

export async function transferItems(
  paths: string[],
  destination: string,
  moveItems: boolean
): Promise<TransferReport> {
  return await invoke<TransferReport>("transfer_items", { paths, destination, moveItems });
}

export interface KnownFolders {
  desktop?: string | null;
  documents?: string | null;
  downloads?: string | null;
}

export async function getKnownFolders(): Promise<KnownFolders> {
  return await invoke<KnownFolders>("get_known_folders");
}

export async function getFavorites(): Promise<string[]> {
  return await invoke<string[]>("get_favorites");
}

export async function setFavorites(favorites: string[]): Promise<void> {
  return await invoke<void>("set_favorites", { favorites });
}

export async function openArchiveEntry(
  archivePath: string,
  entry: string,
  password?: string,
  onProgress?: (p: ProgressPayload) => void
): Promise<void> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<void>("open_archive_entry", {
    archivePath,
    entry,
    password,
    onProgress: channel,
  });
}

export async function showProperties(path: string): Promise<void> {
  return await invoke<void>("show_properties", { path });
}

export type Settings = Record<string, unknown>;

export async function getSettings(): Promise<Settings> {
  return await invoke<Settings>("get_settings");
}

export async function setSettings(changes: Settings): Promise<void> {
  return await invoke<void>("set_settings", { changes });
}

export async function extractForDrag(
  archivePath: string,
  entries: string[],
  password?: string,
  onProgress?: (p: ProgressPayload) => void
): Promise<string[]> {
  const channel = new Channel<ProgressPayload>();
  if (onProgress) {
    channel.onmessage = onProgress;
  }
  return await invoke<string[]>("extract_for_drag", {
    archivePath,
    entries,
    password,
    onProgress: channel,
  });
}

export async function startDragOut(paths: string[]): Promise<void> {
  return await invoke<void>("start_drag_out", { paths });
}

export async function pathExists(path: string): Promise<boolean> {
  return await invoke<boolean>("path_exists", { path });
}

export interface NineZDetails {
  formatVersion: number;
  recoveryPercent: number;
  recoverySize: number;
  dataBlocks: number;
  recoveryBlocks: number;
  blockSize: number;
  solidBlocks: number;
  solidFiles: number;
  storedFiles: number;
  sealedFiles: number;
  headersSize: number;
  dictionary?: number | null;
  createdBy: string;
}

export interface ArchiveInfo {
  path: string;
  kind: string;
  physicalSize: number;
  size: number;
  packedSize?: number | null;
  files: number;
  folders: number;
  method: string;
  solid?: boolean | null;
  blocks?: number | null;
  headersSize?: number | null;
  encrypted: boolean;
  nineZ?: NineZDetails | null;
}

export async function archiveProperties(path: string, password?: string): Promise<ArchiveInfo> {
  return await invoke<ArchiveInfo>("archive_properties", { path, password });
}

/** The folder of the shortcut that started 9-zip, if any. */
export async function getStartFolder(): Promise<string | null> {
  return await invoke<string | null>("get_start_folder");
}
