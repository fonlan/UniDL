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
  TorrentFileEntry,
} from "@shared/types";

export interface SystemOpenRequestPayload {
  requests: OpenTaskRequest[];
}

export interface OpenTaskRequest {
  source: string;
  fileName?: string | null;
  browserCookies?: string | null;
}

export type LogLevel = "info" | "warn" | "error";

let previewEngineSettings: EngineSettings[] = [];

let previewAppSettings: AppSettings = {
  webAccessEnabled: false,
  webAccessPassword: "",
  webAccessUrl: "http://127.0.0.1:18080",
  privateDownloadDomains: [],
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

export function takePendingOpenRequests(): Promise<OpenTaskRequest[]> {
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


export function getTorrentFiles(
  source: string,
  sourceType: SourceType,
  engineSettingsId?: string | null,
  savePath?: string | null,
): Promise<TorrentFileEntry[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("get_torrent_files", {
    source,
    sourceType,
    engineSettingsId: engineSettingsId ?? null,
    savePath: savePath ?? null,
  });
}

export function resolveMagnetName(
  source: string,
  engineSettingsId: string,
  savePath: string,
): Promise<string> {
  if (!hasTauriRuntime()) {
    return Promise.reject(new Error("magnet metadata requires Tauri runtime"));
  }

  return invoke("resolve_magnet_name", { source, engineSettingsId, savePath });
}

export function writeLog(level: LogLevel, message: string): Promise<void> {
  if (!hasTauriRuntime()) {
    return Promise.resolve();
  }

  return invoke("write_log", { level, message });
}

export function createDownloadTask(input: CreateDownloadTaskInput): Promise<DownloadTask> {
  if (!hasTauriRuntime()) {
    const engineSettings = sortEngineSettings(previewEngineSettings);
    const settings = input.engineSettingsId
      ? engineSettings.find((item) => item.id === input.engineSettingsId)
      : engineSettings.find(
          (item) =>
            item.engine === input.engine &&
            item.enabled &&
            item.supportedSourceTypes.includes(input.sourceType),
        );
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
    if (!settings.supportedSourceTypes.includes(input.sourceType)) {
      return Promise.reject(
        new Error(`${settings.id} does not support ${input.sourceType} tasks`),
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
      downloadedBytes: 0,
      totalBytes: 0,
      savePath: input.savePath,
      engineArgs: input.engineArgs,
      selectedFileIndexes: input.selectedFileIndexes ?? null,
      createdAt: new Date().toISOString(),
      completedAt: null,
      errorMessage: null,
    });
  }

  return invoke("create_download_task", { input });
}

export function listEngineSettings(): Promise<EngineSettings[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve(sortEngineSettings(previewEngineSettings).map(cloneEngineSettings));
  }

  return invoke("list_engine_settings");
}

export function getAppSettings(): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    return Promise.resolve({ ...previewAppSettings });
  }

  return invoke("get_app_settings");
}

export function getSystemDownloadDir(): Promise<string> {
  if (!hasTauriRuntime()) {
    return Promise.reject(
      new Error("system download directory is unavailable outside Tauri runtime"),
    );
  }

  return invoke("get_system_download_dir");
}

export function getManagedEngineExecutablePath(
  engine: EngineKind,
): Promise<string | null> {
  if (!hasTauriRuntime()) {
    return Promise.resolve(null);
  }

  return invoke("get_managed_engine_executable_path", { engine });
}

export function saveAppSettings(settings: AppSettingsInput): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    previewAppSettings = {
      ...previewAppSettings,
      ...settings,
    };
    return Promise.resolve({ ...previewAppSettings });
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
        supportedSourceTypes: normalizeSourceTypes(
          nextInput.engine,
          nextInput.supportedSourceTypes,
        ),
        updatedAt: new Date().toISOString(),
      });
      previewEngineSettings = [...previewEngineSettings, next];
      return Promise.resolve(next);
    }

    const next = cloneEngineSettings({
      ...preview,
      ...nextInput,
      supportedSourceTypes: normalizeSourceTypes(
        nextInput.engine,
        nextInput.supportedSourceTypes,
      ),
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

export function updateEngineTrackers(
  settingsId: string,
  subscriptionUrl: string,
): Promise<EngineSettings> {
  if (!hasTauriRuntime()) {
    return Promise.reject(new Error("tracker update requires Tauri runtime"));
  }

  return invoke("update_engine_trackers", { settingsId, subscriptionUrl });
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

    const next = cloneEngineSettings({
      ...settings,
      updatedAt: new Date().toISOString(),
    });
    previewEngineSettings = previewEngineSettings.map((item) =>
      item.id === next.id ? next : item,
    );
    return Promise.resolve({ settings: next, version: "preview" });
  }

  return invoke("install_latest_engine", { settingsId });
}

export function testEngineConnection(settings: EngineSettingsInput): Promise<void> {
  if (!hasTauriRuntime()) {
    if (settings.engine === "yt-dlp") {
      return Promise.reject(new Error("yt-dlp does not use a remote connection"));
    }
    return Promise.resolve();
  }

  return invoke("test_engine_connection", { settings });
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

export function openDownloadedFile(id: string): Promise<void> {
  if (hasTauriRuntime() === false) {
    return Promise.reject(new Error("opening downloaded files requires Tauri runtime"));
  }

  return invoke("open_downloaded_file", { id });
}

export function deleteDownloadTasks(
  ids: string[],
  deleteCompletedFiles: boolean,
): Promise<void> {
  return invoke("delete_download_tasks", { ids, deleteCompletedFiles });
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
    preferredDomains: [...settings.preferredDomains],
  };
}

function normalizeSourceTypes(engine: EngineKind, selected: SourceType[]) {
  const supported = supportedSourceTypes(engine);
  const invalid = selected.find((sourceType) => !supported.includes(sourceType));
  if (invalid) {
    throw new Error(`${engine} does not support ${invalid} tasks`);
  }

  return supported.filter((sourceType) => selected.includes(sourceType));
}

function sortEngineSettings(settings: EngineSettings[]) {
  return [...settings].sort((left, right) => {
    if (left.priority !== right.priority) {
      return left.priority - right.priority;
    }
    return left.id.localeCompare(right.id);
  });
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
