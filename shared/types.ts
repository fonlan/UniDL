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
  engine: EngineKind;
  engineTaskId: string | null;
  fileName: string;
  status: DownloadStatus;
  progress: number;
  speedBytesPerSec: number;
  savePath: string;
  engineArgs: string;
  createdAt: string;
  completedAt: string | null;
  errorMessage: string | null;
}

export interface CreateDownloadTaskInput {
  sourceType: SourceType;
  source: string;
  engine: EngineKind;
  fileName: string;
  savePath: string;
  engineArgs: string;
}

export interface EngineSettings {
  engine: EngineKind;
  enabled: boolean;
  executablePath: string | null;
  defaultDownloadDir: string;
  defaultArgs: string;
  connectionUrl: string | null;
  username: string | null;
  password: string | null;
  remotePath: string | null;
  supportedSourceTypes: SourceType[];
  updatedAt: string;
}

export type EngineSettingsInput = Omit<
  EngineSettings,
  "supportedSourceTypes" | "updatedAt"
>;
