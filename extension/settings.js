const fields = {
  apiBaseUrl: document.querySelector("#api-base-url"),
  cancelOriginal: document.querySelector("#cancel-original"),
  connect: document.querySelector("#connect"),
  minCaptureSizeMb: document.querySelector("#min-capture-size-mb"),
  skipCaptureDomains: document.querySelector("#skip-capture-domains"),
  status: document.querySelector("#status"),
};

fields.connect.addEventListener("click", () => {
  void connect();
});

fields.apiBaseUrl.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    void connect();
  }
});

fields.cancelOriginal.addEventListener("change", () => {
  void run("\u8bbe\u7f6e\u5df2\u4fdd\u5b58", () =>
    sendMessage("save-settings", { settings: collectSettings() }),
  );
});

fields.minCaptureSizeMb.addEventListener("change", () => {
  void run("\u8bbe\u7f6e\u5df2\u4fdd\u5b58", () =>
    sendMessage("save-settings", { settings: collectSettings() }),
  );
});

fields.skipCaptureDomains.addEventListener("change", () => {
  void run("\u8bbe\u7f6e\u5df2\u4fdd\u5b58", () =>
    sendMessage("save-settings", { settings: collectSettings() }),
  );
});

void load();

async function load() {
  await run("", async () => {
    const settings = await sendMessage("get-settings");
    applySettings(settings);
  });
}

async function connect() {
  await run("\u5df2\u8fde\u63a5 UniDL", () =>
    sendMessage("connect", { settings: collectSettings() }),
  );
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
    cancelOriginal: fields.cancelOriginal.checked,
    minCaptureSizeMb: Number(fields.minCaptureSizeMb.value),
    skipCaptureDomains: fields.skipCaptureDomains.value,
  };
}

function applySettings(settings) {
  fields.apiBaseUrl.value = settings.apiBaseUrl ?? "";
  fields.cancelOriginal.checked = settings.cancelOriginal !== false;
  fields.minCaptureSizeMb.value = settings.minCaptureSizeMb ?? 3;
  fields.skipCaptureDomains.value = (settings.skipCaptureDomains ?? []).join("\n");
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
