// Folder paths on Windows ("C:\Users\...") and Linux ("/home/...").
// On Windows every helper behaves exactly as the code did before Linux
// support was added.

/** True in the Windows build (WebView2), false on Linux (WebKitGTK). */
export const IS_WINDOWS = navigator.userAgent.includes("Windows");

/** The folder separator: "\" on Windows, "/" on Linux. */
export const SEP = IS_WINDOWS ? "\\" : "/";

/** Where to go when nothing better is known: drive C: or the file system root. */
export const DEFAULT_ROOT = IS_WINDOWS ? "C:\\" : "/";

/** Removes one trailing separator: "C:\Data\" -> "C:\Data", "/" -> "". */
export function trimSep(path: string): string {
  return path.endsWith(SEP) ? path.slice(0, -1) : path;
}

/** The last part of a path: "C:\Data\photos.zip" -> "photos.zip". */
export function baseName(path: string): string {
  return path.split(SEP).pop() || path;
}

/** Joins a folder and a name with exactly one separator between them. */
export function joinPath(folder: string, name: string): string {
  return `${trimSep(folder)}${SEP}${name}`;
}

/** The folder that holds `path`. Drive roots keep their "\" ("C:\"). */
export function parentFolder(path: string): string {
  const trimmed = trimSep(path);
  const cut = trimmed.lastIndexOf(SEP);
  if (cut <= 0) return DEFAULT_ROOT;
  const parent = trimmed.substring(0, cut);
  return IS_WINDOWS && /^[a-zA-Z]:$/.test(parent) ? parent + "\\" : parent;
}

/** The folder above `path` with its trailing separator, or null at the top. */
export function upFolder(path: string): string | null {
  const trimmed = trimSep(path);
  const cut = trimmed.lastIndexOf(SEP);
  if (cut < 0) return null;
  return trimmed.substring(0, cut + 1) || DEFAULT_ROOT;
}

/** Windows ignores upper and lower case in paths; Linux does not. */
export function samePath(a: string, b: string): boolean {
  return IS_WINDOWS ? a.toLowerCase() === b.toLowerCase() : a === b;
}
