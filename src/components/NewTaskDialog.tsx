import { useEffect, useMemo, useState } from "react";
import type { ClipboardEvent, DragEvent } from "react";
import { FilePlus, FolderOpen, X } from "lucide-react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { createDownloadTask, getTorrentFiles, listEngineSettings, writeLog } from "@/lib/api";
import type {
  DownloadTask,
  EngineKind,
  EngineSettings,
  SourceType,
    TorrentFileEntry,
} from "@shared/types";

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

function engineOptionLabel(settings: EngineSettings) {
  const name = settings.name.trim();
  const engineLabel = engineLabels[settings.engine];
  return name && name !== engineLabel ? `${name} / ${engineLabel}` : engineLabel;
}

interface ParsedSource {
  sourceType: SourceType;
  source: string;
  fileName: string;
}


interface TorrentSelectionState {
  files: TorrentFileEntry[];
  selectedIndexes: Set<number>;
}


function classNames(...names: Array<string | false | null | undefined>) {
  return names.filter(Boolean).join(" ");
}

function parseSource(value: string): ParsedSource | null {
  const source = value.trim();
  if (!source) {
    return null;
  }

  if (/^magnet:/i.test(source)) {
    return {
      sourceType: "magnet",
      source,
      fileName: parseMagnetName(source) ?? "magnet",
    };
  }

  if (/^https?:\/\//i.test(source)) {
    return {
      sourceType: "http",
      source,
      fileName: parseUrlName(source) ?? "http-download",
    };
  }

  if (/^ftp:\/\//i.test(source)) {
    return {
      sourceType: "ftp",
      source,
      fileName: parseUrlName(source) ?? "ftp-download",
    };
  }

  if (/\.torrent(?:$|[?#])/i.test(source)) {
    return {
      sourceType: "torrent",
      source,
      fileName: parsePathName(source) ?? "download.torrent",
    };
  }

  return null;
}

function parseMagnetName(value: string) {
  const match = /(?:[?&])dn=([^&]+)/i.exec(value);
  if (!match) {
    return null;
  }

  try {
    return decodeURIComponent(match[1].replace(/\+/g, " "));
  } catch {
    return match[1];
  }
}

function parseUrlName(value: string) {
  try {
    const url = new URL(value);
    const last = url.pathname.split("/").filter(Boolean).at(-1);
    return last ? decodeURIComponent(last) : null;
  } catch {
    return parsePathName(value);
  }
}

function parsePathName(value: string) {
  const clean = value.split(/[?#]/)[0];
  const parts = clean.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? null;
}

function valueFromDroppedFile(file: File) {
  const fileWithPath = file as File & { path?: string };
  return fileWithPath.path ?? file.name;
}

function defaultSavePath(settings: EngineSettings) {
  if (settings.engine === "qbittorrent") {
    return settings.remotePath || settings.defaultDownloadDir;
  }

  return settings.defaultDownloadDir;
}

export default function NewTaskDialog({
  open,
  initialSource = null,
  initialFileName = null,
  initialBrowserCookies = null,
  onClose,
  onCreated,
}: {
  open: boolean;
  initialSource?: string | null;
  initialFileName?: string | null;
  initialBrowserCookies?: string | null;
  onClose: () => void;
  onCreated: (task: DownloadTask) => void;
}) {
  const [sourceInput, setSourceInput] = useState("");
  const [fileName, setFileName] = useState("");
  const [engineSettings, setEngineSettings] = useState<EngineSettings[]>([]);
  const [selectedEngineSettingsId, setSelectedEngineSettingsId] = useState("");
  const [savePath, setSavePath] = useState("");
  const [engineArgs, setEngineArgs] = useState("");
  const [torrentSelection, setTorrentSelection] = useState<TorrentSelectionState | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const parsedSource = useMemo(() => parseSource(sourceInput), [sourceInput]);
  const compatibleSettings = useMemo(() => {
    if (!parsedSource) {
      return [];
    }
    return engineSettings.filter((settings) =>
      settings.supportedSourceTypes.includes(parsedSource.sourceType),
    );
  }, [engineSettings, parsedSource]);
  const visibleEngineSettings = parsedSource ? compatibleSettings : engineSettings;
  const selectedSettings =
    selectedEngineSettingsId === ""
      ? null
      : engineSettings.find((settings) => settings.id === selectedEngineSettingsId) ?? null;
  const canCreate =
    Boolean(parsedSource) &&
    Boolean(selectedSettings?.enabled) &&
    fileName.trim().length > 0 &&
    savePath.trim().length > 0 &&
    !isCreating;
  const canSelectLocalSavePath = selectedSettings?.engine !== "qbittorrent";

  useEffect(() => {
    if (!open) {
      return;
    }

    setSourceInput(initialSource ?? "");
  }, [initialSource, open]);

  useEffect(() => {
    if (!open) {
      return;
    }

    async function loadSettings() {
      setIsLoading(true);
      setError(null);

      try {
        const settings = await listEngineSettings();
        setEngineSettings(settings);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        setIsLoading(false);
      }
    }

    void loadSettings();
  }, [open]);

  useEffect(() => {
    setFileName(initialFileName ?? parsedSource?.fileName ?? "");
  }, [initialFileName, parsedSource]);

  useEffect(() => {
    if (!parsedSource) {
      return;
    }

    const current = compatibleSettings.find(
      (settings) => settings.id === selectedEngineSettingsId && settings.enabled,
    );
    const next = current ?? compatibleSettings.find((settings) => settings.enabled);
    setSelectedEngineSettingsId(next?.id ?? "");
  }, [compatibleSettings, parsedSource, selectedEngineSettingsId]);

  useEffect(() => {
    if (!selectedSettings) {
      setSavePath("");
      setEngineArgs("");
      setTorrentSelection(null);
      return;
    }

    setSavePath(defaultSavePath(selectedSettings));
    setEngineArgs(selectedSettings.defaultArgs);
    setTorrentSelection(null);
  }, [selectedSettings]);

  function resetAndClose() {
    setSourceInput("");
    setFileName("");
    setSelectedEngineSettingsId("");
    setSavePath("");
    setEngineArgs("");
    setError(null);
    setTorrentSelection(null);
    onClose();
  }

  function handleDrop(event: DragEvent<HTMLTextAreaElement>) {
    event.preventDefault();

    const file = event.dataTransfer.files.item(0);
    if (file) {
      setSourceInput(valueFromDroppedFile(file));
      return;
    }

    const text = event.dataTransfer.getData("text/plain");
    if (text.trim()) {
      setSourceInput(text);
    }
  }

  function handlePaste(event: ClipboardEvent<HTMLTextAreaElement>) {
    const text = event.clipboardData.getData("text/plain");
    if (parseSource(text)) {
      event.preventDefault();
      setSourceInput(text);
    }
  }

  async function submitTask() {
    if (!parsedSource || !selectedSettings) {
      return;
    }

    setIsCreating(true);
    setError(null);

    try {
      void writeLog(
        "info",
        `submitting new task: engine=${selectedSettings.engine}, sourceType=${parsedSource.sourceType}`,
      );
      const task = await createDownloadTask({
        sourceType: parsedSource.sourceType,
        source: parsedSource.source,
        engine: selectedSettings.engine,
        engineSettingsId: selectedSettings.id,
        fileName: fileName.trim(),
        savePath: savePath.trim(),
        engineArgs,
        selectedFileIndexes: torrentSelection?.selectedIndexes.size
          ? [...torrentSelection.selectedIndexes].sort((left, right) => left - right)
          : null,
        browserCookies: initialBrowserCookies,
      });
      onCreated(task);
      void writeLog("info", `new task created: id=${task.id}`);
      resetAndClose();
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setIsCreating(false);
    }
  }

  useEffect(() => {
    if (!open || !parsedSource || parsedSource.sourceType !== "torrent") {
      setTorrentSelection(null);
      return;
    }

    const source = parsedSource.source;

    async function loadTorrentFiles() {
      try {
        const files = await getTorrentFiles(source);
        setTorrentSelection({
          files,
          selectedIndexes: new Set(files.map((file) => file.index)),
        });
      } catch (nextError) {
        setTorrentSelection(null);
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      }
    }

    void loadTorrentFiles();
  }, [open, parsedSource]);

  async function selectSavePath() {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: savePath.trim() || undefined,
      title: "选择下载目录",
    });

    if (typeof selected === "string") {
      setSavePath(selected);
    }
  }

  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-30 grid place-items-center bg-slate-950/30 px-4">
      <div className="flex max-h-[88vh] w-full max-w-3xl flex-col overflow-hidden rounded-lg border border-slate-200 bg-white shadow-2xl">
        <header className="flex h-12 shrink-0 items-center justify-between border-b border-slate-200 px-4">
          <div className="flex min-w-0 items-center gap-2">
            <FilePlus size={17} className="text-emerald-700" />
            <h2 className="truncate text-sm font-semibold text-slate-950">新建任务</h2>
          </div>
          <button
            type="button"
            title="关闭"
            aria-label="关闭"
            onClick={resetAndClose}
            className="grid h-8 w-8 place-items-center rounded-md text-slate-500 hover:bg-slate-100 hover:text-slate-800"
          >
            <X size={16} />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-auto px-4 py-4">
          <div className="grid gap-4">
            <label className="flex flex-col gap-1.5 text-sm text-slate-700">
              <span className="font-medium">来源</span>
              <textarea
                value={sourceInput}
                onChange={(event) => setSourceInput(event.currentTarget.value)}
                onDrop={handleDrop}
                onDragOver={(event) => event.preventDefault()}
                onPaste={handlePaste}
                rows={4}
                className="min-h-24 resize-y rounded-md border border-slate-200 px-3 py-2 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
              />
            </label>

            <div className="flex flex-wrap items-center gap-2">
              {parsedSource ? (
                <span className="inline-flex h-7 items-center rounded-md border border-emerald-200 bg-emerald-50 px-2 text-xs font-medium text-emerald-800">
                  {sourceLabels[parsedSource.sourceType]}
                </span>
              ) : (
                <span className="inline-flex h-7 items-center rounded-md border border-amber-200 bg-amber-50 px-2 text-xs font-medium text-amber-800">
                  未识别
                </span>
              )}
              {parsedSource ? (
                compatibleSettings.length > 0 && (
                  <span className="text-xs text-slate-500">
                    {compatibleSettings.length} 个兼容引擎
                  </span>
                )
              ) : engineSettings.length > 0 ? (
                <span className="text-xs text-slate-500">
                  {engineSettings.length} 个已添加引擎
                </span>
              ) : null}
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                <span className="font-medium">文件名</span>
                <input
                  value={fileName}
                  onChange={(event) => setFileName(event.currentTarget.value)}
                  className="h-9 rounded-md border border-slate-200 px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                />
              </label>

              {parsedSource?.sourceType === "torrent" && torrentSelection && (
                <div className="md:col-span-2 rounded-md border border-slate-200 bg-slate-50 p-3">
                  <div className="mb-2 flex items-center justify-between gap-2">
                    <span className="text-sm font-medium text-slate-700">文件选择</span>
                    <div className="flex gap-2">
                      <button
                        type="button"
                        className="rounded-md border border-slate-200 px-2 py-1 text-xs text-slate-700 hover:bg-white"
                        onClick={() =>
                          setTorrentSelection({
                            ...torrentSelection,
                            selectedIndexes: new Set(torrentSelection.files.map((file) => file.index)),
                          })
                        }
                      >
                        全选
                      </button>
                      <button
                        type="button"
                        className="rounded-md border border-slate-200 px-2 py-1 text-xs text-slate-700 hover:bg-white"
                        onClick={() =>
                          setTorrentSelection({
                            ...torrentSelection,
                            selectedIndexes: new Set(),
                          })
                        }
                      >
                        取消
                      </button>
                    </div>
                  </div>
                  <div className="max-h-48 overflow-auto rounded border border-slate-200 bg-white">
                    {torrentSelection.files.map((file) => (
                      <label
                        key={file.index}
                        className="flex items-center gap-2 border-b border-slate-100 px-3 py-2 text-sm last:border-b-0"
                      >
                        <input
                          type="checkbox"
                          checked={torrentSelection.selectedIndexes.has(file.index)}
                          onChange={(event) =>
                            setTorrentSelection({
                              ...torrentSelection,
                              selectedIndexes: new Set(
                                event.currentTarget.checked
                                  ? [...torrentSelection.selectedIndexes, file.index]
                                  : [...torrentSelection.selectedIndexes].filter((value) => value !== file.index),
                              ),
                            })
                          }
                        />
                        <span className="min-w-0 flex-1 truncate">{file.path}</span>
                        <span className="text-xs text-slate-500">{file.length}</span>
                      </label>
                    ))}
                  </div>
                </div>
              )}

              <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                <span className="font-medium">引擎</span>
                <select
                  value={selectedEngineSettingsId}
                  disabled={isLoading}
                  onChange={(event) =>
                    setSelectedEngineSettingsId(event.currentTarget.value)
                  }
                  className="h-9 rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100 disabled:bg-slate-100 disabled:text-slate-400"
                >
                  <option value="">-</option>
                  {visibleEngineSettings.map((settings) => (
                    <option
                      key={settings.id}
                      value={settings.id}
                      disabled={!settings.enabled}
                    >
                      {engineOptionLabel(settings)}
                      {settings.enabled ? "" : " / 未启用"}
                    </option>
                  ))}
                </select>
              </label>

              <div className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                <span className="font-medium">
                  {selectedSettings?.engine === "qbittorrent" ? "远程保存路径" : "本地目录"}
                </span>
                <div className="flex min-w-0 gap-2">
                  <input
                    value={savePath}
                    onChange={(event) => setSavePath(event.currentTarget.value)}
                    className="h-9 min-w-0 flex-1 rounded-md border border-slate-200 px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                  />
                  {canSelectLocalSavePath && (
                    <button
                      type="button"
                      onClick={() => void selectSavePath()}
                      className="inline-flex h-9 shrink-0 items-center gap-1.5 rounded-md border border-slate-200 px-3 text-sm font-medium text-slate-700 hover:bg-slate-50"
                    >
                      <FolderOpen size={15} />
                      选择
                    </button>
                  )}
                </div>
              </div>

              <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700 md:col-span-2">
                <span className="font-medium">参数</span>
                <textarea
                  value={engineArgs}
                  onChange={(event) => setEngineArgs(event.currentTarget.value)}
                  rows={3}
                  className="min-h-20 resize-y rounded-md border border-slate-200 px-3 py-2 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                />
              </label>
            </div>

            {error && (
              <div className="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-700">
                {error}
              </div>
            )}
          </div>
        </div>

        <footer className="flex h-14 shrink-0 items-center justify-end gap-2 border-t border-slate-200 px-4">
          <button
            type="button"
            onClick={resetAndClose}
            className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50"
          >
            取消
          </button>
          <button
            type="button"
            disabled={!canCreate}
            onClick={() => void submitTask()}
            className={classNames(
              "h-9 rounded-md px-4 text-sm font-medium transition",
              canCreate
                ? "bg-emerald-700 text-white hover:bg-emerald-800"
                : "cursor-not-allowed bg-slate-100 text-slate-400",
            )}
          >
            创建
          </button>
        </footer>
      </div>
    </div>
  );
}
