import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = readJson("package.json");
const tauriConfig = readJson("src-tauri/tauri.conf.json");
const manifest = readJson("extension/manifest.json");
const releaseExe = join(root, "release", "UniDL.exe");
const desktopExe = join(root, "src-tauri", "target", "release", "unidl.exe");
const bundleDir = join(root, "src-tauri", "target", "release", "bundle");
const extensionZip = join(root, "release", `UniDL-extension-v${manifest.version}.zip`);

let failed = false;

check("desktop build stays exe-only", () => {
  assertEqual(packageJson.scripts["build:desktop"], "tauri build --no-bundle");
  assertEqual(tauriConfig.bundle.active, false);
  assertEqual(tauriConfig.bundle.icon.includes("icons/icon.ico"), true);
  assertExists(desktopExe);
  assertExists(releaseExe);
  assertMissing(bundleDir);
  const main = readText("src-tauri/src/main.rs");
  assertIncludes(main, 'windows_subsystem = "windows"');
  assertExists(join(root, "src-tauri", "icons", "icon.ico"));
});

check("extension package contains the MV3 files", () => {
  assertExists(extensionZip);
  const zip = readFileSync(extensionZip);
  for (const file of [
    "manifest.json",
    "background.js",
    "popup.html",
    "popup.css",
    "popup.js",
  ]) {
    assertIncludes(zip, file);
  }
});

check("extension manifest has Chrome/Edge download hooks", () => {
  assertEqual(manifest.manifest_version, 3);
  for (const permission of ["contextMenus", "downloads", "storage"]) {
    assert(manifest.permissions.includes(permission), `missing ${permission}`);
  }
  assertEqual(manifest.background.service_worker, "background.js");
  assertEqual(manifest.action.default_popup, "popup.html");
});

check("task lifecycle API routes are wired", () => {
  const webServer = readText("src-tauri/src/web_server.rs");
  for (const route of [
    '"/api/tasks"',
    '"/api/tasks/pause"',
    '"/api/tasks/resume"',
    '"/api/tasks/delete"',
  ]) {
    assertIncludes(webServer, route);
  }
  const commands = readText("src-tauri/src/commands.rs");
  for (const command of [
    "create_download_task",
    "pause_download_tasks",
    "resume_download_tasks",
    "delete_download_tasks",
  ]) {
    assertIncludes(commands, command);
  }
});

check("torrent and magnet association chain is present", () => {
  assertEqual(tauriConfig.plugins["deep-link"].desktop.schemes.includes("magnet"), true);
  assertEqual(tauriConfig.bundle.fileAssociations[0].ext.includes("torrent"), true);
  const systemOpen = readText("src-tauri/src/system_open.rs");
  assertIncludes(systemOpen, "register_torrent_file_association");
  assertIncludes(systemOpen, "starts_with(\"magnet:\")");
  assertIncludes(systemOpen, "ends_with(\".torrent\")");
});

check("web access auth and event endpoints are present", () => {
  const webServer = readText("src-tauri/src/web_server.rs");
  for (const item of [
    '"/api/health"',
    '"/api/login"',
    '"/api/events"',
    "authorization",
    "x-unidl-token",
    "text/event-stream",
  ]) {
    assertIncludes(webServer, item);
  }
});

check("browser interception sends tasks to local Web API", () => {
  const background = readText("extension/background.js");
  for (const item of [
    "chrome.downloads.onCreated",
    "chrome.contextMenus.onClicked",
    '"/api/health"',
    '"/api/extension/tasks"',
    "cancelDownload",
  ]) {
    assertIncludes(background, item);
  }
  for (const item of [
    "UniDL access password is required",
    '"/api/login"',
    "Bearer",
    "defaultEngine",
    "manual-source",
  ]) {
    assertExcludes(background, item);
  }
});

if (failed) {
  process.exit(1);
}

function check(label, run) {
  try {
    run();
    console.log(`ok ${label}`);
  } catch (error) {
    failed = true;
    console.error(`fail ${label}: ${error.message}`);
  }
}

function readJson(path) {
  return JSON.parse(readText(path));
}

function readText(path) {
  return readFileSync(join(root, path), "utf8");
}

function assert(value, message) {
  if (!value) {
    throw new Error(message);
  }
}

function assertEqual(actual, expected) {
  if (actual !== expected) {
    throw new Error(`expected ${expected}, got ${actual}`);
  }
}

function assertIncludes(value, needle) {
  const found = Buffer.isBuffer(value)
    ? value.includes(Buffer.from(needle))
    : value.includes(needle);
  if (!found) {
    throw new Error(`missing ${needle}`);
  }
}

function assertExcludes(value, needle) {
  const found = Buffer.isBuffer(value)
    ? value.includes(Buffer.from(needle))
    : value.includes(needle);
  if (found) {
    throw new Error(`unexpected ${needle}`);
  }
}

function assertExists(path) {
  if (!existsSync(path)) {
    throw new Error(`missing ${path}`);
  }
}

function assertMissing(path) {
  if (existsSync(path)) {
    throw new Error(`unexpected ${path}`);
  }
}
