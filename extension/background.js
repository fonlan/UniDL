importScripts("background-utils.js", "background-services.js");

const MENU_SEND_LINK = "unidl-send-link";
const DOWNLOAD_CAPTURE_START_GRACE_MS = 10_000;
const extensionStartedAt = Date.now();
const handledDownloadIds = new Set();

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

chrome.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === MENU_SEND_LINK && info.linkUrl) {
    runTask(sendSource(info.linkUrl, undefined, undefined, info.pageUrl));
  }
});
chrome.downloads.onDeterminingFilename.addListener(handleDownloadFilename);
chrome.downloads.onCreated.addListener((download) =>
  runTask(handleDownloadCreated(download)),
);
chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  handleMessage(message)
    .then((payload) => sendResponse({ ok: true, payload }))
    .catch((error) => sendResponse({ ok: false, error: error.message }));
  return true;
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

function handleDownloadFilename(download, suggest) {
  let suggested = false;
  const safeSuggest = (nextSuggestion) => {
    if (suggested) {
      return;
    }
    suggested = true;
    suggest(nextSuggestion);
  };
  runTask(handleDownloadDeterminingFilename(download, safeSuggest));
  return true;
}

async function handleDownloadDeterminingFilename(download, suggest) {
  let captured = false;
  try {
    const settings = await getSettings();
    if (!shouldCaptureDownload(download, settings)) {
      suggest({});
      return;
    }
    captured = true;
    handledDownloadIds.add(download.id);
    suggest(
      settings.cancelOriginal
        ? { filename: download.filename, conflictAction: "overwrite" }
        : {},
    );
    if (settings.cancelOriginal) {
      await cancelDownload(download.id);
    }
    const task = await sendSource(
      download.url,
      settings,
      download.filename,
      download.referrer,
    );
    await remember(task.fileName + " sent to UniDL");
  } catch (error) {
    suggest({});
    throw error;
  } finally {
    if (captured) {
      forgetHandledDownload(download.id);
    }
  }
}

async function handleDownloadCreated(download) {
  if (handledDownloadIds.has(download.id)) {
    return;
  }
  const settings = await getSettings();
  if (!shouldCaptureDownload(download, settings)) {
    return;
  }
  handledDownloadIds.add(download.id);
  try {
    if (settings.cancelOriginal) {
      await cancelDownload(download.id);
    }
    const task = await sendSource(
      download.url,
      settings,
      download.filename,
      download.referrer,
    );
    await remember(task.fileName + " sent to UniDL");
  } finally {
    forgetHandledDownload(download.id);
  }
}

function shouldCaptureDownload(download, settings) {
  if (
    !settings.interceptEnabled ||
    !download.url ||
    download.byExtensionId === chrome.runtime.id ||
    !isActiveDownload(download)
  ) {
    return false;
  }
  if (isBelowMinCaptureSize(download, settings)) {
    return false;
  }
  return !shouldSkipCaptureByDomain(download.url, settings);
}

function forgetHandledDownload(downloadId) {
  setTimeout(() => handledDownloadIds.delete(downloadId), 5_000);
}

function isActiveDownload(download) {
  if (download.state !== "in_progress" || download.endTime) {
    return false;
  }

  const startTime = Date.parse(download.startTime ?? "");
  return (
    Number.isFinite(startTime) &&
    startTime >= extensionStartedAt - DOWNLOAD_CAPTURE_START_GRACE_MS
  );
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
