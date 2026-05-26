export type EngineKind = "aria2" | "yt-dlp" | "qbittorrent";

export type SourceType = "http" | "ftp" | "magnet" | "torrent";

export type DownloadStatus =
  | "queued"
  | "running"
  | "paused"
  | "completed"
  | "failed"
  | "deleted";

export interface DownloadTask {
  id: string;
  sourceType: SourceType;
  source: string;
  engineSettingsId: string;
  engine: EngineKind;
  engineTaskId: string | null;
  fileName: string;
  status: DownloadStatus;
  progress: number;
  speedBytesPerSec: number;
  downloadedBytes: number;
  totalBytes: number;
  savePath: string;
  engineArgs: string;
  selectedFileIndexes?: number[] | null;
  browserCookies?: string | null;
  createdAt: string;
  completedAt: string | null;
  errorMessage: string | null;
}

export interface CreateDownloadTaskInput {
  sourceType: SourceType;
  source: string;
  engine: EngineKind;
  engineSettingsId?: string | null;
  fileName: string;
  savePath: string;
  engineArgs: string;
  selectedFileIndexes?: number[] | null;
  browserCookies?: string | null;
}

export interface TorrentFileEntry {
  index: number;
  path: string;
  length: number;
  completedLength: number;
}

export interface RemoteDirectoryEntry {
  name: string;
  path: string;
}

export interface EngineSettings {
  id: string;
  engine: EngineKind;
  name: string;
  enabled: boolean;
  executablePath: string | null;
  defaultDownloadDir: string;
  defaultArgs: string;
  connectionUrl: string | null;
  username: string | null;
  password: string | null;
  remotePath: string | null;
  supportedSourceTypes: SourceType[];
  preferredDomains: string[];
  trackerSubscriptionUrl: string | null;
  trackers: string[];
  proxyUrl: string | null;
  priority: number;
  updatedAt: string;
}

export type EngineSettingsInput = Omit<EngineSettings, "updatedAt">;

export interface EngineInstallResult {
  settings: EngineSettings;
  version: string;
}

export interface AppSettings {
  webAccessEnabled: boolean;
  webAccessPassword: string;
  webAccessUrl: string;
  privateDownloadDomains: string[];
  appProxyUrl: string;
}

export type AppSettingsInput = AppSettings;
