import { useEffect, useMemo, useState } from "react";
import type { DragEvent as ReactDragEvent, ReactNode } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  Check,
  ChevronDown,
  Download,
  Globe2,
  GripVertical,
  HardDrive,
  Plus,
  RotateCcw,
  Save,
  Trash2,
} from "lucide-react";

import {
  deleteEngineSettings,
  getAppSettings,
  getSystemDownloadDir,
  installLatestEngine,
  listEngineSettings,
  saveAppSettings,
  saveEngineSettings,
} from "@/lib/api";
import type {
  AppSettings,
  AppSettingsInput,
  EngineKind,
  EngineSettings,
  EngineSettingsInput,
  SourceType,
} from "@shared/types";

const engineOrder: EngineKind[] = ["aria2", "yt-dlp", "qbittorrent"];
const sourceTypes: SourceType[] = ["http", "ftp", "magnet", "torrent"];
const engineInstallDir = "%AppData%\\UniDL\\engines";
type SettingsGroup = "web-access" | "download-engines";

const engineLabels: Record<EngineKind, string> = {
  aria2: "aria2",
  "yt-dlp": "yt-dlp",
  qbittorrent: "qBittorrent",
};

const sourceLabels: Record<SourceType, string> = {
  http: "HTTP",
  ftp: "FTP",
  magnet: "Magnet",
  torrent: "Torrent",
};

function classNames(...names: Array<string | false | null | undefined>) {
  return names.filter(Boolean).join(" ");
}

function supportedSourceTypes(engine: EngineKind): SourceType[] {
  switch (engine) {
    case "aria2":
      return ["http", "ftp", "magnet", "torrent"];
    case "yt-dlp":
      return ["http", "ftp"];
    case "qbittorrent":
      return ["magnet", "torrent"];
  }
}

function isLocalDownloadEngine(engine: EngineKind) {
  return engine === "aria2" || engine === "yt-dlp";
}

function defaultEngineSettings(
  engine: EngineKind,
  defaultDownloadDir: string,
): EngineSettings {
  return {
    id: crypto.randomUUID(),
    engine,
    name: engineLabels[engine],
    enabled: false,
    executablePath:
      engine === "aria2"
        ? `${engineInstallDir}\\aria2c.exe`
        : engine === "yt-dlp"
          ? `${engineInstallDir}\\yt-dlp.exe`
          : null,
    defaultDownloadDir,
    defaultArgs:
      engine === "aria2" ? "--continue=true" : engine === "yt-dlp" ? "--newline" : "",
    connectionUrl:
      engine === "aria2"
        ? "http://127.0.0.1:6800/jsonrpc"
        : engine === "qbittorrent"
          ? "http://127.0.0.1:8080"
          : null,
    username: null,
    password: null,
    remotePath: engine === "qbittorrent" ? "" : null,
    supportedSourceTypes: supportedSourceTypes(engine),
    priority: 0,
    updatedAt: "",
  };
}

function sortSettings(settings: EngineSettings[]) {
  return [...settings].sort((left, right) => {
    if (left.priority !== right.priority) {
      return left.priority - right.priority;
    }

    const leftKey = `${left.updatedAt || ""}${left.id}`;
    const rightKey = `${right.updatedAt || ""}${right.id}`;
    return leftKey.localeCompare(rightKey);
  });
}

function assignPriorities(settings: EngineSettings[]) {
  return settings.map((item, index) => ({ ...item, priority: index }));
}

function emptyToNull(value: string) {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function toInput(settings: EngineSettings): EngineSettingsInput {
  return {
    id: settings.id,
    engine: settings.engine,
    name: settings.name.trim(),
    enabled: settings.enabled,
    executablePath: emptyToNull(settings.executablePath ?? ""),
    defaultDownloadDir: settings.defaultDownloadDir,
    defaultArgs: settings.defaultArgs,
    connectionUrl: emptyToNull(settings.connectionUrl ?? ""),
    username: emptyToNull(settings.username ?? ""),
    password: emptyToNull(settings.password ?? ""),
    remotePath: emptyToNull(settings.remotePath ?? ""),
    supportedSourceTypes: settings.supportedSourceTypes,
    priority: settings.priority,
  };
}

function isDirty(saved: EngineSettings, draft: EngineSettings) {
  return JSON.stringify(toInput(saved)) !== JSON.stringify(toInput(draft));
}

function toAppInput(settings: AppSettings): AppSettingsInput {
  return {
    webAccessEnabled: settings.webAccessEnabled,
    webAccessPassword: settings.webAccessPassword,
  };
}

function isAppDirty(saved: AppSettings, draft: AppSettings) {
  return JSON.stringify(toAppInput(saved)) !== JSON.stringify(toAppInput(draft));
}

function Field({
  label,
  value,
  type = "text",
  onChange,
}: {
  label: string;
  value: string;
  type?: "text" | "password";
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <input
        type={type}
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        className="h-9 rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
      />
    </label>
  );
}

function TextAreaField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <textarea
        value={value}
        onChange={(event) => onChange(event.currentTarget.value)}
        rows={3}
        className="min-h-20 resize-y rounded-md border border-slate-200 bg-white px-3 py-2 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
      />
    </label>
  );
}

function IconField({
  label,
  value,
  onChange,
  onClick,
  buttonTitle,
  buttonDisabled,
  buttonLabel,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  onClick: () => void;
  buttonTitle: string;
  buttonDisabled?: boolean;
  buttonLabel: ReactNode;
}) {
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <div className="flex min-w-0 items-center gap-2">
        <input
          value={value}
          onChange={(event) => onChange(event.currentTarget.value)}
          className="h-9 min-w-0 flex-1 rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
        />
        <button
          type="button"
          title={buttonTitle}
          aria-label={buttonTitle}
          disabled={buttonDisabled}
          onClick={onClick}
          className={classNames(
            "grid h-9 w-9 shrink-0 place-items-center rounded-md border transition",
            buttonDisabled &&
              "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
            !buttonDisabled &&
              "border-slate-200 bg-white text-slate-700 hover:border-slate-300 hover:bg-slate-50",
          )}
        >
          {buttonLabel}
        </button>
      </div>
    </label>
  );
}

function SmallIconButton({
  title,
  disabled,
  onClick,
  children,
}: {
  title: string;
  disabled?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      aria-label={title}
      disabled={disabled}
      onClick={onClick}
      className={classNames(
        "grid h-8 w-8 place-items-center rounded-md border transition",
        disabled && "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
        !disabled &&
          "border-slate-200 bg-white text-slate-700 hover:border-slate-300 hover:bg-slate-50",
      )}
    >
      {children}
    </button>
  );
}

function InstalledBadge({ label }: { label: string }) {
  return (
    <span className="inline-flex h-8 items-center gap-1 rounded-md border border-emerald-200 bg-emerald-50 px-2 text-xs text-emerald-800">
      <Check size={13} />
      {label}
    </span>
  );
}

export default function EngineSettingsView() {
  const [activeGroup, setActiveGroup] = useState<SettingsGroup>("web-access");
  const [savedAppSettings, setSavedAppSettings] = useState<AppSettings | null>(null);
  const [draftAppSettings, setDraftAppSettings] = useState<AppSettings | null>(null);
  const [savedSettings, setSavedSettings] = useState<EngineSettings[]>([]);
  const [draftSettings, setDraftSettings] = useState<EngineSettings[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isAddMenuOpen, setIsAddMenuOpen] = useState(false);
  const [isSavingApp, setIsSavingApp] = useState(false);
  const [isSavingEngines, setIsSavingEngines] = useState(false);
  const [savingEngineId, setSavingEngineId] = useState<string | null>(null);
  const [deletingEngineId, setDeletingEngineId] = useState<string | null>(null);
  const [installingEngineId, setInstallingEngineId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedApp, setSavedApp] = useState(false);
  const [savedEngineId, setSavedEngineId] = useState<string | null>(null);
  const [draggedEngineId, setDraggedEngineId] = useState<string | null>(null);

  const ready = Boolean(savedAppSettings && draftAppSettings);
  const appDirty =
    savedAppSettings && draftAppSettings
      ? isAppDirty(savedAppSettings, draftAppSettings)
      : false;

  const savedById = useMemo(
    () => new Map(savedSettings.map((item) => [item.id, item])),
    [savedSettings],
  );

  const dirtySettings = useMemo(
    () =>
      draftSettings.some((draft) => {
        const saved = savedById.get(draft.id);
        return !saved || isDirty(saved, draft);
      }),
    [draftSettings, savedById],
  );

  const hasDirtySettings = appDirty || dirtySettings;
  const engineSettingsCount = draftSettings.length;

  useEffect(() => {
    async function loadSettings() {
      setIsLoading(true);
      setError(null);

      try {
        const [settings, appSettings] = await Promise.all([
          listEngineSettings(),
          getAppSettings(),
        ]);
        const nextSettings = assignPriorities(sortSettings(settings));
        setSavedSettings(nextSettings);
        setDraftSettings(nextSettings.map((item) => ({ ...item })));
        setSavedAppSettings(appSettings);
        setDraftAppSettings(appSettings);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        setIsLoading(false);
      }
    }

    void loadSettings();
  }, []);

  function updateAppDraft(patch: Partial<AppSettings>) {
    setSavedApp(false);
    setDraftAppSettings((current) => (current ? { ...current, ...patch } : current));
  }

  async function saveAppAccess() {
    if (!draftAppSettings) {
      return;
    }

    setIsSavingApp(true);
    setError(null);
    setSavedApp(false);

    try {
      const next = await saveAppSettings(toAppInput(draftAppSettings));
      setSavedAppSettings(next);
      setDraftAppSettings(next);
      setSavedApp(true);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setIsSavingApp(false);
    }
  }

  function resetAppAccess() {
    if (!savedAppSettings) {
      return;
    }

    setDraftAppSettings(savedAppSettings);
  }

  function updateDraft(settingsId: string, patch: Partial<EngineSettings>) {
    setDraftSettings((current) =>
      current.map((item) => (item.id === settingsId ? { ...item, ...patch } : item)),
    );
  }

  async function addEngineSettings(engine: EngineKind) {
    setError(null);

    try {
      let defaultDownloadDir = "";
      if (isLocalDownloadEngine(engine)) {
        defaultDownloadDir = await getSystemDownloadDir();
      }

      setSavedEngineId(null);
      setIsAddMenuOpen(false);
      setDraftSettings((current) => {
        const next = {
          ...defaultEngineSettings(engine, defaultDownloadDir),
          priority: current.length,
        };
        return assignPriorities([...sortSettings(current), next]);
      });
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    }
  }

  function reorderEngineSettings(sourceId: string, targetId: string) {
    if (sourceId === targetId) {
      return;
    }

    setSavedEngineId(null);
    setDraftSettings((current) => {
      const ordered = sortSettings(current);
      const sourceIndex = ordered.findIndex((item) => item.id === sourceId);
      const targetIndex = ordered.findIndex((item) => item.id === targetId);
      if (sourceIndex === -1 || targetIndex === -1) {
        return current;
      }

      const next = [...ordered];
      const [source] = next.splice(sourceIndex, 1);
      next.splice(targetIndex, 0, source);
      return assignPriorities(next);
    });
  }

  function handleDragStart(event: ReactDragEvent<HTMLDivElement>, settingsId: string) {
    event.dataTransfer.effectAllowed = "move";
    event.dataTransfer.setData("text/plain", settingsId);
    setDraggedEngineId(settingsId);
  }

  function handleDragOver(event: ReactDragEvent<HTMLDivElement>) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
  }

  function handleDrop(event: ReactDragEvent<HTMLDivElement>, targetId: string) {
    event.preventDefault();
    const sourceId = event.dataTransfer.getData("text/plain") || draggedEngineId;
    if (sourceId) {
      reorderEngineSettings(sourceId, targetId);
    }
    setDraggedEngineId(null);
  }

  async function deleteEngine(settingsId: string) {
    const draft = draftSettings.find((item) => item.id === settingsId);
    if (!draft) {
      return;
    }

    const confirmed = await confirm(`删除下载引擎“${draft.name}”？`, {
      title: "删除下载引擎",
      kind: "warning",
      okLabel: "删除",
      cancelLabel: "取消",
    });
    if (!confirmed) {
      return;
    }

    setError(null);
    setSavedEngineId(null);

    if (!savedById.has(settingsId)) {
      setDraftSettings((current) => current.filter((item) => item.id !== settingsId));
      return;
    }

    setDeletingEngineId(settingsId);
    try {
      await deleteEngineSettings(settingsId);
      setSavedSettings((current) => current.filter((item) => item.id !== settingsId));
      setDraftSettings((current) => current.filter((item) => item.id !== settingsId));
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setDeletingEngineId(null);
    }
  }

  async function saveEngine(settingsId: string) {
    const draft = draftSettings.find((item) => item.id === settingsId);
    if (!draft) {
      return;
    }

    setSavingEngineId(settingsId);
    setError(null);
    setSavedEngineId(null);

    try {
      const next = await saveEngineSettings(toInput(draft));
      setSavedSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setDraftSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setSavedEngineId(next.id);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setSavingEngineId(null);
    }
  }

  async function saveAllEngines() {
    setIsSavingEngines(true);
    setError(null);
    setSavedEngineId(null);

    try {
      const saved: EngineSettings[] = [];
      for (const draft of sortSettings(draftSettings)) {
        const current = savedById.get(draft.id);
        if (!current || isDirty(current, draft)) {
          saved.push(await saveEngineSettings(toInput(draft)));
        } else {
          saved.push(draft);
        }
      }

      const nextSettings = assignPriorities(sortSettings(saved));
      setSavedSettings(nextSettings);
      setDraftSettings(nextSettings.map((item) => ({ ...item })));
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setIsSavingEngines(false);
    }
  }

  function resetEngine(settingsId: string) {
    const saved = savedById.get(settingsId);
    if (!saved) {
      setDraftSettings((current) => current.filter((item) => item.id !== settingsId));
      return;
    }

    setDraftSettings((current) =>
      sortSettings(
        current.map((item) => (item.id === settingsId ? { ...saved } : item)),
      ),
    );
  }

  async function installLatest(settingsId: string) {
    setInstallingEngineId(settingsId);
    setError(null);

    try {
      const next = await installLatestEngine(settingsId);
      setSavedSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.settings.id), next.settings]),
      );
      setDraftSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.settings.id), next.settings]),
      );
      setSavedEngineId(next.settings.id);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setInstallingEngineId(null);
    }
  }

  return (
    <section className="min-h-0 flex-1 overflow-auto bg-surface">
      <div className="mx-auto flex max-w-6xl flex-col gap-4 px-4 py-4">
        <div className="flex min-h-10 items-center justify-between gap-3">
          <div>
            <h1 className="text-base font-semibold text-slate-950">设置</h1>
            <div className="mt-1 text-xs text-slate-500">
              {hasDirtySettings ? "有未保存更改" : "配置已同步"}
            </div>
          </div>
        </div>

        {error && (
          <div className="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-sm text-rose-700">
            {error}
          </div>
        )}

        {isLoading && (
          <div className="grid min-h-[320px] place-items-center text-sm text-slate-500">
            加载中
          </div>
        )}

        {!isLoading && ready && (
          <div className="grid min-h-0 gap-4 lg:grid-cols-[220px_1fr]">
            <aside className="min-w-0">
              <nav className="flex gap-2 overflow-x-auto rounded-lg border border-slate-200 bg-white p-2 shadow-sm lg:flex-col lg:overflow-visible">
                <button
                  type="button"
                  onClick={() => setActiveGroup("web-access")}
                  className={classNames(
                    "flex min-w-44 items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition lg:min-w-0",
                    activeGroup === "web-access"
                      ? "bg-emerald-50 text-emerald-800"
                      : "text-slate-700 hover:bg-slate-50",
                  )}
                >
                  <Globe2 size={16} />
                  <span className="min-w-0 flex-1 truncate">Web 访问</span>
                  <span className="text-xs text-slate-500">
                    {draftAppSettings?.webAccessEnabled ? "启用" : "关闭"}
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => setActiveGroup("download-engines")}
                  className={classNames(
                    "flex min-w-44 items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition lg:min-w-0",
                    activeGroup === "download-engines"
                      ? "bg-emerald-50 text-emerald-800"
                      : "text-slate-700 hover:bg-slate-50",
                  )}
                >
                  <HardDrive size={16} />
                  <span className="min-w-0 flex-1 truncate">下载引擎</span>
                  <span className="text-xs text-slate-500">{engineSettingsCount}</span>
                </button>
              </nav>
            </aside>

            <div className="min-w-0">
              {activeGroup === "web-access" && draftAppSettings && (
                <article className="rounded-lg border border-slate-200 bg-white shadow-sm">
                  <div className="flex items-center justify-between gap-3 border-b border-slate-100 px-4 py-3">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        Web 访问
                      </h2>
                      <div className="mt-1 text-xs text-slate-500">
                        {draftAppSettings.webAccessEnabled
                          ? draftAppSettings.webAccessUrl
                          : "未启用"}
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      {savedApp && <InstalledBadge label="已保存" />}
                      <SmallIconButton
                        title="撤销"
                        disabled={!appDirty || isSavingApp}
                        onClick={resetAppAccess}
                      >
                        <RotateCcw size={15} />
                      </SmallIconButton>
                      <SmallIconButton
                        title="保存"
                        disabled={!appDirty || isSavingApp}
                        onClick={() => void saveAppAccess()}
                      >
                        <Save size={15} />
                      </SmallIconButton>
                    </div>
                  </div>

                  <div className="grid gap-4 px-4 py-4 lg:grid-cols-[220px_1fr]">
                    <div className="flex flex-col gap-3">
                      <label className="flex items-center justify-between gap-3 rounded-md border border-slate-200 px-3 py-2 text-sm text-slate-700">
                        <span className="font-medium">启用</span>
                        <input
                          type="checkbox"
                          checked={draftAppSettings.webAccessEnabled}
                          onChange={(event) =>
                            updateAppDraft({ webAccessEnabled: event.currentTarget.checked })
                          }
                          className="h-4 w-4 accent-emerald-700"
                        />
                      </label>
                    </div>

                    <div className="grid gap-4 md:grid-cols-2">
                      <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                        <span className="font-medium">访问地址</span>
                        <input
                          value={draftAppSettings.webAccessUrl}
                          readOnly
                          className="h-9 rounded-md border border-slate-200 bg-slate-50 px-3 text-sm text-slate-700 outline-none"
                        />
                      </label>
                      <Field
                        label="访问密码"
                        type="password"
                        value={draftAppSettings.webAccessPassword}
                        onChange={(value) => updateAppDraft({ webAccessPassword: value })}
                      />
                    </div>
                  </div>
                </article>
              )}

              {activeGroup === "download-engines" && (
                <div className="flex flex-col gap-4">
                  <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-200 bg-white px-4 py-3 shadow-sm">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        下载引擎
                      </h2>
                      <div className="mt-1 text-xs text-slate-500">
                        {engineSettingsCount} 个配置
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!dirtySettings || isSavingEngines}
                        onClick={() => void saveAllEngines()}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                          (!dirtySettings || isSavingEngines) &&
                            "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                          dirtySettings &&
                            !isSavingEngines &&
                            "border-slate-200 bg-white text-slate-800 hover:border-slate-300 hover:bg-slate-50",
                        )}
                      >
                        <Save size={15} />
                        <span>保存全部</span>
                      </button>

                      <div className="relative">
                      <button
                        type="button"
                        title="添加下载引擎"
                        aria-label="添加下载引擎"
                        onClick={() => setIsAddMenuOpen((current) => !current)}
                        className="inline-flex h-9 items-center gap-2 rounded-md border border-slate-200 bg-white px-3 text-sm font-medium text-slate-800 transition hover:border-slate-300 hover:bg-slate-50"
                      >
                        <Plus size={15} />
                        <span>添加下载引擎</span>
                        <ChevronDown size={14} />
                      </button>

                      {isAddMenuOpen && (
                        <div className="absolute right-0 z-10 mt-2 w-44 overflow-hidden rounded-md border border-slate-200 bg-white py-1 shadow-lg">
                          {engineOrder.map((engine) => (
                            <button
                              key={engine}
                              type="button"
                              onClick={() => addEngineSettings(engine)}
                              className="flex h-9 w-full items-center px-3 text-left text-sm text-slate-700 hover:bg-slate-50"
                            >
                              {engineLabels[engine]}
                            </button>
                          ))}
                        </div>
                      )}
                      </div>
                    </div>
                  </div>

                  <article className="rounded-lg border border-slate-200 bg-white shadow-sm">
                    <div className="flex items-center justify-between gap-3 border-b border-slate-100 px-4 py-3">
                      <div className="min-w-0">
                        <h2 className="truncate text-sm font-semibold text-slate-950">
                          下载引擎优先级
                        </h2>
                        <div className="mt-1 text-xs text-slate-500">
                          拖拽调整顺序，越靠上优先级越高
                        </div>
                      </div>
                    </div>

                    <div className="flex flex-col gap-4 px-4 py-4">
                      {draftSettings.length === 0 ? (
                        <div className="grid min-h-24 place-items-center rounded-md border border-dashed border-slate-200 text-sm text-slate-500">
                          暂无配置
                        </div>
                      ) : (
                        sortSettings(draftSettings).map((draft, index) => {
                            const saved = savedById.get(draft.id);
                            const dirty = saved ? isDirty(saved, draft) : true;
                            const usesConnection =
                              draft.engine === "aria2" || draft.engine === "qbittorrent";
                            const usesExecutable =
                              draft.engine === "aria2" || draft.engine === "yt-dlp";
                            const canInstall = usesExecutable && Boolean(saved);
                            const isSaving = savingEngineId === draft.id;
                            const isInstalling = installingEngineId === draft.id;
                            const isDeleting = deletingEngineId === draft.id;
                            const isBusy = isSaving || isInstalling || isDeleting;

                            return (
                              <div
                                key={draft.id}
                                draggable
                                onDragStart={(event) => handleDragStart(event, draft.id)}
                                onDragEnd={() => setDraggedEngineId(null)}
                                onDragOver={handleDragOver}
                                onDrop={(event) => handleDrop(event, draft.id)}
                                className="rounded-md border border-slate-200 bg-slate-50/60"
                              >
                                <div className="flex items-center justify-between gap-3 border-b border-slate-200 px-3 py-2">
                                  <div className="flex min-w-0 items-center gap-2">
                                    <GripVertical
                                      size={16}
                                      className="shrink-0 cursor-grab text-slate-400"
                                    />
                                    <div className="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-slate-200 bg-white text-xs font-semibold text-slate-600">
                                      {index + 1}
                                    </div>
                                    <div className="min-w-0">
                                      <div className="truncate text-sm font-medium text-slate-950">
                                        {draft.name}
                                      </div>
                                      <div className="mt-0.5 text-xs text-slate-500">
                                        {engineLabels[draft.engine]} / {draft.id} /{" "}
                                        {draft.updatedAt || "未保存"}
                                      </div>
                                    </div>
                                  </div>

                                  <div className="flex items-center gap-2">
                                    <label
                                      className="relative inline-flex h-6 w-11 cursor-pointer items-center"
                                      title={draft.enabled ? "停用" : "启用"}
                                    >
                                      <input
                                        type="checkbox"
                                        checked={draft.enabled}
                                        onChange={(event) =>
                                          updateDraft(draft.id, {
                                            enabled: event.currentTarget.checked,
                                          })
                                        }
                                        className="peer sr-only"
                                      />
                                      <span className="h-6 w-11 rounded-full bg-slate-300 transition peer-checked:bg-emerald-600 peer-focus-visible:ring-2 peer-focus-visible:ring-emerald-100" />
                                      <span className="absolute left-0.5 h-5 w-5 rounded-full bg-white shadow transition peer-checked:translate-x-5" />
                                    </label>
                                    {savedEngineId === draft.id && (
                                      <InstalledBadge label="已保存" />
                                    )}
                                    <SmallIconButton
                                      title="撤销"
                                      disabled={!dirty || isBusy}
                                      onClick={() => resetEngine(draft.id)}
                                    >
                                      <RotateCcw size={15} />
                                    </SmallIconButton>
                                    <SmallIconButton
                                      title="保存"
                                      disabled={!dirty || isBusy}
                                      onClick={() => void saveEngine(draft.id)}
                                    >
                                      <Save size={15} />
                                    </SmallIconButton>
                                    <SmallIconButton
                                      title="删除"
                                      disabled={isBusy}
                                      onClick={() => void deleteEngine(draft.id)}
                                    >
                                      <Trash2 size={15} />
                                    </SmallIconButton>
                                  </div>
                                </div>

                                <div className="grid gap-4 px-3 py-3 lg:grid-cols-[220px_1fr]">
                                  <div className="flex flex-col gap-2 rounded-md border border-slate-200 bg-white px-3 py-2">
                                    {supportedSourceTypes(draft.engine).map((sourceType) => {
                                      const checked = draft.supportedSourceTypes.includes(sourceType);

                                      return (
                                        <label
                                          key={sourceType}
                                          className={classNames(
                                            "flex h-8 cursor-pointer items-center gap-2 rounded-md border px-2 text-xs transition",
                                            checked
                                              ? "border-emerald-200 bg-emerald-50 text-emerald-800"
                                              : "border-slate-200 bg-slate-50 text-slate-500",
                                          )}
                                        >
                                          <input
                                            type="checkbox"
                                            checked={checked}
                                            onChange={(event) => {
                                              const nextSourceTypes = event.currentTarget.checked
                                                ? sourceTypes.filter(
                                                    (item) =>
                                                      item === sourceType ||
                                                      draft.supportedSourceTypes.includes(item),
                                                  )
                                                : draft.supportedSourceTypes.filter(
                                                    (item) => item !== sourceType,
                                                  );

                                              updateDraft(draft.id, {
                                                supportedSourceTypes: nextSourceTypes,
                                              });
                                            }}
                                            className="h-4 w-4 accent-emerald-700"
                                          />
                                          <span>{sourceLabels[sourceType]}</span>
                                        </label>
                                      );
                                    })}
                                  </div>

                                  <div className="grid gap-4 md:grid-cols-2">
                                    <Field
                                      label="名称"
                                      value={draft.name}
                                      onChange={(value) =>
                                        updateDraft(draft.id, { name: value })
                                      }
                                    />
                                    {usesExecutable && (
                                      <IconField
                                        label="可执行文件"
                                        value={draft.executablePath ?? ""}
                                        onChange={(value) =>
                                          updateDraft(draft.id, { executablePath: value })
                                        }
                                        onClick={() => void installLatest(draft.id)}
                                        buttonTitle="下载最新版"
                                        buttonDisabled={isInstalling || !canInstall || dirty}
                                        buttonLabel={<Download size={15} />}
                                      />
                                    )}
                                    <Field
                                      label="默认下载目录"
                                      value={draft.defaultDownloadDir}
                                      onChange={(value) =>
                                        updateDraft(draft.id, { defaultDownloadDir: value })
                                      }
                                    />
                                    {usesConnection && (
                                      <Field
                                        label="连接地址"
                                        value={draft.connectionUrl ?? ""}
                                        onChange={(value) =>
                                          updateDraft(draft.id, { connectionUrl: value })
                                        }
                                      />
                                    )}
                                    {draft.engine === "qbittorrent" && (
                                      <>
                                        <Field
                                          label="保存路径"
                                          value={draft.remotePath ?? ""}
                                          onChange={(value) =>
                                            updateDraft(draft.id, { remotePath: value })
                                          }
                                        />
                                        <Field
                                          label="用户名"
                                          value={draft.username ?? ""}
                                          onChange={(value) =>
                                            updateDraft(draft.id, { username: value })
                                          }
                                        />
                                        <Field
                                          label="密码"
                                          type="password"
                                          value={draft.password ?? ""}
                                          onChange={(value) =>
                                            updateDraft(draft.id, { password: value })
                                          }
                                        />
                                      </>
                                    )}
                                    <div className="md:col-span-2">
                                      <TextAreaField
                                        label="默认参数"
                                        value={draft.defaultArgs}
                                        onChange={(value) =>
                                          updateDraft(draft.id, { defaultArgs: value })
                                        }
                                      />
                                    </div>
                                  </div>
                                </div>
                              </div>
                            );
                        })
                      )}
                    </div>
                  </article>
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
