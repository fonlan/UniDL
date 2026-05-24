const fields = {
  apiBaseUrl: document.querySelector("#api-base-url"),
  interceptEnabled: document.querySelector("#intercept-enabled"),
  cancelOriginal: document.querySelector("#cancel-original"),
  lastEvent: document.querySelector("#last-event"),
  status: document.querySelector("#status"),
  videos: document.querySelector("#videos"),
};

let videos = [];

document.querySelector("#connect").addEventListener("click", () => {
  void connect();
});

for (const field of [fields.interceptEnabled, fields.cancelOriginal]) {
  field.addEventListener("change", () => {
    void run("设置已保存", () => sendMessage("save-settings", { settings: collectSettings() }));
  });
}

fields.videos.addEventListener("click", (event) => {
  const button = event.target.closest("button[data-video-id]");
  if (!button) {
    return;
  }
  const video = videos.find((item) => item.id === button.dataset.videoId);
  if (video) {
    void run("已发送到 UniDL", () => sendMessage("download-video", { video }));
  }
});

void load();

async function load() {
  await run("", async () => {
    const state = await sendMessage("get-state");
    applyState(state);
  });
  await connect();
}

async function connect() {
  await run("已连接 UniDL", () => sendMessage("connect", { settings: collectSettings() }));
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
    apiBaseUrl: fields.apiBaseUrl.value,
    interceptEnabled: fields.interceptEnabled.checked,
    cancelOriginal: fields.cancelOriginal.checked,
  };
}

function applyState(state) {
  fields.apiBaseUrl.value = state.apiBaseUrl ?? "";
  fields.interceptEnabled.checked = Boolean(state.interceptEnabled);
  fields.cancelOriginal.checked = state.cancelOriginal !== false;
  fields.lastEvent.textContent = state.lastEvent || "未连接";
  videos = Array.isArray(state.videos) ? state.videos : [];
  renderVideos();
}

function renderVideos() {
  fields.videos.textContent = "";
  if (videos.length === 0) {
    const empty = document.createElement("p");
    empty.textContent = "当前页面未识别到 yt-dlp 视频";
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