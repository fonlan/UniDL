import { useEffect, useMemo, useState } from "react";
import type { ClipboardEvent, DragEvent } from "react";
import { ChevronRight, FilePlus, FolderOpen, X } from "lucide-react";
import { openDialog } from "@/lib/tauri";
import { reportDisplayedError } from "@/lib/error";

import {
  checkDownloadDuplicate,
  createDownloadTask,
  listEngineSettings,
  listRemoteDirectories,
  writeLog,
} from "@/lib/api";
import type {
  CreateDownloadTaskInput,
  DownloadDuplicateCheck,
  DownloadDuplicateKind,
  DownloadDuplicateMatch,
  DownloadStatus,
  DownloadTask,
  EngineKind,
  EngineSettings,
  FileConflictAction,
  RemoteDirectoryEntry,
  SourceType,
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

const duplicateKindLabels: Record<DownloadDuplicateKind, string> = {
  same_source: "来源 URL 相同",
  same_final_url: "最终 URL 相同",
  same_save_path: "保存路径和文件名相同",
  same_name_and_size: "文件名和大小相同",
  same_torrent_info_hash: "Torrent info hash 相同",
};

const statusLabels: Record<DownloadStatus, string> = {
  queued: "排队中",
  running: "下载中",
  paused: "已暂停",
  completed: "已完成",
  failed: "失败",
  deleted: "已删除",
};

const ERROR_AUTO_DISMISS_MS = 10_000;
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

interface ParsedSourceInput {
  sources: ParsedSource[];
  error: string | null;
}

interface RemoteDirectoryTreeState {
  open: boolean;
  path: string;
  entries: RemoteDirectoryEntry[];
  loading: boolean;
}

interface DuplicateTaskGroup {
  task: DownloadTask;
  taskState: DownloadDuplicateMatch["taskState"];
  kinds: DownloadDuplicateKind[];
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
      fileName: parseMagnetHash(source) ?? "magnet",
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

function parseSourceInput(value: string): ParsedSourceInput {
  const lines = value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0) {
    return { sources: [], error: null };
  }

  const sources: ParsedSource[] = [];
  for (const line of lines) {
    const parsed = parseSource(line);
    if (!parsed) {
      return { sources: [], error: "来源中包含无法识别的链接" };
    }
    sources.push(parsed);
  }

  const firstSourceType = sources[0]?.sourceType;
  if (
    firstSourceType &&
    sources.some((source) => source.sourceType !== firstSourceType)
  ) {
    return { sources: [], error: "批量添加仅支持相同类型的链接" };
  }

  return { sources, error: null };
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

function parseMagnetHash(value: string) {
  const match = /(?:^magnet:\?|[?&])xt=urn:btih:([^&]+)/i.exec(value);
  if (!match) {
    return null;
  }

  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}

function resolvedInitialFileName(
  initialFileName: string | null,
  parsedSource: ParsedSource | null,
) {
  const initial = initialFileName?.trim();
  if (!initial) {
    return parsedSource?.fileName ?? "";
  }

  if (parsedSource?.sourceType === "magnet" || parsedSource?.sourceType === "torrent") {
    return "";
  }

  return initial;
}

function valueFromDroppedFile(file: File) {
  const fileWithPath = file as File & { path?: string };
  return fileWithPath.path ?? file.name;
}

function sourceHostname(parsedSource: ParsedSource) {
  if (parsedSource.sourceType !== "http" && parsedSource.sourceType !== "ftp") {
    return null;
  }

  try {
    return new URL(parsedSource.source).hostname.toLowerCase();
  } catch {
    return null;
  }
}

function normalizePreferredDomain(domain: string) {
  return domain
    .trim()
    .toLowerCase()
    .replace(/^\*?\./, "");
}

function matchesPreferredDomain(settings: EngineSettings, hostname: string) {
  return settings.preferredDomains.some((domain) => {
    const normalized = normalizePreferredDomain(domain);
    return (
      normalized.length > 0 &&
      (hostname === normalized || hostname.endsWith(`.${normalized}`))
    );
  });
}

function defaultSavePath(settings: EngineSettings) {
  if (settings.engine === "qbittorrent") {
    return settings.remotePath ?? "";
  }

  return settings.defaultDownloadDir;
}

function groupDuplicateMatches(matches: DownloadDuplicateMatch[]) {
  const groups = new Map<string, DuplicateTaskGroup>();
  for (const match of matches) {
    const group = groups.get(match.task.id);
    if (group) {
      if (!group.kinds.includes(match.kind)) {
        group.kinds.push(match.kind);
      }
      continue;
    }

    groups.set(match.task.id, {
      task: match.task,
      taskState: match.taskState,
      kinds: [match.kind],
    });
  }

  return Array.from(groups.values());
}

export default function NewTaskDialog({
  open,
  initialSource = null,
  initialFileName = null,
  initialBrowserCookies = null,
  initialHttpReferrer = null,
  onClose,
  onCreated,
}: {
  open: boolean;
  initialSource?: string | null;
  initialFileName?: string | null;
  initialBrowserCookies?: string | null;
  initialHttpReferrer?: string | null;
  onClose: () => void;
  onCreated: (task: DownloadTask) => void;
}) {
  const [sourceInput, setSourceInput] = useState("");
  const [fileName, setFileName] = useState("");
  const [engineSettings, setEngineSettings] = useState<EngineSettings[]>([]);
  const [selectedEngineSettingsId, setSelectedEngineSettingsId] = useState("");
  const [savePath, setSavePath] = useState("");
  const [remoteDirectoryTree, setRemoteDirectoryTree] =
    useState<RemoteDirectoryTreeState | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [duplicateCheck, setDuplicateCheck] = useState<DownloadDuplicateCheck | null>(
    null,
  );
  const [pendingCreateInput, setPendingCreateInput] =
    useState<CreateDownloadTaskInput | null>(null);
  const [pendingBatchInputs, setPendingBatchInputs] = useState<CreateDownloadTaskInput[]>(
    [],
  );

  const parsedSourceInput = useMemo(() => parseSourceInput(sourceInput), [sourceInput]);
  const parsedSources = parsedSourceInput.sources;
  const parsedSource = parsedSources[0] ?? null;
  const isBatchMode = parsedSources.length > 1;
  const sourceInputError = parsedSourceInput.error;
  const compatibleSettings = useMemo(() => {
    if (!parsedSource) {
      return [];
    }
    const compatible = engineSettings.filter((settings) =>
      settings.supportedSourceTypes.includes(parsedSource.sourceType),
    );
    const hostname = sourceHostname(parsedSource);
    if (!hostname) {
      return compatible;
    }

    return [...compatible].sort((left, right) => {
      const leftMatched = matchesPreferredDomain(left, hostname);
      const rightMatched = matchesPreferredDomain(right, hostname);
      if (leftMatched !== rightMatched) {
        return leftMatched ? -1 : 1;
      }

      return left.priority - right.priority;
    });
  }, [engineSettings, parsedSource]);
  const visibleEngineSettings = parsedSource ? compatibleSettings : engineSettings;
  const selectedSettings =
    selectedEngineSettingsId === ""
      ? null
      : (engineSettings.find((settings) => settings.id === selectedEngineSettingsId) ??
        null);
  const canCreate =
    Boolean(parsedSource) &&
    !sourceInputError &&
    Boolean(selectedSettings?.enabled) &&
    (isBatchMode ||
      parsedSource?.sourceType === "magnet" ||
      parsedSource?.sourceType === "torrent" ||
      fileName.trim().length > 0) &&
    (selectedSettings?.engine === "qbittorrent" || savePath.trim().length > 0) &&
    !isCreating;
  const canSelectLocalSavePath = selectedSettings?.engine !== "qbittorrent";
  const canSelectRemoteSavePath =
    selectedSettings?.engine === "qbittorrent" && Boolean(selectedSettings.enabled);
  const duplicateTaskGroups = useMemo(
    () => (duplicateCheck ? groupDuplicateMatches(duplicateCheck.matches) : []),
    [duplicateCheck],
  );
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
        reportDisplayedError("create download task", nextError, setError);
      } finally {
        setIsLoading(false);
      }
    }

    void loadSettings();
  }, [open]);

  useEffect(() => {
    const nextFileName = resolvedInitialFileName(initialFileName, parsedSource);
    setFileName(isBatchMode ? "" : nextFileName);
  }, [initialFileName, isBatchMode, parsedSource]);

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
      setRemoteDirectoryTree(null);
      return;
    }

    setSavePath(defaultSavePath(selectedSettings));
    setRemoteDirectoryTree(null);
  }, [selectedSettings]);

  useEffect(() => {
    if (!error) {
      return;
    }

    const timerId = window.setTimeout(() => {
      setError(null);
    }, ERROR_AUTO_DISMISS_MS);

    return () => {
      window.clearTimeout(timerId);
    };
  }, [error]);

  function resetAndClose() {
    setSourceInput("");
    setFileName("");
    setSelectedEngineSettingsId("");
    setSavePath("");
    setError(null);
    setDuplicateCheck(null);
    setPendingCreateInput(null);
    setPendingBatchInputs([]);
    setRemoteDirectoryTree(null);
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
    if (parseSourceInput(text).sources.length > 0) {
      event.preventDefault();
      setSourceInput(text);
    }
  }

  function buildCreateInputs(): CreateDownloadTaskInput[] | null {
    if (parsedSources.length === 0 || !selectedSettings || sourceInputError) {
      return null;
    }

    return parsedSources.map((source) => ({
      sourceType: source.sourceType,
      source: source.source,
      engine: selectedSettings.engine,
      engineSettingsId: selectedSettings.id,
      fileName:
        (isBatchMode ? "" : fileName.trim()) ||
        source.fileName ||
        sourceLabels[source.sourceType],
      savePath: savePath.trim(),
      engineArgs: "",
      selectedFileIndexes: null,
      browserCookies: initialBrowserCookies,
      httpReferrer: initialHttpReferrer,
      fileConflictAction: "prompt",
    }));
  }

  async function createTaskRecord(
    input: CreateDownloadTaskInput,
    action: FileConflictAction,
  ) {
    const task = await createDownloadTask({
      ...input,
      fileConflictAction: action,
    });
    onCreated(task);
    void writeLog("info", `new task created: id=${task.id}`);
  }

  async function createInputsUntilBlocked(inputs: CreateDownloadTaskInput[]) {
    for (const [index, input] of inputs.entries()) {
      void writeLog(
        "info",
        `submitting new task: engine=${input.engine}, sourceType=${input.sourceType}`,
      );
      const check = await checkDownloadDuplicate(input);
      if (check.matches.length > 0 || check.localFileConflict) {
        setDuplicateCheck(check);
        setPendingCreateInput(input);
        setPendingBatchInputs(inputs.slice(index + 1));
        return;
      }

      await createTaskRecord(input, "prompt");
    }

    resetAndClose();
  }

  async function submitTask() {
    const inputs = buildCreateInputs();
    if (!inputs || inputs.length === 0 || !selectedSettings) {
      return;
    }

    setIsCreating(true);
    setError(null);
    setDuplicateCheck(null);
    setPendingCreateInput(null);
    setPendingBatchInputs([]);

    try {
      await createInputsUntilBlocked(inputs);
    } catch (nextError) {
      reportDisplayedError("create download task", nextError, setError);
    } finally {
      setIsCreating(false);
    }
  }

  async function skipDuplicateCheck() {
    const restInputs = pendingBatchInputs;
    setDuplicateCheck(null);
    setPendingCreateInput(null);
    setPendingBatchInputs([]);

    if (restInputs.length === 0) {
      return;
    }

    setIsCreating(true);
    setError(null);
    try {
      await createInputsUntilBlocked(restInputs);
    } catch (nextError) {
      reportDisplayedError("create download task", nextError, setError);
    } finally {
      setIsCreating(false);
    }
  }

  async function createPendingTask(action: FileConflictAction) {
    if (!pendingCreateInput) {
      return;
    }

    const input = pendingCreateInput;
    const restInputs = pendingBatchInputs;
    setIsCreating(true);
    setError(null);
    setDuplicateCheck(null);
    setPendingCreateInput(null);
    setPendingBatchInputs([]);

    try {
      await createTaskRecord(input, action);
      await createInputsUntilBlocked(restInputs);
    } catch (nextError) {
      reportDisplayedError("create download task", nextError, setError);
    } finally {
      setIsCreating(false);
    }
  }

  function redownloadDuplicateTask() {
    if (!pendingCreateInput || duplicateCheck?.localFileConflict) {
      return;
    }
    void createPendingTask("prompt");
  }

  function resolveDuplicateCheck(action: Exclude<FileConflictAction, "prompt">) {
    if (!pendingCreateInput) {
      return;
    }
    void createPendingTask(action);
  }

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

  async function selectTorrentFile() {
    const selected = await openDialog({
      multiple: false,
      filters: [
        {
          name: "Torrent",
          extensions: ["torrent"],
        },
      ],
      title: "选择本地 torrent 文件",
    });

    if (typeof selected === "string") {
      setSourceInput(selected);
    }
  }

  async function loadRemoteDirectories(path: string) {
    if (!selectedSettings) {
      return;
    }

    const currentPath = path.trim();
    setRemoteDirectoryTree({
      open: true,
      path: currentPath,
      entries: [],
      loading: true,
    });
    setError(null);

    try {
      const entries = await listRemoteDirectories(selectedSettings.id, currentPath);
      setRemoteDirectoryTree({
        open: true,
        path: currentPath,
        entries,
        loading: false,
      });
    } catch (nextError) {
      setRemoteDirectoryTree(null);
      reportDisplayedError("create download task", nextError, setError);
    }
  }

  function selectRemoteDirectory(path: string) {
    setSavePath(path);
    setRemoteDirectoryTree(null);
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
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium">来源</span>
                <button
                  type="button"
                  onClick={() => void selectTorrentFile()}
                  className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-slate-200 px-3 text-sm font-medium text-slate-700 hover:bg-slate-50"
                >
                  <FolderOpen size={15} />
                  选择 torrent 文件
                </button>
              </div>
              <textarea
                value={sourceInput}
                onChange={(event) => setSourceInput(event.currentTarget.value)}
                onDrop={handleDrop}
                onDragOver={(event) => event.preventDefault()}
                onPaste={handlePaste}
                rows={4}
                className="min-h-24 resize-y rounded-md border border-slate-200 px-3 py-2 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
              />
              {sourceInputError ? (
                <span className="text-xs text-rose-600">{sourceInputError}</span>
              ) : isBatchMode ? (
                <span className="text-xs text-slate-500">
                  将按相同类型、相同目录和相同引擎创建 {parsedSources.length} 个任务。
                </span>
              ) : (
                <span className="text-xs text-slate-500">
                  支持一行一个链接批量添加，同一批链接必须类型相同。
                </span>
              )}
            </label>

            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
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
                {isBatchMode && (
                  <span className="text-xs text-slate-500">
                    批量 {parsedSources.length} 个
                  </span>
                )}
              </div>
              <label className="flex min-w-0 items-center gap-2 text-sm text-slate-700">
                <span className="shrink-0 font-medium">引擎</span>
                <select
                  value={selectedEngineSettingsId}
                  disabled={isLoading}
                  onChange={(event) =>
                    setSelectedEngineSettingsId(event.currentTarget.value)
                  }
                  className="h-9 w-56 rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100 disabled:bg-slate-100 disabled:text-slate-400"
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
            </div>

            <div className="grid gap-4">
              <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                <span className="font-medium">文件名</span>
                <input
                  value={fileName}
                  disabled={
                    isBatchMode ||
                    parsedSource?.sourceType === "magnet" ||
                    parsedSource?.sourceType === "torrent"
                  }
                  onChange={(event) => setFileName(event.currentTarget.value)}
                  className="h-9 rounded-md border border-slate-200 px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100 disabled:bg-slate-100 disabled:text-slate-400"
                />
                {isBatchMode ? (
                  <span className="text-xs text-slate-500">
                    批量模式下文件名保持为空，由每个链接自行推断。
                  </span>
                ) : (
                  (parsedSource?.sourceType === "magnet" ||
                    parsedSource?.sourceType === "torrent") && (
                    <span className="text-xs text-slate-500">
                      BT/Magnet 任务创建时不解析文件名，详情页会显示文件列表。
                    </span>
                  )
                )}
              </label>

              <div className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                <span className="font-medium">
                  {selectedSettings?.engine === "qbittorrent"
                    ? "远程保存路径"
                    : "本地目录"}
                </span>
                <div className="flex min-w-0 gap-2">
                  <input
                    title={
                      selectedSettings?.engine === "qbittorrent"
                        ? "远程保存路径"
                        : "本地目录"
                    }
                    aria-label={
                      selectedSettings?.engine === "qbittorrent"
                        ? "远程保存路径"
                        : "本地目录"
                    }
                    placeholder={
                      selectedSettings?.engine === "qbittorrent"
                        ? "留空使用 qBittorrent 默认保存路径"
                        : undefined
                    }
                    value={savePath}
                    onChange={(event) => setSavePath(event.currentTarget.value)}
                    className="h-9 min-w-0 flex-1 rounded-md border border-slate-200 px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                  />
                  {canSelectLocalSavePath && (
                    <button
                      type="button"
                      title="选择下载目录"
                      aria-label="选择下载目录"
                      onClick={() => void selectSavePath()}
                      className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-slate-200 text-slate-700 hover:bg-slate-50"
                    >
                      <FolderOpen size={15} />
                    </button>
                  )}
                  {canSelectRemoteSavePath && (
                    <button
                      type="button"
                      title="选择远程目录"
                      aria-label="选择远程目录"
                      onClick={() => void loadRemoteDirectories(savePath)}
                      className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-slate-200 text-slate-700 hover:bg-slate-50"
                    >
                      <FolderOpen size={15} />
                    </button>
                  )}
                </div>
                {remoteDirectoryTree?.open && (
                  <div className="rounded-md border border-slate-200 bg-slate-50 p-2">
                    <div className="mb-2 flex items-center justify-between gap-2 text-xs text-slate-500">
                      <span className="min-w-0 truncate">{remoteDirectoryTree.path}</span>
                      <button
                        type="button"
                        onClick={() => selectRemoteDirectory(remoteDirectoryTree.path)}
                        className="shrink-0 rounded border border-slate-200 bg-white px-2 py-1 text-slate-700 hover:bg-slate-100"
                      >
                        选中当前目录
                      </button>
                    </div>
                    {remoteDirectoryTree.loading ? (
                      <div className="px-2 py-1 text-xs text-slate-500">
                        正在读取远程目录…
                      </div>
                    ) : remoteDirectoryTree.entries.length > 0 ? (
                      <div className="max-h-48 overflow-auto rounded border border-slate-200 bg-white">
                        {remoteDirectoryTree.entries.map((entry) => (
                          <button
                            key={entry.path}
                            type="button"
                            onClick={() => void loadRemoteDirectories(entry.path)}
                            onDoubleClick={() => selectRemoteDirectory(entry.path)}
                            className="flex w-full items-center gap-2 border-b border-slate-100 px-3 py-2 text-left text-sm text-slate-700 last:border-b-0 hover:bg-slate-50"
                          >
                            <ChevronRight size={14} className="shrink-0 text-slate-400" />
                            <span className="min-w-0 flex-1 truncate">{entry.name}</span>
                          </button>
                        ))}
                      </div>
                    ) : (
                      <div className="px-2 py-1 text-xs text-slate-500">没有子目录</div>
                    )}
                  </div>
                )}
              </div>
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
            {isBatchMode ? `创建 ${parsedSources.length} 个` : "创建"}
          </button>
        </footer>
      </div>
      {duplicateCheck && (
        <div className="fixed inset-0 z-40 grid place-items-center bg-slate-950/40 px-4">
          <div className="flex max-h-[80vh] w-full max-w-2xl flex-col rounded-lg border border-slate-200 bg-white shadow-2xl">
            <div className="border-b border-slate-200 px-4 py-3">
              <h3 className="text-sm font-semibold text-slate-950">
                发现重复任务或文件冲突
              </h3>
              <p className="mt-1 text-sm text-slate-600">
                请确认是否继续创建新任务；不会自动打开已有任务。
              </p>
            </div>
            <div className="min-h-0 overflow-auto px-4 py-3">
              <div className="grid gap-3">
                {duplicateTaskGroups.length > 0 && (
                  <div className="grid gap-2">
                    {duplicateTaskGroups.map((group) => (
                      <div
                        key={group.task.id}
                        className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-sm text-amber-900"
                      >
                        <div className="flex min-w-0 flex-wrap items-center gap-2">
                          <span className="min-w-0 truncate font-medium">
                            {group.task.fileName || group.task.source}
                          </span>
                          <span className="shrink-0 rounded bg-white/70 px-1.5 py-0.5 text-xs text-amber-800">
                            {group.taskState === "active"
                              ? "已存在未完成任务"
                              : "已完成相同任务"}
                          </span>
                          <span className="shrink-0 text-xs text-amber-700">
                            {statusLabels[group.task.status]}
                          </span>
                        </div>
                        <div className="mt-1 break-all text-xs text-amber-800">
                          {group.task.savePath}
                        </div>
                        <div className="mt-2 flex flex-wrap gap-1.5">
                          {group.kinds.map((kind) => (
                            <span
                              key={kind}
                              className="rounded border border-amber-200 bg-white px-1.5 py-0.5 text-xs text-amber-800"
                            >
                              {duplicateKindLabels[kind]}
                            </span>
                          ))}
                        </div>
                      </div>
                    ))}
                  </div>
                )}

                {duplicateCheck.localFileConflict && (
                  <div className="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-800">
                    <div className="font-medium">本地文件已存在</div>
                    <div className="mt-1">
                      {duplicateCheck.localFileConflict.fileName}
                    </div>
                    <div className="mt-1 break-all text-xs">
                      {duplicateCheck.localFileConflict.path}
                    </div>
                  </div>
                )}

                {duplicateCheck.localFileConflict && duplicateTaskGroups.length > 0 && (
                  <div className="rounded-md border border-slate-200 bg-slate-50 px-3 py-2 text-xs text-slate-600">
                    由于本地存在同名文件，如需继续创建请改用自动重命名或覆盖。
                  </div>
                )}
              </div>
            </div>
            <div className="flex flex-wrap justify-end gap-2 border-t border-slate-200 px-4 py-3">
              <button
                type="button"
                disabled={isCreating}
                onClick={skipDuplicateCheck}
                className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:bg-slate-100 disabled:text-slate-400"
              >
                跳过
              </button>
              {duplicateTaskGroups.length > 0 && (
                <button
                  type="button"
                  disabled={isCreating || Boolean(duplicateCheck.localFileConflict)}
                  onClick={redownloadDuplicateTask}
                  className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:bg-slate-100 disabled:text-slate-400"
                >
                  重新下载
                </button>
              )}
              {duplicateCheck.localFileConflict && (
                <>
                  <button
                    type="button"
                    disabled={isCreating}
                    onClick={() => resolveDuplicateCheck("rename")}
                    className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50 disabled:cursor-not-allowed disabled:bg-slate-100 disabled:text-slate-400"
                  >
                    自动重命名
                  </button>
                  <button
                    type="button"
                    disabled={isCreating}
                    onClick={() => resolveDuplicateCheck("overwrite")}
                    className="h-9 rounded-md bg-rose-600 px-4 text-sm font-medium text-white hover:bg-rose-700 disabled:cursor-not-allowed disabled:bg-slate-100 disabled:text-slate-400"
                  >
                    覆盖
                  </button>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
