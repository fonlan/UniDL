const MENU_SEND_LINK = "unidl-send-link";

const DEFAULT_SETTINGS = {
  apiBaseUrl: "http://127.0.0.1:18080",
  interceptEnabled: false,
  cancelOriginal: true,
  minCaptureSizeMb: 3,
  skipCaptureDomains: [],
  lastEvent: "",
};

chrome.runtime.onInstalled.addListener(() => {
  createContextMenus();
  runTask(clearActiveTabBadge());
});
chrome.runtime.onStartup.addListener(() => {
  createContextMenus();
  runTask(clearActiveTabBadge());
});
chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  if (changeInfo.url) {
    runTask(setBadge(tabId, 0));
  }
});

function createContextMenus() {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: MENU_SEND_LINK,
      title: "Send to UniDL",
      contexts: ["link"],
    });
  });
}

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === MENU_SEND_LINK && info.linkUrl) {
    runTask(sendSource(info.linkUrl));
  }
});
chrome.downloads.onCreated.addListener((download) => runTask(handleDownload(download)));
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  handleMessage(message)
    .then((payload) => sendResponse({ ok: true, payload }))
    .catch((error) => sendResponse({ ok: false, error: error.message }));
  return true;
});

async function handleMessage(message) {
  switch (message?.type) {
    case "get-state":
      return getPopupState();
    case "get-settings":
      return getSettings();
    case "save-settings": {
      const settings = await mergeSettings(message.settings ?? {});
      await saveSettings(settings);
      return getPopupState(settings);
    }
    case "connect": {
      const settings = await mergeSettings(message.settings ?? {});
      await requestJson(settings.apiBaseUrl, "/api/health");
      await saveSettings(settings);
      await remember("Connected to UniDL");
      return getPopupState(settings);
    }
    case "detect-videos": {
      const settings = await mergeSettings(message.settings ?? {});
      await saveSettings(settings);
      return detectActiveTabVideos(settings);
    }
    case "download-video":
      return downloadVideo(message.video);
    default:
      throw new Error("Unknown message");
  }
}

async function getPopupState(cachedSettings) {
  const settings = cachedSettings ?? (await getSettings());
  const canDetectVideos = await canDetectActiveTabVideos(settings);
  return { ...settings, videos: [], canDetectVideos, videoDetectionDone: false };
}

async function clearActiveTabBadge() {
  const tab = await getActiveTab();
  if (tab?.id) {
    await setBadge(tab.id, 0);
  }
}

async function canDetectActiveTabVideos(settings) {
  const { source } = await getActiveVideoTab();
  if (!source) {
    return false;
  }
  try {
    const result = await requestJson(
      settings.apiBaseUrl,
      "/api/extension/videos/support",
      {
        method: "POST",
        body: { source },
      },
    );
    return Boolean(result.canDetectVideos);
  } catch {
    return false;
  }
}

async function detectActiveTabVideos(settings) {
  const { tab, source } = await getActiveVideoTab();
  if (!tab?.id || !source) {
    return { ...settings, videos: [], canDetectVideos: false, videoDetectionDone: true };
  }
  const cookies = await exportCookies(source);
  const result = await requestJson(settings.apiBaseUrl, "/api/extension/videos", {
    method: "POST",
    body: { source, title: tab.title ?? "", cookies },
  });
  const videos = Array.isArray(result.videos) ? result.videos : [];
  return { ...settings, videos, canDetectVideos: true, videoDetectionDone: true };
}

async function getActiveVideoTab() {
  const tab = await getActiveTab();
  if (!tab?.id) {
    return { tab: null, source: null };
  }
  const source = normalizeHttpUrl(tab.url ?? (await getTabUrl(tab.id)));
  return { tab, source };
}

async function downloadVideo(video) {
  const source = normalizeHttpUrl(video?.source);
  if (!source) {
    throw new Error("Video URL is required");
  }
  const fileName = cleanFileName(video?.title) ?? parseUrlName(source) ?? "video";
  const cookies = await exportCookies(source);
  const task = await requestJson(
    (await getSettings()).apiBaseUrl,
    "/api/extension/ytdlp/tasks",
    {
      method: "POST",
      body: { source, fileName, cookies },
    },
  );
  await remember(task.fileName + " sent to UniDL");
  return task;
}

async function handleDownload(download) {
  const settings = await getSettings();
  if (
    !settings.interceptEnabled ||
    !download.url ||
    download.byExtensionId === chrome.runtime.id
  ) {
    return;
  }
  if (isBelowMinCaptureSize(download, settings)) {
    return;
  }
  if (shouldSkipCaptureByDomain(download.url, settings)) {
    return;
  }
  const task = await sendSource(download.url, settings, download.filename);
  if (settings.cancelOriginal) {
    await cancelDownload(download.id);
  }
  await remember(task.fileName + " sent to UniDL");
}

function shouldSkipCaptureByDomain(source, settings) {
  if (!settings.skipCaptureDomains.length) {
    return false;
  }
  let hostname;
  try {
    hostname = new URL(source).hostname.toLowerCase();
  } catch {
    return false;
  }
  return settings.skipCaptureDomains.some(
    (domain) => hostname === domain || hostname.endsWith("." + domain),
  );
}

function isBelowMinCaptureSize(download, settings) {
  const size = getKnownDownloadSize(download);
  if (size === null) {
    return false;
  }
  return size < settings.minCaptureSizeMb * 1024 * 1024;
}

function getKnownDownloadSize(download) {
  for (const field of [download.fileSize, download.totalBytes]) {
    const size = Number(field);
    if (Number.isFinite(size) && size >= 0) {
      return size;
    }
  }
  return null;
}

function runTask(task) {
  task.catch((error) => void remember("UniDL error: " + error.message));
}

async function sendSource(source, cachedSettings, suggestedFileName) {
  const settings = cachedSettings ?? (await getSettings());
  const parsed = parseSource(source, suggestedFileName);
  const task = await requestJson(settings.apiBaseUrl, "/api/extension/tasks", {
    method: "POST",
    body: {
      sourceType: parsed.sourceType,
      source: parsed.source,
      fileName: parsed.fileName,
    },
  });
  await remember(task.fileName + " sent to UniDL");
  return task;
}

function parseSource(value, suggestedFileName) {
  const source = String(value ?? "").trim();
  if (!source) {
    throw new Error("Download source is required");
  }
  if (/^magnet:/i.test(source)) {
    return {
      sourceType: "magnet",
      source,
      fileName: parseMagnetName(source) ?? "magnet",
    };
  }
  if (/^https?:\/\//i.test(source)) {
    return {
      sourceType: "http",
      source,
      fileName: parseNetworkFileName(source, suggestedFileName) ?? "http-download",
    };
  }
  if (/^ftp:\/\//i.test(source)) {
    return {
      sourceType: "ftp",
      source,
      fileName: parseNetworkFileName(source, suggestedFileName) ?? "ftp-download",
    };
  }
  if (source.toLowerCase().split(/[?#]/)[0].endsWith(".torrent")) {
    return {
      sourceType: "torrent",
      source,
      fileName:
        cleanFileName(suggestedFileName) ?? parsePathName(source) ?? "download.torrent",
    };
  }
  throw new Error("Unsupported download source");
}

function parseMagnetName(value) {
  const match = /(?:[?&])dn=([^&]+)/i.exec(value);
  if (!match) {
    return null;
  }
  try {
    return decodeURIComponent(match[1].replace(/\+/g, " "));
  } catch {
    return match[1];
  }
}

function parseUrlName(value) {
  try {
    const url = new URL(value);
    return cleanFileName(url.pathname.split("/").filter(Boolean).at(-1));
  } catch {
    return parsePathName(value);
  }
}

function parseNetworkFileName(source, suggestedFileName) {
  const parsedUrl = new URL(source);
  const suggested = cleanFileName(suggestedFileName);
  const hostname = parsedUrl.hostname.toLowerCase();
  if (suggested && suggested.toLowerCase() !== hostname) {
    return suggested;
  }

  return cleanFileName(parsedUrl.pathname.split("/").filter(Boolean).at(-1));
}

function parsePathName(value) {
  return cleanFileName(
    String(value).split(/[?#]/)[0].split(/[\\/]/).filter(Boolean).at(-1),
  );
}

function cleanFileName(value) {
  const name = String(value ?? "")
    .split(/[\\/]/)
    .filter(Boolean)
    .at(-1)
    ?.trim();
  if (!name) {
    return null;
  }
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

async function exportCookies(source) {
  const cookies = await new Promise((resolve, reject) => {
    chrome.cookies.getAll({ url: source }, (items) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      resolve(items);
    });
  });
  if (!cookies.length) {
    return "";
  }
  const lines = ["# Netscape HTTP Cookie File"];
  for (const cookie of cookies) {
    const domain = cookie.httpOnly ? "#HttpOnly_" + cookie.domain : cookie.domain;
    const includeSubdomains = cookie.domain.startsWith(".") ? "TRUE" : "FALSE";
    const path = cookie.path || "/";
    const secure = cookie.secure ? "TRUE" : "FALSE";
    const expires = cookie.session ? "0" : String(Math.trunc(cookie.expirationDate ?? 0));
    lines.push(
      [domain, includeSubdomains, path, secure, expires, cookie.name, cookie.value].join(
        "\t",
      ),
    );
  }
  return lines.join("\n") + "\n";
}

async function requestJson(apiBaseUrl, path, options = {}) {
  const response = await fetch(trimBaseUrl(apiBaseUrl) + path, {
    method: options.method ?? "GET",
    headers: { "content-type": "application/json" },
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const text = await response.text();
  const payload = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new Error(payload.error ?? "UniDL API failed: " + response.status);
  }
  return payload;
}

function trimBaseUrl(value) {
  let text = String(value ?? "").trim();
  while (text.endsWith("/")) {
    text = text.slice(0, -1);
  }
  if (!text) {
    throw new Error("UniDL API URL is required");
  }
  const url = new URL(text);
  if (url.hostname === "localhost") {
    url.hostname = "127.0.0.1";
  }
  text = url.toString();
  while (text.endsWith("/")) {
    text = text.slice(0, -1);
  }
  return text;
}

function normalizeHttpUrl(value) {
  const source = String(value ?? "").trim();
  return /^https?:\/\//i.test(source) ? source : null;
}

function getActiveTab() {
  return new Promise((resolve) => {
    chrome.tabs.query({ active: true, currentWindow: true }, (tabs) =>
      resolve(tabs[0] ?? null),
    );
  });
}

function getTabUrl(tabId) {
  return new Promise((resolve) => {
    chrome.tabs.get(tabId, (tab) => {
      if (chrome.runtime.lastError) {
        resolve(null);
        return;
      }
      resolve(tab.url ?? null);
    });
  });
}

function setBadge(tabId, count) {
  return new Promise((resolve) => {
    chrome.action.setBadgeBackgroundColor({ tabId, color: "#047857" }, () => {
      chrome.action.setBadgeText(
        { tabId, text: count > 0 ? String(count) : "" },
        resolve,
      );
    });
  });
}

function getSettings() {
  return new Promise((resolve) =>
    chrome.storage.local.get(DEFAULT_SETTINGS, (items) =>
      resolve(normalizeSettings(items)),
    ),
  );
}

function saveSettings(settings) {
  return new Promise((resolve) =>
    chrome.storage.local.set(normalizeSettings(settings), resolve),
  );
}

async function mergeSettings(settings) {
  const current = await getSettings();
  return normalizeSettings({ ...current, ...settings });
}

function normalizeSettings(settings) {
  const next = { ...DEFAULT_SETTINGS, ...settings };
  next.apiBaseUrl = trimBaseUrl(next.apiBaseUrl);
  next.interceptEnabled = Boolean(next.interceptEnabled);
  next.cancelOriginal = Boolean(next.cancelOriginal);
  const minCaptureSizeMb = Number(next.minCaptureSizeMb);
  next.minCaptureSizeMb =
    Number.isFinite(minCaptureSizeMb) && minCaptureSizeMb >= 0
      ? minCaptureSizeMb
      : DEFAULT_SETTINGS.minCaptureSizeMb;
  next.skipCaptureDomains = normalizeDomainList(next.skipCaptureDomains);
  next.lastEvent = String(next.lastEvent ?? "");
  return next;
}

function normalizeDomainList(value) {
  const entries = Array.isArray(value) ? value : String(value ?? "").split(/[\n,]/);
  const domains = [];
  for (const entry of entries) {
    const domain = normalizeDomainEntry(entry);
    if (domain && !domains.includes(domain)) {
      domains.push(domain);
    }
  }
  return domains;
}

function normalizeDomainEntry(value) {
  const text = String(value ?? "").trim().toLowerCase();
  if (!text) {
    return null;
  }
  try {
    const url = new URL(/^\w+:\/\//.test(text) ? text : "http://" + text);
    return trimDomain(url.hostname);
  } catch {
    return trimDomain(text.split(/[/?#]/)[0]);
  }
}

function trimDomain(value) {
  const domain = String(value ?? "")
    .trim()
    .replace(/^\*\./, "")
    .replace(/^\.+|\.+$/g, "");
  return domain || null;
}

async function cancelDownload(downloadId) {
  await new Promise((resolve) => chrome.downloads.cancel(downloadId, resolve));
}

async function remember(message) {
  await new Promise((resolve) =>
    chrome.storage.local.set({ lastEvent: message }, resolve),
  );
}
