import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode } from "react";
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
  openDownloadDirectory,
  openDownloadedFile,
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
import type { DownloadStatus, DownloadTask, EngineKind, SourceType } from "@shared/types";

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

function App() {
  const [view, setView] = useState<"tasks" | "settings">("tasks");
  const [showNewTaskDialog, setShowNewTaskDialog] = useState(false);
  const [newTaskInitialSource, setNewTaskInitialSource] = useState<string | null>(null);
  const [newTaskInitialFileName, setNewTaskInitialFileName] = useState<string | null>(
    null,
  );
  const [newTaskInitialBrowserCookies, setNewTaskInitialBrowserCookies] = useState<
    string | null
  >(null);
  const [showDeleteDialog, setShowDeleteDialog] = useState(false);
  const [tasks, setTasks] = useState<DownloadTask[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
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
  const deleteDialogResolveRef = useRef<((value: boolean | null) => void) | null>(null);

  const selectedTasks = useMemo(
    () => tasks.filter((task) => selectedIds.has(task.id)),
    [selectedIds, tasks],
  );
  const detailTask = useMemo(
    () => tasks.find((task) => task.id === detailTaskId) ?? null,
    [detailTaskId, tasks],
  );
  const contextMenuTask = useMemo(
    () => tasks.find((task) => task.id === taskContextMenu?.taskId) ?? null,
    [taskContextMenu, tasks],
  );
  const allVisibleSelected = tasks.length > 0 && selectedIds.size === tasks.length;
  const selectedScopeTasks = selectedTasks.length > 0 ? selectedTasks : tasks;
  const activeScopeTasks = selectedScopeTasks.filter((task) => !isFinished(task.status));
  const failedScopeTasks = selectedScopeTasks.filter((task) => task.status === "failed");
  const pausedScopeTasks = activeScopeTasks.filter((task) => task.status === "paused");
  const shouldResume =
    failedScopeTasks.length > 0 ||
    (activeScopeTasks.length > 0 && pausedScopeTasks.length === activeScopeTasks.length);
  const toggleDisabled = activeScopeTasks.length === 0 && failedScopeTasks.length === 0;
  const deleteDisabled = selectedIds.size === 0;
  const totalTaskTableWidth = taskTableColumns.reduce(
    (sum, column) => sum + taskColumnWidths[column.key],
    0,
  );

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
    setShowNewTaskDialog(true);
  }

  function closeNewTaskDialog() {
    setShowNewTaskDialog(false);
    setNewTaskInitialSource(null);
    setNewTaskInitialFileName(null);
    setNewTaskInitialBrowserCookies(null);
  }

  function toggleAllSelected() {
    setSelectedIds(
      allVisibleSelected ? new Set() : new Set(tasks.map((task) => task.id)),
    );
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
    setDetailTaskId(task.id);
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
      const ids = [...selectedIds];
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
      await refreshTasks();
    } catch (nextError) {
      reportDisplayedError("load pending open requests", nextError, setError);
    } finally {
      setIsWebAuthenticating(false);
    }
  }

  if (isWebRuntime() && !isWebAuthorized) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-slate-100 px-4">
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

          <label className="mb-2 block text-sm font-medium text-slate-700">
            访问密码
          </label>
          <div className="relative">
            <input
              type={isWebPasswordVisible ? "text" : "password"}
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
    <div className="flex h-screen min-h-[620px] flex-col bg-surface text-ink">
      <header className="flex h-12 shrink-0 items-center border-b border-slate-200 bg-white">
        <div data-tauri-drag-region className="flex min-w-0 items-center gap-2 px-4">
          <img src={logoUrl} alt="UniDL" className="h-7 w-7 rounded-md" />
          <div data-tauri-drag-region className="truncate text-sm font-semibold">
            UniDL
          </div>
        </div>
        <div className="flex items-center gap-2">
          {view === "tasks" ? (
            <>
              <IconButton title="新建" tone="primary" onClick={() => openNewTaskDialog()}>
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
        </div>
        <div data-tauri-drag-region className="h-full min-w-0 flex-1" />
        {hasTauriRuntime() && (
          <div className="flex h-full items-center">
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
          <EngineSettingsView />
        ) : (
          <section className="min-h-0 flex-1 overflow-auto">
            <table
              className="table-fixed border-separate border-spacing-0 text-sm"
              style={{ width: "100%", minWidth: `${totalTaskTableWidth}px` }}
            >
              <colgroup>
                {taskTableColumns.map((column) => (
                  <col
                    key={column.key}
                    style={{ width: `${taskColumnWidths[column.key]}px` }}
                  />
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
                {tasks.map((task) => {
                  const isSelected = selectedIds.has(task.id);

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
                          className="truncate font-medium text-slate-900"
                          title={task.fileName}
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
                          <div className="h-2 overflow-hidden rounded-full bg-slate-200">
                            <div
                              className="h-full rounded-full bg-emerald-700"
                              style={{
                                width: `${Math.min(100, Math.max(0, task.progress))}%`,
                              }}
                            />
                          </div>
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

            {!isLoading && tasks.length === 0 && (
              <div className="grid h-[calc(100vh-180px)] min-h-[320px] place-items-center text-sm text-slate-500">
                暂无任务
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
            className="fixed z-50 min-w-44 rounded-lg border border-slate-200 bg-white py-1 shadow-xl"
            style={{ left: taskContextMenu.x, top: taskContextMenu.y }}
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
            <footer className="flex flex-wrap justify-end gap-2 border-t border-slate-200 px-4 py-3">
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
        onClose={closeNewTaskDialog}
        onCreated={handleTaskCreated}
      />
    </div>
  );
}

export default App;
