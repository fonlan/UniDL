const MENU_SEND_LINK = "unidl-send-link";

const DEFAULT_SETTINGS = {
  apiBaseUrl: "http://127.0.0.1:18080",
  password: "",
  token: "",
  defaultEngine: "aria2",
  savePath: "",
  engineArgs: "",
  interceptEnabled: false,
  cancelOriginal: false,
  lastEvent: "",
};

const ENGINE_SOURCES = {
  aria2: new Set(["http", "ftp", "magnet", "torrent"]),
  "yt-dlp": new Set(["http", "ftp"]),
  qbittorrent: new Set(["magnet", "torrent"]),
};

chrome.runtime.onInstalled.addListener(createContextMenus);
chrome.runtime.onStartup.addListener(createContextMenus);

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

chrome.downloads.onCreated.addListener((download) => {
  runTask(handleDownload(download));
});

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  handleMessage(message)
    .then((payload) => sendResponse({ ok: true, payload }))
    .catch((error) => sendResponse({ ok: false, error: error.message }));
  return true;
});

async function handleMessage(message) {
  switch (message?.type) {
    case "get-state":
      return getSettings();
    case "save-settings": {
      const settings = await mergeSettings(message.settings ?? {});
      await saveSettings(settings);
      return settings;
    }
    case "connect": {
      const settings = await mergeSettings(message.settings ?? {});
      const token = await login(settings);
      const next = { ...settings, token };
      await saveSettings(next);
      await remember("Connected to UniDL");
      return next;
    }
    case "clear-token": {
      const settings = await getSettings();
      const next = { ...settings, token: "" };
      await saveSettings(next);
      return next;
    }
    case "send-source":
      return sendSource(message.source);
    default:
      throw new Error("Unknown message");
  }
}

async function handleDownload(download) {
  const settings = await getSettings();
  if (!settings.interceptEnabled) {
    return;
  }
  if (!download.url || download.byExtensionId === chrome.runtime.id) {
    return;
  }

  const task = await sendSource(download.url, settings, download.filename);
  if (settings.cancelOriginal) {
    await cancelDownload(download.id);
  }
  await remember(`Sent ${task.fileName} to UniDL`);
}

function runTask(task) {
  task.catch((error) => {
    void remember(`UniDL error: ${error.message}`);
  });
}

async function sendSource(source, cachedSettings, suggestedFileName) {
  const settings = cachedSettings ?? (await getSettings());
  const parsed = parseSource(source, suggestedFileName);
  validateTaskSettings(settings, parsed);

  const task = await requestWithAuth(settings, "/api/tasks", {
    method: "POST",
    body: {
      sourceType: parsed.sourceType,
      source: parsed.source,
      engine: settings.defaultEngine,
      fileName: parsed.fileName,
      savePath: settings.savePath,
      engineArgs: settings.engineArgs,
    },
  });
  await remember(`Sent ${task.fileName} to UniDL`);
  return task;
}

function validateTaskSettings(settings, parsed) {
  if (!settings.savePath.trim()) {
    throw new Error("UniDL save path is required");
  }
  if (!ENGINE_SOURCES[settings.defaultEngine]?.has(parsed.sourceType)) {
    throw new Error(`${settings.defaultEngine} does not support ${parsed.sourceType}`);
  }
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
      fileName: cleanFileName(suggestedFileName) ?? parseUrlName(source) ?? "http-download",
    };
  }

  if (/^ftp:\/\//i.test(source)) {
    return {
      sourceType: "ftp",
      source,
      fileName: cleanFileName(suggestedFileName) ?? parseUrlName(source) ?? "ftp-download",
    };
  }

  if (/\.torrent(?:$|[?#])/i.test(source)) {
    return {
      sourceType: "torrent",
      source,
      fileName: cleanFileName(suggestedFileName) ?? parsePathName(source) ?? "download.torrent",
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

function parsePathName(value) {
  return cleanFileName(String(value).split(/[?#]/)[0].split(/[\\/]/).filter(Boolean).at(-1));
}

function cleanFileName(value) {
  const name = String(value ?? "").split(/[\\/]/).filter(Boolean).at(-1)?.trim();
  if (!name) {
    return null;
  }
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

async function requestWithAuth(settings, path, options = {}) {
  const token = settings.token || (await loginAndPersist(settings));
  try {
    return await requestJson(settings.apiBaseUrl, path, {
      ...options,
      token,
    });
  } catch (error) {
    if (!/unauthorized|invalid password/i.test(error.message)) {
      throw error;
    }
    const nextToken = await loginAndPersist({ ...settings, token: "" });
    return requestJson(settings.apiBaseUrl, path, {
      ...options,
      token: nextToken,
    });
  }
}

async function loginAndPersist(settings) {
  const token = await login(settings);
  await saveSettings({ ...settings, token });
  return token;
}

async function login(settings) {
  if (!settings.password.trim()) {
    throw new Error("UniDL access password is required");
  }
  const response = await requestJson(settings.apiBaseUrl, "/api/login", {
    method: "POST",
    body: { password: settings.password },
  });
  if (!response.token) {
    throw new Error("UniDL login response missing token");
  }
  return response.token;
}

async function requestJson(apiBaseUrl, path, options = {}) {
  const headers = { "content-type": "application/json" };
  if (options.token) {
    headers.authorization = `Bearer ${options.token}`;
  }

  const response = await fetch(`${trimBaseUrl(apiBaseUrl)}${path}`, {
    method: options.method ?? "GET",
    headers,
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  const text = await response.text();
  const payload = text ? JSON.parse(text) : {};
  if (!response.ok) {
    throw new Error(payload.error ?? `UniDL API failed: ${response.status}`);
  }
  return payload;
}

function trimBaseUrl(value) {
  const url = String(value ?? "").trim().replace(/\/+$/, "");
  if (!url) {
    throw new Error("UniDL API URL is required");
  }
  return url;
}

function getSettings() {
  return new Promise((resolve) => {
    chrome.storage.local.get(DEFAULT_SETTINGS, (items) => {
      resolve(normalizeSettings(items));
    });
  });
}

function saveSettings(settings) {
  return new Promise((resolve) => {
    chrome.storage.local.set(normalizeSettings(settings), resolve);
  });
}

async function mergeSettings(settings) {
  const current = await getSettings();
  return normalizeSettings({ ...current, ...settings });
}

function normalizeSettings(settings) {
  const next = { ...DEFAULT_SETTINGS, ...settings };
  next.apiBaseUrl = trimBaseUrl(next.apiBaseUrl);
  next.password = String(next.password ?? "");
  next.token = String(next.token ?? "");
  next.defaultEngine = ENGINE_SOURCES[next.defaultEngine] ? next.defaultEngine : "aria2";
  next.savePath = String(next.savePath ?? "");
  next.engineArgs = String(next.engineArgs ?? "");
  next.interceptEnabled = Boolean(next.interceptEnabled);
  next.cancelOriginal = Boolean(next.cancelOriginal);
  next.lastEvent = String(next.lastEvent ?? "");
  return next;
}

async function cancelDownload(downloadId) {
  await new Promise((resolve) => {
    chrome.downloads.cancel(downloadId, resolve);
  });
}

async function remember(message) {
  await new Promise((resolve) => {
    chrome.storage.local.set({ lastEvent: message }, resolve);
  });
}
