import { invoke } from "@tauri-apps/api/core";

import type { DownloadTask } from "@shared/types";

function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function listDownloadTasks(): Promise<DownloadTask[]> {
  if (!hasTauriRuntime()) {
    return Promise.resolve([]);
  }

  return invoke("list_download_tasks");
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
