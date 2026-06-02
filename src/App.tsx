import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
  ReactNode,
} from "react";
import {
  ArrowLeft,
  Check,
  Copy,
  Eye,
  EyeOff,
  FolderOpen,
  Minus,
  Pause,
  Play,
  Plus,
  RefreshCw,
  Search,
  Settings,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { getCurrentWindow, listen, message } from "@/lib/tauri";
import { reportDisplayedError } from "@/lib/error";
import EngineSettingsView from "@/components/EngineSettingsView";
import NewTaskDialog from "@/components/NewTaskDialog";
import TaskDetailPanel from "@/components/TaskDetailPanel";
import logoUrl from "../logo.png";
import {
  createDownloadTask,
  deleteDownloadTasks,
  getAppSettings,
  openDownloadDirectory,
  openDownloadedFile,
  readClipboardText,
  pauseAllUnfinishedDownloadTasks,
  pauseDownloadTasks,
  refreshDownloadTasks,
  resumeAllPausedDownloadTasks,
  resumeDownloadTasks,
  takePendingOpenRequests,
  writeLog,
} from "@/lib/api";
import type { OpenTaskRequest, SystemOpenRequestPayload } from "@/lib/api";
import { getWebToken, hasTauriRuntime, isWebRuntime, webLogin } from "@/lib/runtime";
import type {
  AppSettings,
  DownloadStatus,
  DownloadTask,
  EngineKind,
  SourceType,
} from "@shared/types";

const statusLabels: Record<DownloadStatus, string> = {
  queued: "排队中",
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

type TaskColumnKey =
  | "selected"
  | "fileName"
  | "engine"
  | "status"
  | "progress"
  | "size"
  | "speed"
  | "savePath"
  | "createdAt"
  | "completedAt";

type TaskColumn = {
  key: TaskColumnKey;
  label: string;
  width: number;
  minWidth: number;
  resizable: boolean;
};

type TaskContextMenuState = {
  taskId: string;
  x: number;
  y: number;
};

const taskTableColumns: TaskColumn[] = [
  { key: "selected", label: "选择", width: 48, minWidth: 48, resizable: false },
  { key: "fileName", label: "文件名", width: 220, minWidth: 140, resizable: true },
  { key: "engine", label: "引擎", width: 112, minWidth: 84, resizable: true },
  { key: "status", label: "状态", width: 112, minWidth: 84, resizable: true },
  { key: "progress", label: "进度", width: 176, minWidth: 140, resizable: true },
  { key: "size", label: "已下载/总大小", width: 152, minWidth: 124, resizable: true },
  { key: "speed", label: "速度", width: 112, minWidth: 92, resizable: true },
  { key: "savePath", label: "路径", width: 260, minWidth: 160, resizable: true },
  { key: "createdAt", label: "创建时间", width: 128, minWidth: 104, resizable: true },
  { key: "completedAt", label: "完成时间", width: 128, minWidth: 104, resizable: true },
];

const centeredTaskColumnKeys = new Set<TaskColumnKey>([
  "engine",
  "status",
  "progress",
  "size",
  "speed",
]);

function isFinished(status: DownloadStatus) {
  return status === "completed" || status === "failed" || status === "deleted";
}

function isResumableTask(task: DownloadTask) {
  return task.engine === "aria2" || task.engine === "yt-dlp";
}

function isLocalDownloadEngine(engine: EngineKind) {
  return engine === "aria2" || engine === "yt-dlp";
}

function classNames(...names: Array<string | false | null | undefined>) {
  return names.filter(Boolean).join(" ");
}

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

function clipboardDownloadSource(value: string) {
  const source = value.trim();
  return /^(?:https?:\/\/|magnet:)/i.test(source) ? source : null;
}

const ERROR_AUTO_DISMISS_MS = 10_000;

function IconButton({
  title,
  disabled,
  onClick,
  children,
  tone = "neutral",
}: {
  title: string;
  disabled?: boolean;
  onClick?: () => void;
  children: ReactNode;
  tone?: "neutral" | "primary" | "danger";
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={classNames(
        "grid h-9 w-9 place-items-center rounded-md border text-sm transition",
        "shrink-0",
        "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2",
        disabled && "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
        !disabled &&
          tone === "neutral" &&
          "border-slate-200 bg-white text-slate-700 hover:border-slate-300 hover:bg-slate-50 focus-visible:outline-slate-500",
        !disabled &&
          tone === "primary" &&
          "border-emerald-700 bg-emerald-700 text-white hover:bg-emerald-800 focus-visible:outline-emerald-500",
        !disabled &&
          tone === "danger" &&
          "border-rose-200 bg-white text-rose-700 hover:border-rose-300 hover:bg-rose-50 focus-visible:outline-rose-500",
      )}
    >
      {children}
    </button>
  );
}

function StatusBadge({ status }: { status: DownloadStatus }) {
  const styles: Record<DownloadStatus, string> = {
    queued: "border-slate-200 bg-slate-50 text-slate-700",
    running: "border-emerald-200 bg-emerald-50 text-emerald-700",
    paused: "border-amber-200 bg-amber-50 text-amber-800",
    completed: "border-sky-200 bg-sky-50 text-sky-700",
    failed: "border-rose-200 bg-rose-50 text-rose-700",
    deleted: "border-slate-200 bg-slate-50 text-slate-500",
  };

  return (
    <span
      className={classNames(
        "inline-flex h-6 items-center rounded-md border px-2 text-xs font-medium",
        styles[status],
      )}
    >
      {statusLabels[status]}
    </span>
  );
}

function MobileTaskCard({
  task,
  isSelected,
  isActive,
  onOpen,
  onDoubleClick,
  onContextMenu,
  onToggleSelected,
}: {
  task: DownloadTask;
  isSelected: boolean;
  isActive: boolean;
  onOpen: () => void;
  onDoubleClick: () => void;
  onContextMenu: (event: ReactMouseEvent<HTMLElement>) => void;
  onToggleSelected: () => void;
}) {
  const isDownloadedFileMissing = task.downloadedFileMissing;

  return (
    <article
      onClick={onOpen}
      onDoubleClick={onDoubleClick}
      onContextMenu={onContextMenu}
      className={classNames(
        "cursor-pointer rounded-lg border bg-white p-3 shadow-sm transition",
        "hover:border-slate-300 hover:bg-slate-50",
        isDownloadedFileMissing && "text-slate-400 opacity-70",
        isActive && "border-sky-200 bg-sky-50 hover:bg-sky-50",
        isSelected && "border-emerald-200 bg-emerald-50 hover:bg-emerald-50",
      )}
    >
      <div className="flex min-w-0 items-start gap-3">
        <button
          type="button"
          title="选择任务"
          aria-label="选择任务"
          onClick={(event) => {
            event.stopPropagation();
            onToggleSelected();
          }}
          className={classNames(
            "mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded border bg-white",
            isSelected
              ? "border-emerald-700 text-emerald-700"
              : "border-slate-300 text-transparent",
          )}
        >
          <Check size={15} strokeWidth={3} />
        </button>

        <div className="min-w-0 flex-1">
          <div
            className={classNames(
              "truncate text-sm font-semibold text-slate-900",
              isDownloadedFileMissing && "text-slate-500 line-through",
            )}
            title={
              isDownloadedFileMissing
                ? `${task.fileName}（文件已不存在）`
                : task.fileName
            }
          >
            {task.fileName}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2 text-xs text-slate-500">
            <span>{sourceLabels[task.sourceType]}</span>
            <span className="h-1 w-1 rounded-full bg-slate-300" />
            <span>{engineLabels[task.engine]}</span>
          </div>
        </div>

        <StatusBadge status={task.status} />
      </div>

      <div className="mt-3 space-y-1.5">
        <div className="flex items-center justify-between gap-3 text-xs text-slate-600">
          <span>进度</span>
          <span className="tabular-nums">{task.progress.toFixed(1)}%</span>
        </div>
        <progress
          className="task-progress block"
          value={Math.min(100, Math.max(0, task.progress))}
          max={100}
          aria-label={`${task.fileName} 下载进度`}
        />
      </div>

      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-md bg-slate-50 px-2 py-1.5">
          <div className="text-slate-500">速度</div>
          <div className="mt-0.5 truncate tabular-nums text-slate-800">
            {formatSpeed(task.speedBytesPerSec)}
          </div>
        </div>
        <div className="rounded-md bg-slate-50 px-2 py-1.5">
          <div className="text-slate-500">大小</div>
          <div className="mt-0.5 truncate tabular-nums text-slate-800">
            {formatBytes(task.downloadedBytes)} / {formatBytes(task.totalBytes)}
          </div>
        </div>
        <div className="rounded-md bg-slate-50 px-2 py-1.5">
          <div className="text-slate-500">创建</div>
          <div className="mt-0.5 truncate tabular-nums text-slate-800">
            {formatDate(task.createdAt)}
          </div>
        </div>
        <div className="rounded-md bg-slate-50 px-2 py-1.5">
          <div className="text-slate-500">完成</div>
          <div className="mt-0.5 truncate tabular-nums text-slate-800">
            {formatDate(task.completedAt)}
          </div>
        </div>
      </div>

      <div className="mt-3 rounded-md border border-slate-100 bg-slate-50 px-2 py-1.5 text-xs">
        <div className="text-slate-500">保存路径</div>
        <div className="mt-0.5 truncate text-slate-700" title={task.savePath}>
          {task.savePath}
        </div>
      </div>
    </article>
  );
}

function App() {
  const [themeMode, setThemeMode] = useState<AppSettings["themeMode"]>("light");
  const [view, setView] = useState<"tasks" | "settings">("tasks");
  const [showNewTaskDialog, setShowNewTaskDialog] = useState(false);
  const [newTaskInitialSource, setNewTaskInitialSource] = useState<string | null>(null);
  const [newTaskInitialFileName, setNewTaskInitialFileName] = useState<string | null>(
    null,
  );
  const [newTaskInitialBrowserCookies, setNewTaskInitialBrowserCookies] = useState<
    string | null
  >(null);
  const [newTaskInitialHttpReferrer, setNewTaskInitialHttpReferrer] = useState<
    string | null
  >(null);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [isSearchOpen, setIsSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [taskContextMenu, setTaskContextMenu] = useState<TaskContextMenuState | null>(
    null,
  );
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [webPassword, setWebPassword] = useState("");
  const [isWebPasswordVisible, setIsWebPasswordVisible] = useState(false);
  const [isWebAuthenticating, setIsWebAuthenticating] = useState(false);
  const [isWebAuthorized, setIsWebAuthorized] = useState(
    () => !isWebRuntime() || !!getWebToken(),
  );

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

  const [detailTaskId, setDetailTaskId] = useState<string | null>(null);
  const [isWindowMaximized, setIsWindowMaximized] = useState(false);
  const [taskColumnWidths, setTaskColumnWidths] = useState<Record<TaskColumnKey, number>>(
    () =>
      Object.fromEntries(
        taskTableColumns.map((column) => [column.key, column.width]),
      ) as Record<TaskColumnKey, number>,
  );
  const hasLoadedTasksRef = useRef(false);
  const taskTableRef = useRef<HTMLTableElement | null>(null);
  const taskContextMenuPanelRef = useRef<HTMLDivElement | null>(null);
  const searchContainerRef = useRef<HTMLDivElement | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const deleteDialogResolveRef = useRef<((value: boolean | null) => void) | null>(null);

  const loadThemeMode = useCallback(async () => {
    const settings = await getAppSettings();
    setThemeMode(settings.themeMode);
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", themeMode === "dark");
  }, [themeMode]);

  useEffect(() => {
    if (isWebRuntime() && !isWebAuthorized) {
      return;
    }

    void loadThemeMode().catch((nextError) => {
      reportDisplayedError("load theme settings", nextError, setError);
    });
  }, [isWebAuthorized, loadThemeMode]);

  const normalizedSearchQuery = searchQuery.trim().toLowerCase();
  const visibleTasks = useMemo(() => {
    if (!normalizedSearchQuery) {
      return tasks;
    }

    return tasks.filter((task) => {
      const fileName = task.fileName.toLowerCase();
      const source = task.source.toLowerCase();
      return (
        fileName.includes(normalizedSearchQuery) || source.includes(normalizedSearchQuery)
      );
    });
  }, [normalizedSearchQuery, tasks]);

  const selectedTasks = useMemo(
    () => visibleTasks.filter((task) => selectedIds.has(task.id)),
    [selectedIds, visibleTasks],
  );
  const detailTask = useMemo(
    () => tasks.find((task) => task.id === detailTaskId) ?? null,
    [detailTaskId, tasks],
  );
  const contextMenuTask = useMemo(
    () => tasks.find((task) => task.id === taskContextMenu?.taskId) ?? null,
    [taskContextMenu, tasks],
  );
  const allVisibleSelected =
    visibleTasks.length > 0 && visibleTasks.every((task) => selectedIds.has(task.id));
  const selectedScopeTasks = selectedTasks.length > 0 ? selectedTasks : visibleTasks;
  const activeScopeTasks = selectedScopeTasks.filter((task) => !isFinished(task.status));
  const failedScopeTasks = selectedScopeTasks.filter((task) => task.status === "failed");
  const pausedScopeTasks = activeScopeTasks.filter((task) => task.status === "paused");
  const shouldResume =
    failedScopeTasks.length > 0 ||
    (activeScopeTasks.length > 0 && pausedScopeTasks.length === activeScopeTasks.length);
  const toggleDisabled = activeScopeTasks.length === 0 && failedScopeTasks.length === 0;
  const deleteDisabled = selectedTasks.length === 0;
  const totalTaskTableWidth = taskTableColumns.reduce(
    (sum, column) => sum + taskColumnWidths[column.key],
    0,
  );

  useLayoutEffect(() => {
    if (taskTableRef.current) {
      taskTableRef.current.style.minWidth = `${totalTaskTableWidth}px`;
    }
  }, [totalTaskTableWidth]);

  useLayoutEffect(() => {
    if (!taskContextMenu || !taskContextMenuPanelRef.current) {
      return;
    }

    const panel = taskContextMenuPanelRef.current;
    const panelRect = panel.getBoundingClientRect();
    const margin = 8;
    const left = Math.max(
      margin,
      Math.min(taskContextMenu.x, window.innerWidth - panelRect.width - margin),
    );
    const top = Math.max(
      margin,
      Math.min(taskContextMenu.y, window.innerHeight - panelRect.height - margin),
    );

    panel.style.left = `${left}px`;
    panel.style.top = `${top}px`;
  }, [taskContextMenu]);

  useEffect(() => {
    if (isSearchOpen) {
      searchInputRef.current?.focus();
    }
  }, [isSearchOpen]);

  useEffect(() => {
    function focusSearchInputOnShortcut(event: KeyboardEvent) {
      const isSearchShortcut =
        event.ctrlKey &&
        !event.altKey &&
        !event.metaKey &&
        !event.shiftKey &&
        event.key.toLowerCase() === "f";

      if (!isSearchShortcut || view !== "tasks") {
        return;
      }

      event.preventDefault();
      setIsSearchOpen(true);
      window.requestAnimationFrame(() => searchInputRef.current?.focus());
    }

    window.addEventListener("keydown", focusSearchInputOnShortcut);

    return () => {
      window.removeEventListener("keydown", focusSearchInputOnShortcut);
    };
  }, [view]);

  useEffect(() => {
    if (!isSearchOpen || normalizedSearchQuery) {
      return;
    }

    function closeEmptySearchOnOutsideClick(event: PointerEvent) {
      const target = event.target;
      if (
        target instanceof Node &&
        searchContainerRef.current &&
        searchContainerRef.current.contains(target)
      ) {
        return;
      }

      setIsSearchOpen(false);
    }

    window.addEventListener("pointerdown", closeEmptySearchOnOutsideClick);

    return () => {
      window.removeEventListener("pointerdown", closeEmptySearchOnOutsideClick);
    };
  }, [isSearchOpen, normalizedSearchQuery]);

  function handleColumnResizeStart(
    event: ReactPointerEvent<HTMLButtonElement>,
    column: TaskColumn,
  ) {
    if (!column.resizable || event.button !== 0) {
      return;
    }

    event.preventDefault();
    event.stopPropagation();

    const startX = event.clientX;
    const startWidth = taskColumnWidths[column.key];
    const originalCursor = document.body.style.cursor;
    const originalUserSelect = document.body.style.userSelect;

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    function handlePointerMove(pointerEvent: PointerEvent) {
      const nextWidth = Math.max(
        column.minWidth,
        startWidth + pointerEvent.clientX - startX,
      );
      setTaskColumnWidths((current) => ({ ...current, [column.key]: nextWidth }));
    }

    function handlePointerUp() {
      document.removeEventListener("pointermove", handlePointerMove);
      document.removeEventListener("pointerup", handlePointerUp);
      document.body.style.cursor = originalCursor;
      document.body.style.userSelect = originalUserSelect;
    }

    document.addEventListener("pointermove", handlePointerMove);
    document.addEventListener("pointerup", handlePointerUp, { once: true });
  }

  async function confirmDeleteCompletedFiles() {
    setShowDeleteDialog(true);
    return new Promise<boolean | null>((resolve) => {
      deleteDialogResolveRef.current = resolve;
    });
  }

  function resolveDeleteDialog(value: boolean | null) {
    deleteDialogResolveRef.current?.(value);
    deleteDialogResolveRef.current = null;
    setShowDeleteDialog(false);
  }

  const replaceTasks = useCallback((nextTasks: DownloadTask[]) => {
    setTasks(nextTasks);
    setSelectedIds((current) => {
      const nextIds = new Set(nextTasks.map((task) => task.id));
      return new Set([...current].filter((id) => nextIds.has(id)));
    });
  }, []);

  const refreshTasks = useCallback(async () => {
    const shouldShowLoading = !hasLoadedTasksRef.current;
    if (shouldShowLoading) {
      setIsLoading(true);
    }
    setError(null);

    try {
      const nextTasks = await refreshDownloadTasks();
      replaceTasks(nextTasks);
    } catch (nextError) {
      if (isWebRuntime() && !getWebToken()) {
        setIsWebAuthorized(false);
      }
      reportDisplayedError("load pending open requests", nextError, setError);
    } finally {
      hasLoadedTasksRef.current = true;
      if (shouldShowLoading) {
        setIsLoading(false);
      }
    }
  }, [replaceTasks]);

  const syncWindowMaximizedState = useCallback(async () => {
    setIsWindowMaximized(await getCurrentWindow().isMaximized());
  }, []);

  useEffect(() => {
    void writeLog("info", "task view mounted");
    if (isWebRuntime() && !isWebAuthorized) {
      setIsLoading(false);
      return;
    }
    void refreshTasks();
  }, [isWebAuthorized, refreshTasks]);

  useEffect(() => {
    if (!hasTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;
    const currentWindow = getCurrentWindow();

    void syncWindowMaximizedState();
    void currentWindow
      .onResized(() => {
        void syncWindowMaximizedState();
      })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
        } else {
          unlisten = nextUnlisten;
        }
      })
      .catch((nextError) => {
        if (disposed) {
          return;
        }
        reportDisplayedError("load pending open requests", nextError, setError);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [syncWindowMaximizedState]);

  useEffect(() => {
    if (detailTaskId && !detailTask) {
      setDetailTaskId(null);
    }
  }, [detailTask, detailTaskId]);

  useEffect(() => {
    if (detailTaskId && !visibleTasks.some((task) => task.id === detailTaskId)) {
      setDetailTaskId(null);
    }
  }, [detailTaskId, visibleTasks]);

  useEffect(() => {
    if (taskContextMenu && !contextMenuTask) {
      setTaskContextMenu(null);
    }
  }, [contextMenuTask, taskContextMenu]);

  useEffect(() => {
    if (!taskContextMenu) {
      return;
    }

    function handleCloseContextMenu() {
      setTaskContextMenu(null);
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setTaskContextMenu(null);
      }
    }

    window.addEventListener("pointerdown", handleCloseContextMenu);
    window.addEventListener("resize", handleCloseContextMenu);
    window.addEventListener("blur", handleCloseContextMenu);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("pointerdown", handleCloseContextMenu);
      window.removeEventListener("resize", handleCloseContextMenu);
      window.removeEventListener("blur", handleCloseContextMenu);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [taskContextMenu]);

  useEffect(() => {
    if (!hasTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;

    void listen("download-tasks-updated", () => {
      void refreshTasks();
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
        } else {
          unlisten = nextUnlisten;
        }
      })
      .catch((nextError) => {
        if (disposed) {
          return;
        }
        reportDisplayedError("load pending open requests", nextError, setError);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [refreshTasks]);

  useEffect(() => {
    if (!isWebRuntime() || !isWebAuthorized) {
      return;
    }

    const timerId = window.setInterval(() => {
      void refreshTasks();
    }, 2_000);

    return () => {
      window.clearInterval(timerId);
    };
  }, [isWebAuthorized, refreshTasks]);

  useEffect(() => {
    if (!hasTauriRuntime()) {
      return;
    }

    let disposed = false;
    let unlisten: (() => void) | null = null;

    const openRequests = (requests: OpenTaskRequest[]) => {
      const [request] = requests;
      if (!request) {
        return;
      }
      void writeLog(
        "info",
        `opening task dialog from system source: count=${requests.length}`,
      );
      setView("tasks");
      setNewTaskInitialSource(request.source);
      setNewTaskInitialFileName(request.fileName ?? null);
      setNewTaskInitialBrowserCookies(request.browserCookies ?? null);
      setNewTaskInitialHttpReferrer(request.httpReferrer ?? null);
      setShowNewTaskDialog(true);
    };

    const openPendingRequests = (fallbackRequests: OpenTaskRequest[] = []) => {
      void takePendingOpenRequests()
        .then((requests) => {
          if (disposed) {
            return;
          }
          openRequests(requests.length > 0 ? requests : fallbackRequests);
        })
        .catch((nextError) => {
          if (disposed) {
            return;
          }
          reportDisplayedError("load pending open requests", nextError, setError);
        });
    };

    void listen<SystemOpenRequestPayload>("system-open-request", (event) => {
      openPendingRequests(event.payload.requests);
    })
      .then((nextUnlisten) => {
        if (disposed) {
          nextUnlisten();
        } else {
          unlisten = nextUnlisten;
          openPendingRequests();
        }
      })
      .catch((nextError) => {
        if (disposed) {
          return;
        }
        reportDisplayedError("load pending open requests", nextError, setError);
      });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  function openNewTaskDialog(source: string | null = null) {
    void writeLog("info", "opening new task dialog");
    setView("tasks");
    setNewTaskInitialSource(source);
    setNewTaskInitialFileName(null);
    setNewTaskInitialBrowserCookies(null);
    setNewTaskInitialHttpReferrer(null);
    setShowNewTaskDialog(true);
  }

  async function openNewTaskDialogFromToolbar() {
    try {
      const clipboardText = await readClipboardText();
      const source = clipboardText ? clipboardDownloadSource(clipboardText) : null;
      openNewTaskDialog(source);
    } catch (nextError) {
      reportDisplayedError("read clipboard", nextError, setError);
      openNewTaskDialog();
    }
  }

  function closeNewTaskDialog() {
    setShowNewTaskDialog(false);
    setNewTaskInitialSource(null);
    setNewTaskInitialFileName(null);
    setNewTaskInitialBrowserCookies(null);
    setNewTaskInitialHttpReferrer(null);
  }

  function toggleAllSelected() {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (allVisibleSelected) {
        visibleTasks.forEach((task) => next.delete(task.id));
      } else {
        visibleTasks.forEach((task) => next.add(task.id));
      }
      return next;
    });
  }

  function toggleTaskSelected(taskId: string) {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (next.has(taskId)) {
        next.delete(taskId);
      } else {
        next.add(taskId);
      }
      return next;
    });
  }

  function closeTaskContextMenu() {
    setTaskContextMenu(null);
  }

  function openTaskContextMenu(task: DownloadTask, x: number, y: number) {
    setSelectedIds((current) => {
      if (current.has(task.id)) {
        return current;
      }
      return new Set([task.id]);
    });
    setTaskContextMenu({ taskId: task.id, x, y });
  }

  async function copyTaskText(value: string, label: string) {
    setError(null);
    try {
      await navigator.clipboard.writeText(value);
      closeTaskContextMenu();
    } catch (nextError) {
      setError(
        nextError instanceof Error
          ? `复制${label}失败：${nextError.message}`
          : `复制${label}失败：${String(nextError)}`,
      );
    }
  }

  async function pauseTasksByIds(ids: string[]) {
    if (ids.length === 0) {
      return;
    }

    setError(null);
    try {
      await pauseDownloadTasks(ids);
      closeTaskContextMenu();
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
      await refreshTasks();
    }
  }

  async function resumeTasksByIds(tasksToResume: DownloadTask[]) {
    const ids = tasksToResume
      .filter((task) => task.status === "paused" || task.status === "failed")
      .map((task) => task.id);
    if (ids.length === 0) {
      return;
    }

    const restartTasks = tasksToResume.filter(
      (task) => task.status === "failed" && !isResumableTask(task),
    );

    setError(null);
    try {
      if (restartTasks.length > 0) {
        await message(`${restartTasks.length} 个失败任务不支持续传，将从头开始下载。`, {
          title: "需要重新下载",
          kind: "warning",
        });
      }
      await resumeDownloadTasks(ids);
      closeTaskContextMenu();
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
      await refreshTasks();
    }
  }

  async function deleteTasksByIds(tasksToDelete: DownloadTask[]) {
    const ids = tasksToDelete.map((task) => task.id);
    if (ids.length === 0) {
      return;
    }

    const hasLocalDownloadTasks = tasksToDelete.some((task) =>
      isLocalDownloadEngine(task.engine),
    );
    const deleteCompletedFiles = hasLocalDownloadTasks
      ? await confirmDeleteCompletedFiles()
      : false;

    if (deleteCompletedFiles === null) {
      return;
    }

    setError(null);
    try {
      await deleteDownloadTasks(ids, deleteCompletedFiles);
      setSelectedIds(
        (current) => new Set([...current].filter((id) => !ids.includes(id))),
      );
      closeTaskContextMenu();
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
    }
  }

  async function redownloadTask(task: DownloadTask) {
    const deleteDownloadedFiles = isLocalDownloadEngine(task.engine)
      ? await confirmDeleteCompletedFiles()
      : false;

    if (deleteDownloadedFiles === null) {
      return;
    }

    setError(null);
    try {
      await deleteDownloadTasks([task.id], deleteDownloadedFiles);
      const recreatedTask = await createDownloadTask({
        sourceType: task.sourceType,
        source: task.source,
        engine: task.engine,
        engineSettingsId: task.engineSettingsId,
        fileName: task.fileName,
        savePath: task.savePath,
        engineArgs: task.engineArgs,
        selectedFileIndexes: task.selectedFileIndexes ?? null,
        browserCookies: task.browserCookies ?? null,
        httpReferrer: task.httpReferrer ?? null,
      });
      setSelectedIds(new Set([recreatedTask.id]));
      setDetailTaskId(recreatedTask.id);
      closeTaskContextMenu();
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
      await refreshTasks();
    }
  }

  async function openTaskDownloadDirectory(task: DownloadTask) {
    if (!isLocalDownloadEngine(task.engine)) {
      return;
    }

    setError(null);
    try {
      await openDownloadDirectory(task.id);
      closeTaskContextMenu();
    } catch (nextError) {
      reportDisplayedError("open download directory", nextError, setError);
    }
  }

  async function togglePaused() {
    if (toggleDisabled) {
      return;
    }

    setError(null);
    try {
      const ids = selectedTasks.map((task) => task.id);
      const resumeTasks = ids.length > 0 ? selectedTasks : pausedScopeTasks;
      const resumeIds = resumeTasks
        .filter((task) => task.status === "paused" || task.status === "failed")
        .map((task) => task.id);
      const restartTasks = resumeTasks.filter(
        (task) => task.status === "failed" && !isResumableTask(task),
      );
      if (shouldResume && restartTasks.length > 0) {
        await message(`${restartTasks.length} 个失败任务不支持续传，将从头开始下载。`, {
          title: "需要重新下载",
          kind: "warning",
        });
      }
      if (ids.length > 0) {
        if (shouldResume) {
          await resumeDownloadTasks(resumeIds);
        } else {
          await pauseDownloadTasks(ids);
        }
      } else if (shouldResume) {
        await resumeAllPausedDownloadTasks();
      } else {
        await pauseAllUnfinishedDownloadTasks();
      }
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
      await refreshTasks();
    }
  }

  async function deleteSelectedTasks() {
    if (selectedTasks.length === 0) {
      return;
    }

    await deleteTasksByIds(selectedTasks);
  }

  function openTaskDetails(task: DownloadTask) {
    if (detailTaskId === task.id && selectedIds.has(task.id)) {
      setSelectedIds((current) => {
        const next = new Set(current);
        next.delete(task.id);
        return next;
      });
      setDetailTaskId(null);
      return;
    }

    setSelectedIds(new Set([task.id]));
    setDetailTaskId(task.id);
  }

  async function handleTaskDoubleClick(task: DownloadTask) {
    if (isLocalDownloadEngine(task.engine) === false) {
      return;
    }

    setError(null);
    try {
      await openDownloadedFile(task.id);
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
    }
  }

  async function toggleWindowMaximized() {
    if (!hasTauriRuntime()) {
      return;
    }

    const currentWindow = getCurrentWindow();
    await currentWindow.toggleMaximize();
    setIsWindowMaximized(await currentWindow.isMaximized());
  }

  function handleTaskCreated(task: DownloadTask) {
    setView("tasks");
    setTasks((current) => [task, ...current]);
  }

  const contextMenuActionTasks = useMemo(() => {
    if (!contextMenuTask) {
      return [];
    }

    if (selectedIds.has(contextMenuTask.id) && selectedTasks.length > 0) {
      return selectedTasks;
    }

    return [contextMenuTask];
  }, [contextMenuTask, selectedIds, selectedTasks]);
  const canPauseContextMenuTasks = contextMenuActionTasks.some(
    (task) => !isFinished(task.status) && task.status !== "paused",
  );
  async function submitWebLogin(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setIsWebAuthenticating(true);
    setError(null);
    try {
      await webLogin(webPassword);
      setIsWebAuthorized(true);
      setWebPassword("");
      await loadThemeMode();
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
    } finally {
      setIsWebAuthenticating(false);
    }
  }

  if (isWebRuntime() && !isWebAuthorized) {
    return (
      <div className="flex min-h-[100dvh] items-center justify-center bg-slate-100 px-4">
        <form
          onSubmit={(event) => void submitWebLogin(event)}
          className="w-full max-w-md rounded-2xl border border-slate-200 bg-white p-6 shadow-sm"
        >
          <div className="mb-6 flex items-center gap-3">
            <img src={logoUrl} alt="UniDL" className="h-10 w-10 rounded-xl" />
            <div>
              <div className="text-lg font-semibold text-slate-900">UniDL Web</div>
              <div className="text-sm text-slate-500">输入访问密码后进入完整界面</div>
            </div>
          </div>

          <label
            htmlFor="web-access-password"
            className="mb-2 block text-sm font-medium text-slate-700"
          >
            访问密码
          </label>
          <div className="relative">
            <input
              id="web-access-password"
              type={isWebPasswordVisible ? "text" : "password"}
              title="访问密码"
              value={webPassword}
              onChange={(event) => setWebPassword(event.target.value)}
              className="h-11 w-full rounded-lg border border-slate-300 px-3 pr-11 text-sm outline-none ring-0 transition focus:border-emerald-500"
              autoFocus
            />
            <button
              type="button"
              title={isWebPasswordVisible ? "隐藏密码" : "显示密码"}
              aria-label={isWebPasswordVisible ? "隐藏密码" : "显示密码"}
              onClick={() => setIsWebPasswordVisible((current) => !current)}
              className="absolute inset-y-0 right-0 grid w-11 place-items-center text-slate-500 transition hover:text-slate-700"
            >
              {isWebPasswordVisible ? <EyeOff size={18} /> : <Eye size={18} />}
            </button>
          </div>

          {error && (
            <div className="mt-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-700">
              {error}
            </div>
          )}

          <button
            type="submit"
            disabled={isWebAuthenticating || webPassword.trim().length === 0}
            className="mt-5 inline-flex h-11 w-full items-center justify-center rounded-lg bg-emerald-700 px-4 text-sm font-medium text-white transition hover:bg-emerald-800 disabled:cursor-not-allowed disabled:bg-slate-300"
          >
            {isWebAuthenticating ? "登录中..." : "进入 UniDL"}
          </button>
        </form>
      </div>
    );
  }

  const canResumeContextMenuTasks = contextMenuActionTasks.some(
    (task) => task.status === "paused" || task.status === "failed",
  );

  return (
    <div className="flex h-[100dvh] min-h-0 flex-col bg-surface text-ink">
      <header className="flex h-12 shrink-0 items-center border-b border-slate-200 bg-white px-2 md:px-0">
        <div
          data-tauri-drag-region
          className="flex min-w-0 shrink-0 items-center gap-2 px-1 md:px-4"
        >
          <img src={logoUrl} alt="UniDL" className="h-7 w-7 rounded-md" />
          <div
            data-tauri-drag-region
            className="hidden truncate text-sm font-semibold sm:block"
          >
            UniDL
          </div>
        </div>
        <div className="ml-auto flex min-w-0 items-center justify-end gap-1 md:ml-0 md:gap-2">
          {view === "tasks" ? (
            <>
              <IconButton
                title="新建"
                tone="primary"
                onClick={() => void openNewTaskDialogFromToolbar()}
              >
                <Plus size={18} />
              </IconButton>
              <IconButton
                title={shouldResume ? "开始" : "暂停"}
                disabled={toggleDisabled}
                onClick={() => void togglePaused()}
              >
                {shouldResume ? <Play size={17} /> : <Pause size={17} />}
              </IconButton>
              <IconButton
                title="删除"
                tone="danger"
                disabled={deleteDisabled}
                onClick={() => void deleteSelectedTasks()}
              >
                <Trash2 size={17} />
              </IconButton>
              <IconButton
                title="刷新"
                disabled={isLoading}
                onClick={() => void refreshTasks()}
              >
                <RefreshCw size={17} className={isLoading ? "animate-spin" : ""} />
              </IconButton>
            </>
          ) : (
            <IconButton title="返回" onClick={() => setView("tasks")}>
              <ArrowLeft size={18} />
            </IconButton>
          )}

          <IconButton
            title="设置"
            disabled={view === "settings"}
            onClick={() => setView("settings")}
          >
            <Settings size={17} />
          </IconButton>
          {view === "tasks" &&
            (isSearchOpen ? (
              <div ref={searchContainerRef} className="relative w-24 min-w-0 sm:w-40 md:w-56">
                <input
                  ref={searchInputRef}
                  type="text"
                  value={searchQuery}
                  onChange={(event) => setSearchQuery(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Escape") {
                      if (searchQuery) {
                        setSearchQuery("");
                      } else {
                        setIsSearchOpen(false);
                      }
                    }
                  }}
                  placeholder="搜索任务"
                  aria-label="搜索任务"
                  className="h-9 w-full rounded-md border border-slate-200 bg-white px-3 pr-9 text-sm text-slate-800 outline-none transition placeholder:text-slate-400 focus:border-emerald-500 focus:ring-2 focus:ring-emerald-100"
                />
                {searchQuery.length > 0 && (
                  <button
                    type="button"
                    title="清空搜索"
                    aria-label="清空搜索"
                    onClick={() => {
                      setSearchQuery("");
                      searchInputRef.current?.focus();
                    }}
                    className="absolute inset-y-0 right-0 grid w-9 place-items-center text-slate-400 transition hover:text-slate-700"
                  >
                    <X size={15} />
                  </button>
                )}
              </div>
            ) : (
              <IconButton title="搜索" onClick={() => setIsSearchOpen(true)}>
                <Search size={17} />
              </IconButton>
            ))}
        </div>
        <div
          data-tauri-drag-region
          className="hidden h-full min-w-0 flex-1 md:block"
        />
        {hasTauriRuntime() && (
          <div className="-mr-2 flex h-full items-center md:mr-0">
            <button
              type="button"
              title="最小化"
              aria-label="最小化"
              onClick={() => void getCurrentWindow().minimize()}
              className="grid h-12 w-12 place-items-center text-slate-600 hover:bg-slate-100"
            >
              <Minus size={16} />
            </button>
            <button
              type="button"
              title={isWindowMaximized ? "还原" : "最大化"}
              aria-label={isWindowMaximized ? "还原" : "最大化"}
              onClick={() => void toggleWindowMaximized()}
              className="grid h-12 w-12 place-items-center text-slate-600 hover:bg-slate-100"
            >
              {isWindowMaximized ? (
                <Copy size={15} className="-scale-x-100" />
              ) : (
                <Square size={14} />
              )}
            </button>
            <button
              type="button"
              title="关闭"
              aria-label="关闭"
              onClick={() => void getCurrentWindow().close()}
              className="grid h-12 w-12 place-items-center text-slate-600 hover:bg-rose-600 hover:text-white"
            >
              <X size={17} />
            </button>
          </div>
        )}
      </header>

      <main className="flex min-h-0 flex-1 flex-col">
        {view === "tasks" && error && (
          <div className="border-b border-rose-200 bg-rose-50 px-4 py-2 text-sm text-rose-700">
            {error}
          </div>
        )}

        {view === "settings" ? (
          <EngineSettingsView
            onDownloadRecordsCleared={replaceTasks}
            onThemeModeChange={setThemeMode}
          />
        ) : (
          <section className="min-h-0 flex-1 overflow-auto">
            <div className="grid gap-3 p-3 md:hidden">
              {visibleTasks.map((task) => (
                <MobileTaskCard
                  key={task.id}
                  task={task}
                  isSelected={selectedIds.has(task.id)}
                  isActive={detailTaskId === task.id}
                  onOpen={() => openTaskDetails(task)}
                  onDoubleClick={() => {
                    void handleTaskDoubleClick(task);
                  }}
                  onContextMenu={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    openTaskContextMenu(task, event.clientX, event.clientY);
                  }}
                  onToggleSelected={() => toggleTaskSelected(task.id)}
                />
              ))}
            </div>
            <table
              ref={taskTableRef}
              className="hidden w-full table-fixed border-separate border-spacing-0 text-sm md:table"
            >
              <colgroup>
                {taskTableColumns.map((column) => (
                  <col key={column.key} width={taskColumnWidths[column.key]} />
                ))}
              </colgroup>
              <thead className="sticky top-0 z-10 bg-slate-100 text-xs font-semibold uppercase tracking-normal text-slate-600">
                <tr>
                  {taskTableColumns.map((column) => (
                    <th
                      key={column.key}
                      className={classNames(
                        "relative border-b border-slate-200 py-3",
                        column.key === "selected" ? "px-4" : "px-3",
                        centeredTaskColumnKeys.has(column.key)
                          ? "text-center"
                          : "text-left",
                      )}
                    >
                      {column.key === "selected" ? (
                        <button
                          type="button"
                          title="选择全部"
                          aria-label="选择全部"
                          onClick={toggleAllSelected}
                          className="grid h-5 w-5 place-items-center rounded border border-slate-300 bg-white text-emerald-700"
                        >
                          {allVisibleSelected && <Check size={14} strokeWidth={3} />}
                        </button>
                      ) : (
                        column.label
                      )}
                      {column.resizable && (
                        <button
                          type="button"
                          title={`调整${column.label}列宽`}
                          aria-label={`调整${column.label}列宽`}
                          onPointerDown={(event) =>
                            handleColumnResizeStart(event, column)
                          }
                          className="absolute inset-y-0 right-0 w-2 cursor-col-resize touch-none rounded-sm hover:bg-emerald-500/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-emerald-500"
                        />
                      )}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {visibleTasks.map((task) => {
                  const isSelected = selectedIds.has(task.id);
                  const isDownloadedFileMissing = task.downloadedFileMissing;

                  return (
                    <tr
                      key={task.id}
                      onClick={() => openTaskDetails(task)}
                      onDoubleClick={() => handleTaskDoubleClick(task)}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        openTaskContextMenu(task, event.clientX, event.clientY);
                      }}
                      className={classNames(
                        "cursor-pointer bg-white hover:bg-slate-50",
                        isDownloadedFileMissing && "text-slate-400 opacity-70",
                        detailTaskId === task.id && "bg-sky-50 hover:bg-sky-50",
                        isSelected && "bg-emerald-50 hover:bg-emerald-50",
                      )}
                    >
                      <td className="border-b border-slate-100 px-4 py-3">
                        <button
                          type="button"
                          title="选择任务"
                          aria-label="选择任务"
                          onClick={(event) => {
                            event.stopPropagation();
                            toggleTaskSelected(task.id);
                          }}
                          className={classNames(
                            "grid h-5 w-5 place-items-center rounded border bg-white",
                            isSelected
                              ? "border-emerald-700 text-emerald-700"
                              : "border-slate-300 text-transparent",
                          )}
                        >
                          <Check size={14} strokeWidth={3} />
                        </button>
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3">
                        <div
                          className={classNames(
                            "truncate font-medium text-slate-900",
                            isDownloadedFileMissing && "text-slate-500 line-through",
                          )}
                          title={
                            isDownloadedFileMissing
                              ? `${task.fileName}（文件已不存在）`
                              : task.fileName
                          }
                        >
                          {task.fileName}
                        </div>
                        <div className="mt-1 text-xs text-slate-500">
                          {sourceLabels[task.sourceType]}
                        </div>
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 text-center text-slate-700">
                        {engineLabels[task.engine]}
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 text-center">
                        <StatusBadge status={task.status} />
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3">
                        <div className="space-y-1.5">
                          <div className="text-center text-xs tabular-nums text-slate-600">
                            {task.progress.toFixed(1)}%
                          </div>
                          <progress
                            className="task-progress block"
                            value={Math.min(100, Math.max(0, task.progress))}
                            max={100}
                            aria-label={`${task.fileName} 下载进度`}
                          />
                        </div>
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 text-center tabular-nums text-slate-700">
                        {formatBytes(task.downloadedBytes)} /{" "}
                        {formatBytes(task.totalBytes)}
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 text-center tabular-nums text-slate-700">
                        {formatSpeed(task.speedBytesPerSec)}
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3">
                        <div className="truncate text-slate-700" title={task.savePath}>
                          {task.savePath}
                        </div>
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 tabular-nums text-slate-600">
                        {formatDate(task.createdAt)}
                      </td>
                      <td className="border-b border-slate-100 px-3 py-3 tabular-nums text-slate-600">
                        {formatDate(task.completedAt)}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>

            {!isLoading && visibleTasks.length === 0 && (
              <div className="grid h-[calc(100vh-180px)] min-h-[320px] place-items-center text-sm text-slate-500">
                {tasks.length === 0 ? "暂无任务" : "没有匹配的任务"}
              </div>
            )}
          </section>
        )}

        {view === "tasks" && detailTask && (
          <TaskDetailPanel task={detailTask} onClose={() => setDetailTaskId(null)} />
        )}
      </main>

      {taskContextMenu && contextMenuTask && (
        <div
          className="fixed inset-0 z-30"
          onContextMenu={(event) => event.preventDefault()}
        >
          <div
            ref={taskContextMenuPanelRef}
            className="fixed z-50 min-w-44 rounded-lg border border-slate-200 bg-white py-1 shadow-xl"
            onPointerDown={(event) => event.stopPropagation()}
          >
            <button
              type="button"
              disabled={!canPauseContextMenuTasks}
              onClick={() =>
                void pauseTasksByIds(
                  contextMenuActionTasks
                    .filter(
                      (task) => !isFinished(task.status) && task.status !== "paused",
                    )
                    .map((task) => task.id),
                )
              }
              className={classNames(
                "flex w-full items-center gap-2 px-3 py-2 text-left text-sm",
                canPauseContextMenuTasks
                  ? "text-slate-700 hover:bg-slate-50"
                  : "cursor-not-allowed text-slate-400",
              )}
            >
              <Pause size={15} />
              暂停下载
            </button>
            <button
              type="button"
              disabled={!canResumeContextMenuTasks}
              onClick={() => void resumeTasksByIds(contextMenuActionTasks)}
              className={classNames(
                "flex w-full items-center gap-2 px-3 py-2 text-left text-sm",
                canResumeContextMenuTasks
                  ? "text-slate-700 hover:bg-slate-50"
                  : "cursor-not-allowed text-slate-400",
              )}
            >
              <Play size={15} />
              恢复下载
            </button>
            <button
              type="button"
              onClick={() => void redownloadTask(contextMenuTask)}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-slate-700 hover:bg-slate-50"
            >
              <RefreshCw size={15} />
              重新下载
            </button>
            <button
              type="button"
              onClick={() => void deleteTasksByIds(contextMenuActionTasks)}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-rose-700 hover:bg-rose-50"
            >
              <Trash2 size={15} />
              删除任务
            </button>
            <div className="my-1 border-t border-slate-100" />
            {isLocalDownloadEngine(contextMenuTask.engine) && (
              <button
                type="button"
                onClick={() => void openTaskDownloadDirectory(contextMenuTask)}
                className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-slate-700 hover:bg-slate-50"
              >
                <FolderOpen size={15} />
                打开下载目录
              </button>
            )}
            <button
              type="button"
              onClick={() => void copyTaskText(contextMenuTask.source, "下载链接")}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-slate-700 hover:bg-slate-50"
            >
              <Copy size={15} />
              复制下载链接
            </button>
            <button
              type="button"
              onClick={() => void copyTaskText(contextMenuTask.savePath, "保存路径")}
              className="flex w-full items-center gap-2 px-3 py-2 text-left text-sm text-slate-700 hover:bg-slate-50"
            >
              <Copy size={15} />
              复制保存路径
            </button>
          </div>
        </div>
      )}

      {showDeleteDialog && (
        <div className="fixed inset-0 z-40 grid place-items-center bg-slate-950/30 px-4">
          <div className="w-full max-w-md rounded-lg bg-white shadow-xl">
            <div className="border-b border-slate-200 px-4 py-3">
              <h2 className="text-base font-semibold text-slate-900">删除任务</h2>
            </div>
            <div className="space-y-3 px-4 py-4 text-sm text-slate-700">
              <p>所选任务包含本地下载文件，是否同时删除已下载文件/文件夹？</p>
            </div>
            <footer className="grid gap-2 border-t border-slate-200 px-4 py-3 sm:flex sm:flex-wrap sm:justify-end">
              <button
                type="button"
                onClick={() => resolveDeleteDialog(true)}
                className="h-9 rounded-md border border-rose-200 bg-white px-4 text-sm font-medium text-rose-700 hover:bg-rose-50"
              >
                删除文件
              </button>
              <button
                type="button"
                onClick={() => resolveDeleteDialog(false)}
                className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50"
              >
                保留文件
              </button>
              <button
                type="button"
                onClick={() => resolveDeleteDialog(null)}
                className="h-9 rounded-md border border-slate-200 px-4 text-sm font-medium text-slate-700 hover:bg-slate-50"
              >
                取消删除
              </button>
            </footer>
          </div>
        </div>
      )}

      <NewTaskDialog
        open={showNewTaskDialog}
        initialSource={newTaskInitialSource}
        initialFileName={newTaskInitialFileName}
        initialBrowserCookies={newTaskInitialBrowserCookies}
        initialHttpReferrer={newTaskInitialHttpReferrer}
        onClose={closeNewTaskDialog}
        onCreated={handleTaskCreated}
      />
    </div>
  );
}

export default App;
