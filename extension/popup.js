const fields = {
  interceptEnabled: document.querySelector("#intercept-enabled"),
  detectVideos: document.querySelector("#detect-videos"),
  openSettings: document.querySelector("#open-settings"),
  status: document.querySelector("#status"),
  videos: document.querySelector("#videos"),
};

let videos = [];
let canDetectVideos = false;
let videoDetectionDone = false;
let isDetecting = false;

fields.openSettings.addEventListener("click", () => {
  if (chrome.runtime.openOptionsPage) {
    chrome.runtime.openOptionsPage();
    return;
  }
  chrome.tabs.create({ url: chrome.runtime.getURL("settings.html") });
});

fields.detectVideos.addEventListener("click", () => {
  if (isDetecting) {
    return;
  }
  isDetecting = true;
  renderVideos();
  void run("\u68c0\u6d4b\u5b8c\u6210", () =>
    sendMessage("detect-videos", { settings: collectSettings() }),
  ).finally(() => {
    isDetecting = false;
    renderVideos();
  });
});

fields.interceptEnabled.addEventListener("change", () => {
  void run("\u8bbe\u7f6e\u5df2\u4fdd\u5b58", () =>
    sendMessage("save-settings", { settings: collectSettings() }),
  );
});

fields.videos.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-video-id]");
  if (!button) {
    return;
  }
  const video = videos.find((item) => item.id === button.dataset.videoId);
  if (video) {
    void run("\u5df2\u53d1\u9001\u5230 UniDL", () =>
      sendMessage("download-video", { video }),
    );
  }
});

void load();

async function load() {
  await run("", async () => {
    const state = await sendMessage("get-state");
    applyState(state);
  });
}

async function run(successMessage, action) {
  setStatus("", "");
  try {
    const result = await action();
    if (result && result.apiBaseUrl) {
      applyState(result);
    }
    if (successMessage) {
      setStatus(successMessage, "ok");
    }
  } catch (error) {
    setStatus(error.message, "error");
  }
}

function collectSettings() {
  return {
    interceptEnabled: fields.interceptEnabled.checked,
  };
}

function applyState(state) {
  fields.interceptEnabled.checked = Boolean(state.interceptEnabled);
  videos = Array.isArray(state.videos) ? state.videos : [];
  canDetectVideos = Boolean(state.canDetectVideos);
  videoDetectionDone = Boolean(state.videoDetectionDone);
  renderVideos();
}

function renderVideos() {
  fields.videos.textContent = "";
  fields.detectVideos.hidden = !canDetectVideos;
  fields.detectVideos.disabled = !canDetectVideos || isDetecting;
  fields.detectVideos.textContent = isDetecting
    ? "\u68c0\u6d4b\u4e2d\u2026"
    : "\u68c0\u6d4b\u5f53\u524d\u9875\u9762";
  if (isDetecting) {
    const loading = document.createElement("div");
    loading.className = "detecting";
    const spinner = document.createElement("span");
    spinner.className = "spinner";
    spinner.setAttribute("aria-hidden", "true");
    const text = document.createElement("span");
    text.textContent =
      "\u6b63\u5728\u8c03\u7528 yt-dlp \u68c0\u6d4b\u5f53\u524d\u9875\u9762\u2026";
    loading.append(spinner, text);
    fields.videos.append(loading);
    return;
  }
  if (!canDetectVideos) {
    const empty = document.createElement("p");
    empty.textContent =
      "\u5f53\u524d\u9875\u9762\u4e0d\u53ef\u68c0\u6d4b\uff0c\u6216\u672a\u914d\u7f6e\u53ef\u7528 yt-dlp \u5f15\u64ce";
    fields.videos.append(empty);
    return;
  }
  if (!videoDetectionDone) {
    const empty = document.createElement("p");
    empty.textContent =
      "\u70b9\u51fb\u201c\u68c0\u6d4b\u5f53\u524d\u9875\u9762\u201d\u67e5\u627e\u53ef\u4e0b\u8f7d\u89c6\u9891";
    fields.videos.append(empty);
    return;
  }
  if (videos.length === 0) {
    const empty = document.createElement("p");
    empty.textContent = "\u672a\u8bc6\u522b\u5230\u53ef\u4e0b\u8f7d\u89c6\u9891";
    fields.videos.append(empty);
    return;
  }
  for (const video of videos) {
    const button = document.createElement("button");
    button.type = "button";
    button.dataset.videoId = video.id;
    button.textContent = video.title || video.source;
    fields.videos.append(button);
  }
}

function sendMessage(type, payload = {}) {
  return new Promise((resolve, reject) => {
    chrome.runtime.sendMessage({ type, ...payload }, (response) => {
      if (chrome.runtime.lastError) {
        reject(new Error(chrome.runtime.lastError.message));
        return;
      }
      if (!response?.ok) {
        reject(new Error(response?.error ?? "UniDL extension request failed"));
        return;
      }
      resolve(response.payload);
    });
  });
}

function setStatus(message, tone) {
  fields.status.textContent = message;
  fields.status.className = tone;
}
