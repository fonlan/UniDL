import { useEffect, useMemo, useState } from "react";
import { X } from "lucide-react";

import { getTorrentFiles } from "@/lib/api";
import type {
  DownloadStatus,
  DownloadTask,
  EngineKind,
  SourceType,
  TorrentFileEntry,
} from "@shared/types";

const statusLabels: Record<DownloadStatus, string> = {
  queued: "等待中",
  running: "下载中",
  paused: "已暂停",
  completed: "已完成",
  failed: "失败",
  deleted: "已删除",
};

const sourceLabels: Record<SourceType, string> = {
  http: "HTTP",
  ftp: "FTP",
  magnet: "Magnet",
  torrent: "Torrent",
};

const engineLabels: Record<EngineKind, string> = {
  aria2: "aria2",
  "yt-dlp": "yt-dlp",
  qbittorrent: "qBittorrent",
};

function formatSpeed(bytesPerSecond: number) {
  if (bytesPerSecond <= 0) {
    return "-";
  }

  const units = ["B/s", "KB/s", "MB/s", "GB/s"];
  let value = bytesPerSecond;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

function formatBytes(bytes: number) {
  if (bytes <= 0) {
    return "0 B";
  }

  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unitIndex]}`;
}

function formatDate(value: string | null) {
  if (!value) {
    return "-";
  }

  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function parseMagnetTrackers(source: string) {
  if (!source.startsWith("magnet:?")) {
    return [];
  }

  return new URLSearchParams(source.slice("magnet:?".length)).getAll("tr");
}

function DetailField({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg border border-slate-200 bg-slate-50 px-3 py-2">
      <div className="text-xs text-slate-500">{label}</div>
      <div className="mt-1 truncate text-sm text-slate-800" title={value}>
        {value || "-"}
      </div>
    </div>
  );
}

export default function TaskDetailPanel({
  task,
  onClose,
}: {
  task: DownloadTask;
  onClose: () => void;
}) {
  const [torrentFiles, setTorrentFiles] = useState<TorrentFileEntry[]>([]);
  const [isLoadingTorrentFiles, setIsLoadingTorrentFiles] = useState(false);
  const [torrentFileError, setTorrentFileError] = useState<string | null>(null);
  const trackers = parseMagnetTrackers(task.source);
  const selectedFileIndexes = useMemo(
    () => new Set(task.selectedFileIndexes ?? []),
    [task.selectedFileIndexes],
  );

  useEffect(() => {
    let disposed = false;
    setTorrentFiles([]);
    setTorrentFileError(null);

    if (task.sourceType !== "torrent") {
      setIsLoadingTorrentFiles(false);
      return;
    }

    setIsLoadingTorrentFiles(true);
    void getTorrentFiles(task.source)
      .then((files) => {
        if (!disposed) {
          setTorrentFiles(files);
        }
      })
      .catch((nextError) => {
        if (!disposed) {
          setTorrentFileError(nextError instanceof Error ? nextError.message : String(nextError));
        }
      })
      .finally(() => {
        if (!disposed) {
          setIsLoadingTorrentFiles(false);
        }
      });

    return () => {
      disposed = true;
    };
  }, [task.source, task.sourceType]);

  return (
    <aside className="shrink-0 border-t border-slate-200 bg-white shadow-[0_-16px_40px_rgba(15,23,42,0.08)]">
      <div className="flex items-center justify-between border-b border-slate-100 px-4 py-3">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-slate-900">{task.fileName}</div>
          <div className="mt-1 text-xs text-slate-500">下载任务详情</div>
        </div>
        <button
          type="button"
          title="关闭详情"
          aria-label="关闭详情"
          onClick={onClose}
          className="grid h-8 w-8 place-items-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-700"
        >
          <X size={17} />
        </button>
      </div>

      <div className="max-h-[42vh] overflow-auto px-4 py-4">
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
          <DetailField label="任务 ID" value={task.id} />
          <DetailField label="引擎任务 ID" value={task.engineTaskId ?? "-"} />
          <DetailField label="来源类型" value={sourceLabels[task.sourceType]} />
          <DetailField label="下载引擎" value={engineLabels[task.engine]} />
          <DetailField label="状态" value={statusLabels[task.status]} />
          <DetailField label="进度" value={`${task.progress.toFixed(1)}%`} />
          <DetailField label="速度" value={formatSpeed(task.speedBytesPerSec)} />
          <DetailField label="保存路径" value={task.savePath} />
          <DetailField label="创建时间" value={formatDate(task.createdAt)} />
          <DetailField label="完成时间" value={formatDate(task.completedAt)} />
          <DetailField label="引擎参数" value={task.engineArgs || "-"} />
          <DetailField label="错误信息" value={task.errorMessage ?? "-"} />
        </div>

        <div className="mt-4 rounded-lg border border-slate-200">
          <div className="border-b border-slate-100 px-3 py-2 text-xs font-semibold uppercase tracking-normal text-slate-500">
            原始来源
          </div>
          <div className="break-all px-3 py-2 text-sm text-slate-700">{task.source}</div>
        </div>

        {(task.sourceType === "magnet" || task.sourceType === "torrent") && (
          <div className="mt-4 grid gap-4 lg:grid-cols-2">
            <section className="rounded-lg border border-slate-200">
              <div className="border-b border-slate-100 px-3 py-2 text-xs font-semibold uppercase tracking-normal text-slate-500">
                Tracker
              </div>
              {task.sourceType === "magnet" ? (
                trackers.length > 0 ? (
                  <ul className="max-h-40 overflow-auto px-3 py-2 text-sm text-slate-700">
                    {trackers.map((tracker) => (
                      <li key={tracker} className="break-all border-b border-slate-100 py-1 last:border-b-0">
                        {tracker}
                      </li>
                    ))}
                  </ul>
                ) : (
                  <div className="px-3 py-2 text-sm text-slate-500">磁链未包含 tr 参数。</div>
                )
              ) : (
                <div className="px-3 py-2 text-sm text-slate-500">
                  当前 .torrent 解析接口只提供文件列表，未暴露 tracker 信息。
                </div>
              )}
            </section>

            <section className="rounded-lg border border-slate-200">
              <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2">
                <div className="text-xs font-semibold uppercase tracking-normal text-slate-500">文件列表</div>
                {task.sourceType === "torrent" && torrentFiles.length > 0 && (
                  <div className="text-xs text-slate-500">{torrentFiles.length} 个文件</div>
                )}
              </div>
              {task.sourceType === "magnet" ? (
                <div className="px-3 py-2 text-sm text-slate-500">
                  磁链任务当前无法在开始前解析文件列表，需要下载引擎暴露元数据后显示。
                </div>
              ) : isLoadingTorrentFiles ? (
                <div className="px-3 py-2 text-sm text-slate-500">正在读取文件列表...</div>
              ) : torrentFileError ? (
                <div className="px-3 py-2 text-sm text-rose-700">{torrentFileError}</div>
              ) : torrentFiles.length === 0 ? (
                <div className="px-3 py-2 text-sm text-slate-500">未读取到文件列表。</div>
              ) : (
                <div className="max-h-56 overflow-auto">
                  <table className="w-full table-fixed text-left text-sm">
                    <thead className="sticky top-0 bg-white text-xs text-slate-500">
                      <tr>
                        <th className="w-16 border-b border-slate-100 px-3 py-2">序号</th>
                        <th className="border-b border-slate-100 px-3 py-2">路径</th>
                        <th className="w-24 border-b border-slate-100 px-3 py-2 text-right">大小</th>
                        <th className="w-20 border-b border-slate-100 px-3 py-2 text-right">选择</th>
                      </tr>
                    </thead>
                    <tbody>
                      {torrentFiles.map((file) => (
                        <tr key={file.index}>
                          <td className="border-b border-slate-100 px-3 py-2 tabular-nums text-slate-500">
                            {file.index}
                          </td>
                          <td className="border-b border-slate-100 px-3 py-2">
                            <div className="truncate text-slate-700" title={file.path}>{file.path}</div>
                          </td>
                          <td className="border-b border-slate-100 px-3 py-2 text-right tabular-nums text-slate-600">
                            {formatBytes(file.length)}
                          </td>
                          <td className="border-b border-slate-100 px-3 py-2 text-right text-slate-600">
                            {selectedFileIndexes.size === 0 || selectedFileIndexes.has(file.index) ? "是" : "否"}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </div>
        )}
      </div>
    </aside>
  );
}