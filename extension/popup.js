const fields = {
  apiBaseUrl: document.querySelector("#api-base-url"),
  password: document.querySelector("#password"),
  defaultEngine: document.querySelector("#default-engine"),
  savePath: document.querySelector("#save-path"),
  engineArgs: document.querySelector("#engine-args"),
  interceptEnabled: document.querySelector("#intercept-enabled"),
  cancelOriginal: document.querySelector("#cancel-original"),
  manualSource: document.querySelector("#manual-source"),
  lastEvent: document.querySelector("#last-event"),
  status: document.querySelector("#status"),
};

document.querySelector("#connect").addEventListener("click", () => {
  void run("连接成功", () => sendMessage("connect", { settings: collectSettings() }));
});

document.querySelector("#save-settings").addEventListener("click", () => {
  void run("设置已保存", () => sendMessage("save-settings", { settings: collectSettings() }));
});

document.querySelector("#clear-token").addEventListener("click", () => {
  void run("令牌已清除", () => sendMessage("clear-token"));
});

document.querySelector("#send-source").addEventListener("click", () => {
  void run("已发送到 UniDL", () =>
    sendMessage("send-source", { source: fields.manualSource.value }),
  );
});

void load();

async function load() {
  await run("", async () => {
    const settings = await sendMessage("get-state");
    applySettings(settings);
  });
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
    password: fields.password.value,
    defaultEngine: fields.defaultEngine.value,
    savePath: fields.savePath.value,
    engineArgs: fields.engineArgs.value,
    interceptEnabled: fields.interceptEnabled.checked,
    cancelOriginal: fields.cancelOriginal.checked,
  };
}

function applySettings(settings) {
  fields.apiBaseUrl.value = settings.apiBaseUrl ?? "";
  fields.password.value = settings.password ?? "";
  fields.defaultEngine.value = settings.defaultEngine ?? "aria2";
  fields.savePath.value = settings.savePath ?? "";
  fields.engineArgs.value = settings.engineArgs ?? "";
  fields.interceptEnabled.checked = Boolean(settings.interceptEnabled);
  fields.cancelOriginal.checked = Boolean(settings.cancelOriginal);
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
