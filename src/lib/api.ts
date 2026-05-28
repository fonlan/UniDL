import { invoke } from "@tauri-apps/api/core";

import type {
  AppSettings,
  AppSettingsInput,
  CreateDownloadTaskInput,
  DownloadFileConflict,
  DownloadTask,
  EngineInstallResult,
  EngineKind,
  EngineSettings,
  EngineSettingsInput,
  RemoteDirectoryEntry,
  SourceType,
  TorrentFileEntry,
} from "@shared/types";
import { hasTauriRuntime, webJson, webRequest } from "@/lib/runtime";
import { writeWebLog } from "@/lib/web-log";

export interface SystemOpenRequestPayload {
  requests: OpenTaskRequest[];
}

export interface OpenTaskRequest {
  source: string;
  fileName?: string | null;
  browserCookies?: string | null;
  httpReferrer?: string | null;
}

export type LogLevel = "info" | "warn" | "error";

function jsonRequest(method: string, body?: unknown): RequestInit {
  return {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  };
}

export function listDownloadTasks(): Promise<DownloadTask[]> {
  if (!hasTauriRuntime()) {
    return webJson("/api/tasks");
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
    return webJson("/api/tasks/refresh", jsonRequest("POST"));
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
    return webJson(
      "/api/torrent-files",
      jsonRequest("POST", {
        source,
        sourceType,
        engineSettingsId: engineSettingsId ?? null,
        savePath: savePath ?? null,
      }),
    );
  }

  return invoke("get_torrent_files", {
    source,
    sourceType,
    engineSettingsId: engineSettingsId ?? null,
    savePath: savePath ?? null,
  });
}

export function getTaskTorrentFiles(id: string): Promise<TorrentFileEntry[]> {
  if (!hasTauriRuntime()) {
    return webJson(`/api/tasks/${encodeURIComponent(id)}/torrent-files`);
  }

  return invoke("get_task_torrent_files", { id });
}

export function updateTaskFileSelection(
  id: string,
  selectedFileIndexes: number[],
): Promise<DownloadTask> {
  if (!hasTauriRuntime()) {
    return webJson(
      `/api/tasks/${encodeURIComponent(id)}/file-selection`,
      jsonRequest("POST", { selectedFileIndexes }),
    );
  }

  return invoke("update_task_file_selection", { id, selectedFileIndexes });
}

export function listRemoteDirectories(
  engineSettingsId: string,
  path: string,
): Promise<RemoteDirectoryEntry[]> {
  if (!hasTauriRuntime()) {
    return webJson(
      "/api/remote-directories",
      jsonRequest("POST", { engineSettingsId, path }),
    );
  }

  return invoke("list_remote_directories", { engineSettingsId, path });
}

export function resolveMagnetName(
  source: string,
  engineSettingsId: string,
  savePath: string,
): Promise<string> {
  if (!hasTauriRuntime()) {
    return webJson(
      "/api/magnet-name",
      jsonRequest("POST", { source, engineSettingsId, savePath }),
    );
  }

  return invoke("resolve_magnet_name", { source, engineSettingsId, savePath });
}

export function writeLog(level: LogLevel, message: string): Promise<void> {
  if (!hasTauriRuntime()) {
    return writeWebLog(level, message);
  }

  return invoke("write_log", { level, message });
}

export function createDownloadTask(
  input: CreateDownloadTaskInput,
): Promise<DownloadTask> {
  if (!hasTauriRuntime()) {
    return webJson("/api/tasks", jsonRequest("POST", input));
  }

  return invoke("create_download_task", { input });
}

export function checkDownloadFileConflict(
  input: CreateDownloadTaskInput,
): Promise<DownloadFileConflict | null> {
  if (!hasTauriRuntime()) {
    return webJson("/api/tasks/conflict", jsonRequest("POST", input));
  }

  return invoke("check_download_file_conflict", { input });
}

export function listEngineSettings(): Promise<EngineSettings[]> {
  if (!hasTauriRuntime()) {
    return webJson("/api/engine-settings");
  }

  return invoke("list_engine_settings");
}

export function getAppSettings(): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    return webJson("/api/app-settings");
  }

  return invoke("get_app_settings");
}

export function getSystemDownloadDir(): Promise<string> {
  if (!hasTauriRuntime()) {
    return webJson("/api/system-download-dir");
  }

  return invoke("get_system_download_dir");
}

export function getManagedEngineExecutablePath(
  engine: EngineKind,
): Promise<string | null> {
  if (!hasTauriRuntime()) {
    return webJson(
      "/api/managed-engine-executable-path",
      jsonRequest("POST", { engine }),
    );
  }

  return invoke("get_managed_engine_executable_path", { engine });
}

export function saveAppSettings(settings: AppSettingsInput): Promise<AppSettings> {
  if (!hasTauriRuntime()) {
    return webJson("/api/app-settings", jsonRequest("POST", settings));
  }

  return invoke("save_app_settings", { settings });
}

export function saveEngineSettings(
  settings: EngineSettingsInput,
): Promise<EngineSettings> {
  if (!hasTauriRuntime()) {
    return webJson("/api/engine-settings", jsonRequest("POST", settings));
  }

  return invoke("save_engine_settings", { settings });
}

export function deleteEngineSettings(settingsId: string): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest(`/api/engine-settings/${encodeURIComponent(settingsId)}`, {
      method: "DELETE",
    }).then(() => undefined);
  }

  return invoke("delete_engine_settings", { settingsId });
}

export function updateEngineTrackers(
  settingsId: string,
  subscriptionUrls: string,
): Promise<EngineSettings> {
  if (!hasTauriRuntime()) {
    return webJson(
      `/api/engine-settings/${encodeURIComponent(settingsId)}/trackers`,
      jsonRequest("POST", { subscriptionUrl: subscriptionUrls }),
    );
  }

  return invoke("update_engine_trackers", {
    settingsId,
    subscriptionUrl: subscriptionUrls,
  });
}

export function installLatestEngine(settingsId: string): Promise<EngineInstallResult> {
  if (!hasTauriRuntime()) {
    return webJson(
      `/api/engine-settings/${encodeURIComponent(settingsId)}/install-latest`,
      jsonRequest("POST"),
    );
  }

  return invoke("install_latest_engine", { settingsId });
}

export function testEngineConnection(settings: EngineSettingsInput): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest("/api/test-engine-connection", jsonRequest("POST", settings)).then(
      () => undefined,
    );
  }

  return invoke("test_engine_connection", { settings });
}

export function validateEngineSourceType(
  engine: EngineSettings["engine"],
  sourceType: SourceType,
): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest(
      "/api/validate-engine-source-type",
      jsonRequest("POST", { engine, sourceType }),
    ).then(() => undefined);
  }

  return invoke("validate_engine_source_type", { engine, sourceType });
}

export function pauseDownloadTasks(ids: string[]): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest("/api/tasks/pause", jsonRequest("POST", { ids })).then(
      () => undefined,
    );
  }

  return invoke("pause_download_tasks", { ids });
}

export function resumeDownloadTasks(ids: string[]): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest("/api/tasks/resume", jsonRequest("POST", { ids })).then(
      () => undefined,
    );
  }

  return invoke("resume_download_tasks", { ids });
}

export function openDownloadedFile(id: string): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest(
      `/api/tasks/${encodeURIComponent(id)}/open`,
      jsonRequest("POST"),
    ).then(() => undefined);
  }

  return invoke("open_downloaded_file", { id });
}

export function openDownloadDirectory(id: string): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest(
      `/api/tasks/${encodeURIComponent(id)}/open-directory`,
      jsonRequest("POST"),
    ).then(() => undefined);
  }

  return invoke("open_download_directory", { id });
}

export function deleteDownloadTasks(
  ids: string[],
  deleteCompletedFiles: boolean,
): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest(
      "/api/tasks/delete",
      jsonRequest("POST", { ids, deleteCompletedFiles }),
    ).then(() => undefined);
  }

  return invoke("delete_download_tasks", { ids, deleteCompletedFiles });
}

export function clearDownloadRecords(olderThanDays: number | null): Promise<number> {
  if (!hasTauriRuntime()) {
    return webJson("/api/tasks/clear-records", jsonRequest("POST", { olderThanDays }));
  }

  return invoke("clear_download_records", { olderThanDays });
}

export function pauseAllUnfinishedDownloadTasks(): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest("/api/tasks/pause-all", { method: "POST" }).then(() => undefined);
  }

  return invoke("pause_all_unfinished_download_tasks");
}

export function resumeAllPausedDownloadTasks(): Promise<void> {
  if (!hasTauriRuntime()) {
    return webRequest("/api/tasks/resume-all", { method: "POST" }).then(() => undefined);
  }

  return invoke("resume_all_paused_download_tasks");
}
