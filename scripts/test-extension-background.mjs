import assert from "node:assert/strict";
import fs from "node:fs";
import vm from "node:vm";

const source = fs.readFileSync("extension/background.js", "utf8");

const listeners = {
  addListener() {},
};

const context = {
  chrome: {
    action: {
      setBadgeBackgroundColor() {},
      setBadgeText() {},
    },
    contextMenus: {
      create() {},
      onClicked: listeners,
      removeAll(callback) {
        callback();
      },
    },
    cookies: {
      getAll() {},
    },
    downloads: {
      cancel() {},
      onCreated: listeners,
    },
    runtime: {
      id: "unidl-test",
      onInstalled: listeners,
      onMessage: listeners,
      onStartup: listeners,
    },
    storage: {
      local: {
        get() {},
        set() {},
      },
    },
    tabs: {
      get() {},
      onUpdated: listeners,
      query() {},
    },
  },
};

vm.createContext(context);
vm.runInContext(source, context, { filename: "extension/background.js" });

assert.equal(
  context.isActiveDownload({
    state: "in_progress",
    startTime: new Date().toISOString(),
  }),
  true,
  "active browser downloads should be captured",
);
assert.equal(
  context.isActiveDownload({ state: "complete", endTime: "2026-06-02T00:00:00Z" }),
  false,
  "completed browser download records should not be captured again",
);
assert.equal(
  context.isActiveDownload({ state: "interrupted" }),
  false,
  "interrupted browser download records should not be captured again",
);
assert.equal(
  context.isActiveDownload({
    state: "in_progress",
    startTime: new Date().toISOString(),
    endTime: "2026-06-02T00:00:00Z",
  }),
  false,
  "download records with an end time should not be captured again",
);
assert.equal(
  context.isActiveDownload({
    state: "in_progress",
    startTime: new Date(Date.now() - 60_000).toISOString(),
  }),
  false,
  "restored in-progress browser download records should not be captured again",
);
assert.equal(
  context.isActiveDownload({ state: "in_progress" }),
  false,
  "download records without a start time should not be captured",
);
