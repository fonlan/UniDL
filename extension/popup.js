const fields = {
  apiBaseUrl: document.querySelector("#api-base-url"),
  interceptEnabled: document.querySelector("#intercept-enabled"),
  cancelOriginal: document.querySelector("#cancel-original"),
  lastEvent: document.querySelector("#last-event"),
  status: document.querySelector("#status"),
};

document.querySelector("#connect").addEventListener("click", () => {
  void connect();
});

for (const field of [fields.interceptEnabled, fields.cancelOriginal]) {
  field.addEventListener("change", () => {
    void run("设置已保存", () => sendMessage("save-settings", { settings: collectSettings() }));
  });
}

void load();

async function load() {
  await run("", async () => {
    const settings = await sendMessage("get-state");
    applySettings(settings);
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
      applySettings(result);
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

function applySettings(settings) {
  fields.apiBaseUrl.value = settings.apiBaseUrl ?? "";
  fields.interceptEnabled.checked = Boolean(settings.interceptEnabled);
  fields.cancelOriginal.checked = settings.cancelOriginal !== false;
  fields.lastEvent.textContent = settings.lastEvent || "未连接";
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
