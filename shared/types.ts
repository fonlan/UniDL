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
  createdAt: string;
  completedAt: string | null;
  errorMessage: string | null;
}
