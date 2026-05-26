import { useEffect, useMemo, useState } from "react";
import { X } from "lucide-react";

import { getTaskTorrentFiles, updateTaskFileSelection } from "@/lib/api";
import { reportError } from "@/lib/error";
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

const fileCategoryFilters = [
  { label: "视频", extensions: ["3g2", "3gp", "avi", "flv", "m2ts", "m4v", "mkv", "mov", "mp4", "mpeg", "mpg", "rmvb", "ts", "webm", "wmv"] },
  { label: "图片", extensions: ["avif", "bmp", "gif", "heic", "jpeg", "jpg", "png", "svg", "tif", "tiff", "webp"] },
  { label: "音频", extensions: ["aac", "ape", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma"] },
  { label: "字幕", extensions: ["ass", "srt", "ssa", "sub", "sup", "vtt"] },
  { label: "文档", extensions: ["azw3", "doc", "docx", "epub", "md", "mobi", "pdf", "ppt", "pptx", "txt", "xls", "xlsx"] },
  { label: "压缩包", extensions: ["7z", "bz2", "gz", "iso", "rar", "tar", "tgz", "xz", "zip"] },
].map((filter) => ({
  ...filter,
  extensions: new Set(filter.extensions),
}));

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

function fileProgress(file: TorrentFileEntry) {
  return file.length > 0 ? (file.completedLength / file.length) * 100 : 0;
}

function fileExtension(path: string) {
  const fileName = path.split(/[\\/]/).pop() ?? path;
  const dotIndex = fileName.lastIndexOf(".");

  if (dotIndex <= 0 || dotIndex === fileName.length - 1) {
    return "";
  }

  return fileName.slice(dotIndex + 1).toLowerCase();
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
  const [activeTab, setActiveTab] = useState<"detail" | "files">("detail");
  const [torrentFiles, setTorrentFiles] = useState<TorrentFileEntry[]>([]);
  const [isLoadingTorrentFiles, setIsLoadingTorrentFiles] = useState(false);
  const [torrentFileError, setTorrentFileError] = useState<string | null>(null);
  const [selectedFileIndexes, setSelectedFileIndexes] = useState<Set<number>>(
    () => new Set(task.selectedFileIndexes ?? []),
  );
  const [isSavingFileSelection, setIsSavingFileSelection] = useState(false);
  const trackers = parseMagnetTrackers(task.source);
  const selectedTorrentFileCount = useMemo(
    () => torrentFiles.filter((file) => selectedFileIndexes.has(file.index)).length,
    [selectedFileIndexes, torrentFiles],
  );
  const allTorrentFilesSelected = torrentFiles.length > 0 && torrentFiles.every((file) => selectedFileIndexes.has(file.index));
  const categoryFilterButtons = useMemo(
    () =>
      fileCategoryFilters.map((filter) => ({
        ...filter,
        count: torrentFiles.filter((file) => filter.extensions.has(fileExtension(file.path))).length,
      })),
    [torrentFiles],
  );

  useEffect(() => {
    if (task.selectedFileIndexes && task.selectedFileIndexes.length > 0) {
      setSelectedFileIndexes(new Set(task.selectedFileIndexes));
    }
  }, [task.id, task.selectedFileIndexes]);

  useEffect(() => {
    let disposed = false;
    setTorrentFiles([]);
    setTorrentFileError(null);

    if (task.sourceType !== "magnet" && task.sourceType !== "torrent") {
      setIsLoadingTorrentFiles(false);
      return;
    }

    setIsLoadingTorrentFiles(true);
    void getTaskTorrentFiles(task.id)
      .then((files) => {
        if (!disposed) {
          setTorrentFiles(files);
          if (!task.selectedFileIndexes || task.selectedFileIndexes.length === 0) {
            setSelectedFileIndexes(new Set(files.map((file) => file.index)));
          }
        }
      })
      .catch((nextError) => {
        if (!disposed) {
          setTorrentFileError(reportError("load task torrent files", nextError));
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
  }, [task.id, task.sourceType]);

  async function saveFileSelection(nextSelectedFileIndexes: Set<number>) {
    if (nextSelectedFileIndexes.size === 0) {
      return;
    }
    setSelectedFileIndexes(nextSelectedFileIndexes);
    setIsSavingFileSelection(true);
    setTorrentFileError(null);
    try {
      await updateTaskFileSelection(
        task.id,
        [...nextSelectedFileIndexes].sort((left, right) => left - right),
      );
    } catch (nextError) {
      setTorrentFileError(reportError("update task file selection", nextError));
    } finally {
      setIsSavingFileSelection(false);
    }
  }

  function saveFiles(files: TorrentFileEntry[]) {
    void saveFileSelection(new Set(files.map((file) => file.index)));
  }

  function invertFileSelection() {
    saveFiles(torrentFiles.filter((file) => !selectedFileIndexes.has(file.index)));
  }

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

      {(task.sourceType === "magnet" || task.sourceType === "torrent") && (
        <div className="flex gap-2 border-b border-slate-100 px-4 pt-3">
          {[
            ["detail", "详情"],
            ["files", "文件列表"],
          ].map(([tab, label]) => (
            <button
              key={tab}
              type="button"
              onClick={() => setActiveTab(tab as "detail" | "files")}
              className={`border-b-2 px-3 pb-2 text-sm font-medium ${
                activeTab === tab
                  ? "border-emerald-700 text-emerald-700"
                  : "border-transparent text-slate-500 hover:text-slate-700"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      )}

      <div className="max-h-[42vh] overflow-auto px-4 py-4">
        {activeTab === "detail" && (
          <>
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
          <DetailField label="任务 ID" value={task.id} />
          <DetailField label="引擎任务 ID" value={task.engineTaskId ?? "-"} />
          <DetailField label="来源类型" value={sourceLabels[task.sourceType]} />
          <DetailField label="下载引擎" value={engineLabels[task.engine]} />
          <DetailField label="状态" value={statusLabels[task.status]} />
          <DetailField label="进度" value={`${task.progress.toFixed(1)}%`} />
          <DetailField label="已下载大小" value={formatBytes(task.downloadedBytes)} />
          <DetailField label="总大小" value={formatBytes(task.totalBytes)} />
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
          <div className="mt-4">
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
          </div>
        )}
          </>
        )}

        {activeTab === "files" && (task.sourceType === "magnet" || task.sourceType === "torrent") && (
          <section className="rounded-lg border border-slate-200">
            <div className="flex items-center justify-between border-b border-slate-100 px-3 py-2">
              <div className="text-xs font-semibold uppercase tracking-normal text-slate-500">文件列表</div>
              {torrentFiles.length > 0 && (
                <div className="text-xs text-slate-500">
                  {selectedTorrentFileCount}/{torrentFiles.length} 个文件
                  {isSavingFileSelection ? "，正在保存…" : ""}
                </div>
              )}
            </div>
            {isLoadingTorrentFiles ? (
              <div className="px-3 py-2 text-sm text-slate-500">正在读取文件列表...</div>
            ) : torrentFileError ? (
              <div className="px-3 py-2 text-sm text-rose-700">{torrentFileError}</div>
            ) : torrentFiles.length === 0 ? (
              <div className="px-3 py-2 text-sm text-slate-500">未读取到文件列表，请等待引擎获取 BT 元数据后刷新。</div>
            ) : (
              <>
                <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 px-3 py-2">
                  <button
                    type="button"
                    disabled={isSavingFileSelection || allTorrentFilesSelected}
                    onClick={() => saveFiles(torrentFiles)}
                    className="rounded-md border border-slate-200 px-2.5 py-1 text-xs text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    全选
                  </button>
                  <button
                    type="button"
                    disabled={isSavingFileSelection || selectedTorrentFileCount === torrentFiles.length}
                    onClick={invertFileSelection}
                    className="rounded-md border border-slate-200 px-2.5 py-1 text-xs text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    反选
                  </button>
                  {categoryFilterButtons.map((filter) => (
                    <button
                      key={filter.label}
                      type="button"
                      disabled={isSavingFileSelection || filter.count === 0}
                      onClick={() => saveFiles(torrentFiles.filter((file) => filter.extensions.has(fileExtension(file.path))))}
                      className="rounded-md border border-slate-200 px-2.5 py-1 text-xs text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:opacity-50"
                    >
                      {filter.label}({filter.count})
                    </button>
                  ))}
                </div>
                <div className="max-h-[30vh] overflow-auto">
                  <table className="w-full table-fixed text-left text-sm">
                  <thead className="sticky top-0 bg-white text-xs text-slate-500">
                    <tr>
                      <th className="w-12 border-b border-slate-100 px-3 py-2">选择</th>
                      <th className="border-b border-slate-100 px-3 py-2">相对路径</th>
                      <th className="w-24 border-b border-slate-100 px-3 py-2 text-right">大小</th>
                      <th className="w-24 border-b border-slate-100 px-3 py-2 text-right">进度</th>
                    </tr>
                  </thead>
                  <tbody>
                    {torrentFiles.map((file) => (
                      <tr key={file.index}>
                        <td className="border-b border-slate-100 px-3 py-2">
                          <input
                            type="checkbox"
                            checked={selectedFileIndexes.has(file.index)}
                            disabled={isSavingFileSelection || (selectedFileIndexes.size === 1 && selectedFileIndexes.has(file.index))}
                            onChange={(event) => {
                              const next = new Set(selectedFileIndexes);
                              if (event.currentTarget.checked) {
                                next.add(file.index);
                              } else {
                                next.delete(file.index);
                              }
                              void saveFileSelection(next);
                            }}
                          />
                        </td>
                        <td className="border-b border-slate-100 px-3 py-2">
                          <div className="truncate text-slate-700" title={file.path}>{file.path}</div>
                        </td>
                        <td className="border-b border-slate-100 px-3 py-2 text-right tabular-nums text-slate-600">
                          {formatBytes(file.length)}
                        </td>
                        <td className="border-b border-slate-100 px-3 py-2 text-right tabular-nums text-slate-600">
                          {fileProgress(file).toFixed(1)}%
                        </td>
                      </tr>
                    ))}
                  </tbody>
                  </table>
                </div>
              </>
            )}
          </section>
        )}
      </div>
    </aside>
  );
}
