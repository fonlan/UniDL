async function sendSource(source, cachedSettings, suggestedFileName, referrerUrl) {
  const settings = cachedSettings ?? (await getSettings());
  const parsed = parseSource(source, suggestedFileName);
  const httpReferrer = normalizeHttpUrl(referrerUrl);
  const task = await requestJson(settings.apiBaseUrl, "/api/extension/tasks", {
    method: "POST",
    body: {
      sourceType: parsed.sourceType,
      source: parsed.source,
      fileName: parsed.fileName,
      httpReferrer,
    },
  });
  await remember(task.fileName + " sent to UniDL");
  return task;
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

async function cancelDownload(downloadId) {
  await new Promise((resolve) => chrome.downloads.cancel(downloadId, resolve));
}

async function remember(message) {
  await new Promise((resolve) =>
    chrome.storage.local.set({ lastEvent: message }, resolve),
  );
}
