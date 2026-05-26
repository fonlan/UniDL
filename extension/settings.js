const fields = {
  apiBaseUrl: document.querySelector("#api-base-url"),
  cancelOriginal: document.querySelector("#cancel-original"),
  connect: document.querySelector("#connect"),
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
  };
}

function applySettings(settings) {
  fields.apiBaseUrl.value = settings.apiBaseUrl ?? "";
  fields.cancelOriginal.checked = settings.cancelOriginal !== false;
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
