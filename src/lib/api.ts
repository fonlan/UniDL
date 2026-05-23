import { invoke } from "@tauri-apps/api/core";

import type {
  CreateDownloadTaskInput,
  DownloadTask,
  EngineSettings,
  EngineSettingsInput,
  SourceType,
} from "@shared/types";

const previewEngineSettings: EngineSettings[] = [
  {
    engine: "aria2",
    enabled: false,
    executablePath: null,
    defaultDownloadDir: "",
    defaultArgs: "--continue=true",
    connectionUrl: null,
    username: null,
    password: null,
    remotePath: null,
    supportedSourceTypes: ["http", "ftp", "magnet", "torrent"],
    updatedAt: "",
  },
  {
    engine: "yt-dlp",
    enabled: false,
    executablePath: null,
    defaultDownloadDir: "",
    defaultArgs: "--newline",
    connectionUrl: null,
    username: null,
    password: null,
    remotePath: null,
    supportedSourceTypes: ["http", "ftp"],
    updatedAt: "",
  },
  {
    engine: "qbittorrent",
    enabled: false,
    executablePath: null,
    defaultDownloadDir: "",
    defaultArgs: "",
    connectionUrl: "http://127.0.0.1:8080",
    username: null,
    password: null,
    remotePath: "",
    supportedSourceTypes: ["magnet", "torrent"],
    updatedAt: "",
  },
];

function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function listDownloadTasks(): Promise<DownloadTask[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("list_download_tasks");
}

export function createDownloadTask(input: CreateDownloadTaskInput): Promise<DownloadTask> {
  if (!hasTauriRuntime()) {
    return Promise.resolve({
      id: crypto.randomUUID(),
      sourceType: input.sourceType,
      source: input.source,
      engine: input.engine,
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

export function saveEngineSettings(
  settings: EngineSettingsInput,
): Promise<EngineSettings> {
  if (!hasTauriRuntime()) {
    const preview = previewEngineSettings.find((item) => item.engine === settings.engine);
    if (!preview) {
      return Promise.reject(new Error(`Unknown engine: ${settings.engine}`));
    }

    return Promise.resolve(cloneEngineSettings({ ...preview, ...settings }));
  }

  return invoke("save_engine_settings", { settings });
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
