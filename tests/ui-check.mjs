// Automatic window checks for 9-zip, run before every build:
//
//   npm run check:ui
//
// It opens the real 9-zip window code (index.html, src/main.ts) in Microsoft
// Edge with a pretend backend, then checks what a reviewer would notice:
//   1. every menu item and button is wired to code,
//   2. the window starts without script errors,
//   3. every dialog fits the window, with no scroll bars,
//   4. the password boxes work for each archive format,
//   5. dialogs started from Explorer fill their window exactly,
//      with no grey margin, scroll bar, resize or maximise.
// It prints each problem it finds and exits with an error if there are any.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { createServer } from "vite";
import { chromium } from "playwright-core";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const problems = [];
const problem = (text) => problems.push(text);
let passed = 0;
const ok = () => passed++;

// 1. Static check: every menu item and button id is used by the code.
const html = readFileSync(join(root, "index.html"), "utf8");
const code = readFileSync(join(root, "src", "main.ts"), "utf8");
for (const [, id] of html.matchAll(/id="((?:m|btn)-[a-z0-9-]+)"/g)) {
  if (code.includes(`"${id}"`)) ok();
  else problem(`"${id}" exists in the window but no code uses it (a dead menu item or button).`);
}

// A pretend backend: answers the commands the window sends.
const mockBackend = (launch, settings = {}) => `
  window.__CALLS__ = [];
  window.__MESSAGES__ = [];
  let nextId = 0;
  const desktop = "C:\\\\Users\\\\Test\\\\Desktop";
  const files = [
    { name: "Reports", path: desktop + "\\\\Reports", size: 0, isDir: true, isArchive: false, modified: 1700000000, extension: "" },
    { name: "notes.txt", path: desktop + "\\\\notes.txt", size: 1200, isDir: false, isArchive: false, modified: 1700000000, extension: "txt" },
    { name: "photos.zip", path: desktop + "\\\\photos.zip", size: 50000, isDir: false, isArchive: true, modified: 1700000000, extension: "zip" },
  ];
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: "main" }, currentWebview: { windowLabel: "main", label: "main" } },
    transformCallback(callback) { const id = ++nextId; window["_" + id] = callback; return id; },
    unregisterCallback(id) { delete window["_" + id]; },
    convertFileSrc: (path) => path,
    async invoke(cmd, args) {
      window.__CALLS__.push({ cmd, args: JSON.parse(JSON.stringify(args ?? {}, (k, v) => (typeof v === "function" ? undefined : v))) });
      switch (cmd) {
        case "get_drives": return [{ letter: "C:", label: "Local Disk", totalBytes: 1, freeBytes: 1, driveType: "fixed" }];
        case "list_directory": return files;
        case "list_archive": return [
          { name: "notes", path: "notes/", size: 0, compressedSize: 0, isDir: true, packedKnown: false },
          { name: "a.txt", path: "notes/a.txt", size: 100, compressedSize: 40, isDir: false, packedKnown: false, method: "LZMA2 (solid)", sha256: "ab".repeat(32), crc32: "ABABABAB" },
          { name: "deep.txt", path: "notes/inner/deep.txt", size: 50, compressedSize: 20, isDir: false, packedKnown: false },
          { name: "photo.jpg", path: "photo.jpg", size: 9000, compressedSize: 9000, isDir: false, packedKnown: true, method: "Stored" },
        ];
        case "archive_properties": return { path: "C:\\\\Archives\\\\x.9z", kind: "9z", physicalSize: 12000, size: 9150, packedSize: 9100, files: 3, folders: 2, method: "LZMA2:8m (solid) + Stored", solid: true, blocks: 1, headersSize: 300, encrypted: false, nineZ: { formatVersion: 3, recoveryPercent: 10, recoverySize: 910, dataBlocks: 40, recoveryBlocks: 4, blockSize: 16384, solidBlocks: 1, solidFiles: 2, storedFiles: 1, sealedFiles: 3, headersSize: 300, dictionary: 8388608, createdBy: "9-zip" } };
        case "get_launch_action": return ${JSON.stringify(launch)};
        case "get_known_folders": return { desktop };
        case "get_favorites": return [];
        case "get_settings": return ${JSON.stringify(settings)};
        case "is_packaged": return false;
        case "path_exists": return false;
        case "plugin:dialog|message": window.__MESSAGES__.push(args); return "Ok";
        case "plugin:dialog|ask": return true;
        case "compress_files": await new Promise((r) => setTimeout(r, 3000)); return null;
        default: return null;
      }
    },
  };
`;

const server = await createServer({
  root,
  configFile: join(root, "vite.config.ts"),
  server: { port: 1430, strictPort: false },
  logLevel: "error",
});
await server.listen();
const url = server.resolvedUrls.local[0];

const browser = await chromium.launch({ channel: "msedge" });

async function openWindow(launch, size = { width: 1000, height: 680 }, settings = {}) {
  const page = await browser.newPage({ viewport: size });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.addInitScript(mockBackend(launch, settings));
  await page.goto(url);
  await page.waitForTimeout(600);
  return { page, errors };
}

/** Whether a dialog fits the window with no scroll bar inside it. */
async function dialogFits(page, modalId) {
  return page.evaluate((id) => {
    const modal = document.getElementById(id);
    const box = modal.querySelector(".modal-window");
    const content = modal.querySelector(".modal-content");
    const rect = box.getBoundingClientRect();
    return {
      fitsWindow: rect.left >= 0 && rect.top >= 0 && rect.right <= innerWidth + 1 && rect.bottom <= innerHeight + 1,
      scrolls: content ? content.scrollHeight > content.clientHeight + 1 : false,
      width: Math.round(rect.width),
      height: Math.round(rect.height),
    };
  }, modalId);
}

// 2. Normal start: no script errors, the window is shown, the list fills.
{
  const { page, errors } = await openWindow(null);
  errors.length ? errors.forEach((e) => problem(`Script error at start: ${e}`)) : ok();
  const shown = await page.evaluate(() => window.__CALLS__.some((c) => c.cmd === "plugin:window|show"));
  shown ? ok() : problem("The window is never shown at start (it starts hidden).");
  const rows = await page.locator("#grid-body tr").count();
  rows === 3 ? ok() : problem(`The file list shows ${rows} rows instead of 3.`);

  // 3. Every dialog fits a normal window with no scroll bar.
  for (const id of ["modal-compress", "modal-extract", "modal-progress", "modal-rename", "modal-password", "modal-hash", "modal-about", "modal-benchmark", "modal-properties"]) {
    if (!(await page.locator("#" + id).count())) continue;
    await page.evaluate((m) => document.getElementById(m).classList.remove("hidden"), id);
    const fit = await dialogFits(page, id);
    fit.fitsWindow ? ok() : problem(`${id} does not fit inside the 1000x680 window.`);
    fit.scrolls ? problem(`${id} shows a scroll bar inside the dialog.`) : ok();
    await page.evaluate((m) => document.getElementById(m).classList.add("hidden"), id);
  }

  // 4. Password boxes in Add to Archive, for every format.
  await page.locator("#grid-body tr").nth(1).click();
  await page.locator("#btn-add").click();
  const formats = await page.$$eval("#select-format option", (o) => o.map((x) => x.value));
  for (const format of formats) {
    await page.selectOption("#select-format", format);
    const state = await page.evaluate(() => ({
      pw: !document.getElementById("compress-password").disabled,
      again: !document.getElementById("compress-password-again").disabled,
      show: !document.getElementById("compress-show-password").disabled,
      names: !document.getElementById("compress-encrypt-names").disabled,
    }));
    const should = format === "zip" || format === "7z";
    state.pw === should && state.again === should && state.show === should
      ? ok()
      : problem(`Format ${format}: password boxes are ${state.pw ? "on" : "off"} but should be ${should ? "on" : "off"}.`);
    state.names === (format === "7z") ? ok() : problem(`Format ${format}: "Encrypt file names" should be ${format === "7z" ? "on" : "off"}.`);
  }
  await page.selectOption("#select-format", "7z");
  await page.fill("#compress-password", "Secret-1");
  await page.fill("#compress-password-again", "Secret-1");
  await page.check("#compress-show-password");
  const kept = await page.evaluate(() => [
    document.getElementById("compress-password").value,
    document.getElementById("compress-password-again").value,
    document.getElementById("compress-password-again").disabled,
  ]);
  kept[0] === "Secret-1" && kept[1] === "Secret-1" && !kept[2]
    ? ok()
    : problem("Ticking Show password clears or locks the password boxes.");
  await page.fill("#compress-password-again", "Different");
  await page.click("#btn-start-compress");
  await page.waitForTimeout(200);
  const warned = await page.evaluate(() => window.__MESSAGES__.some((m) => /do not match/.test(m.message)));
  warned ? ok() : problem("Different passwords in the two boxes are not caught.");
  await page.close();
}

// 6. Inside an archive: folders open, Up goes back, Properties fits.
{
  const { page, errors } = await openWindow(null);
  await page.locator("#grid-body tr", { hasText: "photos.zip" }).dblclick();
  await page.waitForTimeout(300);
  const top = await page.$$eval("#grid-body tr", (rows) => rows.map((r) => r.textContent.trim().split(/\s+/)[0]));
  top.join(",") === "notes,photo.jpg" ? ok() : problem(`Archive top level shows [${top}] instead of the folder "notes" and photo.jpg.`);
  await page.locator("#grid-body tr", { hasText: "notes" }).dblclick();
  await page.waitForTimeout(200);
  const inside = await page.$$eval("#grid-body tr", (rows) => rows.map((r) => r.textContent.trim().split(/\s+/)[0]));
  inside.join(",") === "inner,a.txt" ? ok() : problem(`Folder "notes" inside the archive shows [${inside}] instead of inner and a.txt.`);
  const address = await page.inputValue("#input-path");
  address.endsWith("photos.zip\\notes\\") ? ok() : problem(`The address bar shows "${address}" inside an archive folder.`);
  const packed = await page.locator("#grid-body tr", { hasText: "a.txt" }).locator(".col-packed").textContent();
  packed.trim() === "" ? ok() : problem("A file in a solid block shows a packed size of its own.");
  await page.locator("#grid-body tr", { hasText: "a.txt" }).click();
  await page.locator("#btn-info").click();
  await page.waitForTimeout(300);
  const fit = await dialogFits(page, "modal-properties");
  fit.fitsWindow && !fit.scrolls ? ok() : problem("The Properties window does not fit, or shows a scroll bar.");
  const props = await page.textContent("#properties-table");
  ["SHA-256", "Recovery Data", "10%", "VSA Integrity Seal", "LZMA2"].every((t) => props.includes(t))
    ? ok()
    : problem("Properties lacks the .9z details (seal, recovery data or method).");
  await page.locator("#btn-properties-ok").click();
  await page.keyboard.press("Backspace");
  await page.waitForTimeout(200);
  const back = await page.$$eval("#grid-body tr", (rows) => rows.length);
  back === 2 ? ok() : problem("Backspace inside an archive folder does not go back up.");
  errors.length ? errors.forEach((e) => problem(`Archive browsing: script error: ${e}`)) : ok();
  await page.close();
}

// 7. Saved column widths: no needless horizontal scroll bar.
{
  const { page } = await openWindow(null, { width: 1000, height: 680 }, { columnWidths: { name: 250, size: 90, packed: 90, modified: 140, attributes: 70, crc: 80 } });
  await page.waitForTimeout(300);
  const scroll = await page.evaluate(() => {
    const c = document.getElementById("grid-container");
    return c.scrollWidth - c.clientWidth;
  });
  scroll <= 1 ? ok() : problem(`Narrow saved columns still leave a horizontal scroll bar (${scroll}px too wide).`);
  await page.close();
}

// 5. Dialogs started from Explorer fill their window exactly.
for (const [action, modalId] of [
  ["add", "modal-compress"],
  ["quick_9z", "modal-progress"],
]) {
  const target = "C:\\Users\\Test\\Desktop\\notes.txt";
  const { page, errors } = await openWindow({ action, targetPath: target, targetPaths: [target] });
  errors.length ? errors.forEach((e) => problem(`Explorer "${action}": script error: ${e}`)) : ok();
  const calls = await page.evaluate(() => window.__CALLS__);
  const sizes = calls.filter((c) => c.cmd === "plugin:window|set_size");
  if (!sizes.length) {
    problem(`Explorer "${action}": the window is never sized to the dialog.`);
  } else {
    const text = JSON.stringify(sizes[sizes.length - 1].args);
    const width = Number(text.match(/"width":([\d.]+)/)?.[1]);
    const height = Number(text.match(/"height":([\d.]+)/)?.[1]);
    // Look at the page at exactly that size, as Windows will show it.
    await page.setViewportSize({ width: Math.round(width), height: Math.round(height) });
    await page.waitForTimeout(150);
    const fit = await page.evaluate((id) => {
      const box = document.getElementById(id).querySelector(".modal-window").getBoundingClientRect();
      const content = document.getElementById(id).querySelector(".modal-content");
      return {
        gapRight: Math.round(innerWidth - box.right),
        gapBottom: Math.round(innerHeight - box.bottom),
        scrolls: content ? content.scrollHeight > content.clientHeight + 1 : false,
        pageScrolls: document.documentElement.scrollHeight > innerHeight + 1,
        mainHidden: getComputedStyle(document.getElementById("app")).display === "none",
      };
    }, modalId);
    Math.abs(fit.gapRight) <= 1 && Math.abs(fit.gapBottom) <= 1
      ? ok()
      : problem(`Explorer "${action}": ${fit.gapRight}px of empty space on the right and ${fit.gapBottom}px at the bottom.`);
    fit.scrolls || fit.pageScrolls ? problem(`Explorer "${action}": the dialog shows a scroll bar.`) : ok();
    fit.mainHidden ? ok() : problem(`Explorer "${action}": the main window shows behind the dialog.`);
    // Optional pictures for a human check: UI_SHOTS=<folder> npm run check:ui
    if (process.env.UI_SHOTS) await page.screenshot({ path: join(process.env.UI_SHOTS, `explorer-${action}.png`) });
    for (const flag of ["set_resizable", "set_maximizable"]) {
      const call = calls.find((c) => c.cmd === `plugin:window|${flag}`);
      call && JSON.stringify(call.args).includes("false")
        ? ok()
        : problem(`Explorer "${action}": the dialog window can still be ${flag === "set_resizable" ? "resized" : "maximised"}.`);
    }
  }
  await page.close();
}

await browser.close();
await server.close();

console.log(`\n9-zip window check: ${passed} checks passed, ${problems.length} problem(s).`);
for (const p of problems) console.log("  PROBLEM: " + p);
process.exit(problems.length ? 1 : 0);
