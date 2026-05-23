import { invoke } from "@tauri-apps/api/core";

import type {
  AppSettings,
  AppSettingsInput,
  CreateDownloadTaskInput,
  DownloadTask,
  EngineInstallResult,
  EngineKind,
  EngineSettings,
  EngineSettingsInput,
  SourceType,
} from "@shared/types";

export interface SystemOpenRequestPayload {
  sources: string[];
}

const defaultEngineDir = "%AppData%\\UniDL\\engines";

let previewEngineSettings: EngineSettings[] = [];

const previewAppSettings: AppSettings = {
  webAccessEnabled: false,
  webAccessPassword: "",
  webAccessUrl: "http://127.0.0.1:18080",
};

function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function listDownloadTasks(): Promise<DownloadTask[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("list_download_tasks");
}

export function takePendingOpenRequests(): Promise<string[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("take_pending_open_requests");
}

export function refreshDownloadTasks(): Promise<DownloadTask[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("refresh_download_tasks");
}

export function createDownloadTask(input: CreateDownloadTaskInput): Promise<DownloadTask> {
  if (!hasTauriRuntime()) {
    const settings = input.engineSettingsId
      ? previewEngineSettings.find((item) => item.id === input.engineSettingsId)
      : previewEngineSettings.find(
          (item) => item.engine === input.engine && item.enabled,
        ) ?? previewEngineSettings.find((item) => item.engine === input.engine);
    if (!settings) {
      return Promise.reject(
        new Error(
          input.engineSettingsId
            ? `Unknown engine settings: ${input.engineSettingsId}`
            : `Unknown engine: ${input.engine}`,
        ),
      );
    }
    if (settings.engine !== input.engine) {
      return Promise.reject(
        new Error(
          `engine settings ${settings.id} does not match ${input.engine}`,
        ),
      );
    }

    return Promise.resolve({
      id: crypto.randomUUID(),
      sourceType: input.sourceType,
      source: input.source,
      engineSettingsId: settings.id,
      engine: settings.engine,
      engineTaskId: null,
      fileName: input.fileName,
      status: "queued",
      progress: 0,
      speedBytesPerSec: 0,
      savePath: input.savePath,
      engineArgs: input.engineArgs,
      createdAt: new Date().toISOString(),
      completedAt: null,
      errorMessage: null,
    });
  }

  return invoke("create_download_task", { input });
}

export function listEngineSettings(): Promise<EngineSettings[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve(previewEngineSettings.map(cloneEngineSettings));
  }

  return invoke("list_engine_settings");
}

export function getAppSettings(): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    return Promise.resolve({ ...previewAppSettings });
  }

  return invoke("get_app_settings");
}

export function saveAppSettings(settings: AppSettingsInput): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    return Promise.resolve({
      ...previewAppSettings,
      ...settings,
    });
  }

  return invoke("save_app_settings", { settings });
}

export function saveEngineSettings(
  settings: EngineSettingsInput,
): Promise<EngineSettings> {
  if (!hasTauriRuntime()) {
    const nextInput = { ...settings, name: settings.name.trim() };
    if (!nextInput.name) {
      return Promise.reject(new Error("engine settings name is required"));
    }

    const preview = previewEngineSettings.find((item) => item.id === nextInput.id);
    if (!preview) {
      const next = cloneEngineSettings({
        ...nextInput,
        supportedSourceTypes: supportedSourceTypes(nextInput.engine),
        updatedAt: new Date().toISOString(),
      });
      previewEngineSettings = [...previewEngineSettings, next];
      return Promise.resolve(next);
    }

    const next = cloneEngineSettings({
      ...preview,
      ...nextInput,
      updatedAt: new Date().toISOString(),
    });
    previewEngineSettings = previewEngineSettings.map((item) =>
      item.id === next.id ? next : item,
    );
    return Promise.resolve(next);
  }

  return invoke("save_engine_settings", { settings });
}

export function deleteEngineSettings(settingsId: string): Promise<void> {
  if (!hasTauriRuntime()) {
    const settings = previewEngineSettings.find((item) => item.id === settingsId);
    if (!settings) {
      return Promise.reject(new Error(`Unknown engine settings: ${settingsId}`));
    }
    previewEngineSettings = previewEngineSettings.filter((item) => item.id !== settingsId);
    return Promise.resolve();
  }

  return invoke("delete_engine_settings", { settingsId });
}

export function installLatestEngine(settingsId: string): Promise<EngineInstallResult> {
  if (!hasTauriRuntime()) {
    const settings = previewEngineSettings.find((item) => item.id === settingsId);
    if (!settings) {
      return Promise.reject(new Error(`Unknown engine settings: ${settingsId}`));
    }
    if (settings.engine === "qbittorrent") {
      return Promise.reject(
        new Error("qBittorrent does not have a managed executable"),
      );
    }

    const executablePath =
      settings.engine === "aria2"
        ? `${defaultEngineDir}\\aria2c.exe`
        : `${defaultEngineDir}\\yt-dlp.exe`;
    const next = cloneEngineSettings({
      ...settings,
      executablePath,
      updatedAt: new Date().toISOString(),
    });
    previewEngineSettings = previewEngineSettings.map((item) =>
      item.id === next.id ? next : item,
    );
    return Promise.resolve({ settings: next, version: "preview" });
  }

  return invoke("install_latest_engine", { settingsId });
}

export function validateEngineSourceType(
  engine: EngineSettings["engine"],
  sourceType: SourceType,
): Promise<void> {
  if (!hasTauriRuntime()) {
    const preview = previewEngineSettings.find((item) => item.engine === engine);
    if (preview?.supportedSourceTypes.includes(sourceType)) {
      return Promise.resolve();
    }
    return Promise.reject(new Error(`${engine} does not support ${sourceType} tasks`));
  }

  return invoke("validate_engine_source_type", { engine, sourceType });
}

export function pauseDownloadTasks(ids: string[]): Promise<void> {
  return invoke("pause_download_tasks", { ids });
}

export function resumeDownloadTasks(ids: string[]): Promise<void> {
  return invoke("resume_download_tasks", { ids });
}

export function deleteDownloadTasks(ids: string[]): Promise<void> {
  return invoke("delete_download_tasks", { ids });
}

export function pauseAllUnfinishedDownloadTasks(): Promise<void> {
  return invoke("pause_all_unfinished_download_tasks");
}

export function resumeAllPausedDownloadTasks(): Promise<void> {
  return invoke("resume_all_paused_download_tasks");
}

function cloneEngineSettings(settings: EngineSettings): EngineSettings {
  return {
    ...settings,
    supportedSourceTypes: [...settings.supportedSourceTypes],
  };
}

function supportedSourceTypes(engine: EngineKind): SourceType[] {
  switch (engine) {
    case "aria2":
      return ["http", "ftp", "magnet", "torrent"];
    case "yt-dlp":
      return ["http", "ftp"];
    case "qbittorrent":
      return ["magnet", "torrent"];
  }
}
