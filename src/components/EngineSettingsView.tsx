import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Check, RotateCcw, Save } from "lucide-react";

import { listEngineSettings, saveEngineSettings } from "@/lib/api";
import type {
  EngineKind,
  EngineSettings,
  EngineSettingsInput,
  SourceType,
} from "@shared/types";

const engineOrder: EngineKind[] = ["aria2", "yt-dlp", "qbittorrent"];
const sourceTypes: SourceType[] = ["http", "ftp", "magnet", "torrent"];

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

type EngineSettingsMap = Record<EngineKind, EngineSettings>;

function classNames(...names: Array<string | false | null | undefined>) {
  return names.filter(Boolean).join(" ");
}

function emptyToNull(value: string) {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function toInput(settings: EngineSettings): EngineSettingsInput {
  return {
    engine: settings.engine,
    enabled: settings.enabled,
    executablePath: emptyToNull(settings.executablePath ?? ""),
    defaultDownloadDir: settings.defaultDownloadDir,
    defaultArgs: settings.defaultArgs,
    connectionUrl: emptyToNull(settings.connectionUrl ?? ""),
    username: emptyToNull(settings.username ?? ""),
    password: emptyToNull(settings.password ?? ""),
    remotePath: emptyToNull(settings.remotePath ?? ""),
  };
}

function toMap(settings: EngineSettings[]): EngineSettingsMap {
  return Object.fromEntries(
    settings.map((item) => [item.engine, item]),
  ) as EngineSettingsMap;
}

function isDirty(saved: EngineSettings, draft: EngineSettings) {
  return JSON.stringify(toInput(saved)) !== JSON.stringify(toInput(draft));
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

export default function EngineSettingsView() {
  const [savedSettings, setSavedSettings] = useState<EngineSettingsMap | null>(null);
  const [draftSettings, setDraftSettings] = useState<EngineSettingsMap | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [savingEngine, setSavingEngine] = useState<EngineKind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [savedEngine, setSavedEngine] = useState<EngineKind | null>(null);

  const ready = savedSettings && draftSettings;
  const hasDirtySettings = useMemo(() => {
    if (!ready) {
      return false;
    }
    return engineOrder.some((engine) =>
      isDirty(savedSettings[engine], draftSettings[engine]),
    );
  }, [draftSettings, ready, savedSettings]);

  useEffect(() => {
    async function loadSettings() {
      setIsLoading(true);
      setError(null);

      try {
        const settings = await listEngineSettings();
        const next = toMap(settings);
        setSavedSettings(next);
        setDraftSettings(next);
      } catch (nextError) {
        setError(nextError instanceof Error ? nextError.message : String(nextError));
      } finally {
        setIsLoading(false);
      }
    }

    void loadSettings();
  }, []);

  function updateDraft(engine: EngineKind, patch: Partial<EngineSettings>) {
    setDraftSettings((current) => {
      if (!current) {
        return current;
      }

      return {
        ...current,
        [engine]: {
          ...current[engine],
          ...patch,
        },
      };
    });
  }

  async function saveEngine(engine: EngineKind) {
    if (!draftSettings) {
      return;
    }

    setSavingEngine(engine);
    setError(null);
    setSavedEngine(null);

    try {
      const next = await saveEngineSettings(toInput(draftSettings[engine]));
      setSavedSettings((current) => (current ? { ...current, [engine]: next } : current));
      setDraftSettings((current) => (current ? { ...current, [engine]: next } : current));
      setSavedEngine(engine);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setSavingEngine(null);
    }
  }

  function resetEngine(engine: EngineKind) {
    if (!savedSettings) {
      return;
    }

    setDraftSettings((current) =>
      current ? { ...current, [engine]: savedSettings[engine] } : current,
    );
  }

  return (
    <section className="min-h-0 flex-1 overflow-auto bg-surface">
      <div className="mx-auto flex max-w-6xl flex-col gap-4 px-4 py-4">
        <div className="flex min-h-10 items-center justify-between gap-3">
          <div>
            <h1 className="text-base font-semibold text-slate-950">引擎设置</h1>
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

        {!isLoading &&
          ready &&
          engineOrder.map((engine) => {
            const draft = draftSettings[engine];
            const saved = savedSettings[engine];
            const dirty = isDirty(saved, draft);
            const supports = new Set(draft.supportedSourceTypes);
            const isQBittorrent = engine === "qbittorrent";
            const usesConnection = engine === "aria2" || engine === "qbittorrent";
            const usesExecutable = engine === "aria2" || engine === "yt-dlp";

            return (
              <article
                key={engine}
                className="rounded-lg border border-slate-200 bg-white shadow-sm"
              >
                <div className="flex items-center justify-between gap-3 border-b border-slate-100 px-4 py-3">
                  <div className="min-w-0">
                    <h2 className="truncate text-sm font-semibold text-slate-950">
                      {engineLabels[engine]}
                    </h2>
                    <div className="mt-1 flex flex-wrap gap-2">
                      {sourceTypes.map((sourceType) => {
                        const supported = supports.has(sourceType);
                        return (
                          <label
                            key={sourceType}
                            className={classNames(
                              "inline-flex h-7 items-center gap-1.5 rounded-md border px-2 text-xs",
                              supported
                                ? "border-emerald-200 bg-emerald-50 text-emerald-800"
                                : "border-slate-200 bg-slate-50 text-slate-400",
                            )}
                          >
                            <input
                              type="checkbox"
                              checked={supported}
                              disabled
                              className="h-3.5 w-3.5 accent-emerald-700"
                              readOnly
                            />
                            {sourceLabels[sourceType]}
                          </label>
                        );
                      })}
                    </div>
                  </div>

                  <div className="flex items-center gap-2">
                    {savedEngine === engine && (
                      <span className="inline-flex h-8 items-center gap-1 rounded-md border border-emerald-200 bg-emerald-50 px-2 text-xs text-emerald-800">
                        <Check size={13} />
                        已保存
                      </span>
                    )}
                    <SmallIconButton
                      title="撤销"
                      disabled={!dirty || savingEngine === engine}
                      onClick={() => resetEngine(engine)}
                    >
                      <RotateCcw size={15} />
                    </SmallIconButton>
                    <SmallIconButton
                      title="保存"
                      disabled={!dirty || savingEngine === engine}
                      onClick={() => void saveEngine(engine)}
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
                        checked={draft.enabled}
                        onChange={(event) =>
                          updateDraft(engine, { enabled: event.currentTarget.checked })
                        }
                        className="h-4 w-4 accent-emerald-700"
                      />
                    </label>
                  </div>

                  <div className="grid gap-4 md:grid-cols-2">
                    {usesExecutable && (
                      <Field
                        label="可执行文件"
                        value={draft.executablePath ?? ""}
                        onChange={(value) => updateDraft(engine, { executablePath: value })}
                      />
                    )}
                    <Field
                      label="默认下载目录"
                      value={draft.defaultDownloadDir}
                      onChange={(value) =>
                        updateDraft(engine, { defaultDownloadDir: value })
                      }
                    />
                    {usesConnection && (
                      <Field
                        label="连接地址"
                        value={draft.connectionUrl ?? ""}
                        onChange={(value) => updateDraft(engine, { connectionUrl: value })}
                      />
                    )}
                    {isQBittorrent && (
                      <>
                        <Field
                          label="保存路径"
                          value={draft.remotePath ?? ""}
                          onChange={(value) => updateDraft(engine, { remotePath: value })}
                        />
                        <Field
                          label="用户名"
                          value={draft.username ?? ""}
                          onChange={(value) => updateDraft(engine, { username: value })}
                        />
                        <Field
                          label="密码"
                          type="password"
                          value={draft.password ?? ""}
                          onChange={(value) => updateDraft(engine, { password: value })}
                        />
                      </>
                    )}
                    <div className="md:col-span-2">
                      <TextAreaField
                        label="默认参数"
                        value={draft.defaultArgs}
                        onChange={(value) => updateDraft(engine, { defaultArgs: value })}
                      />
                    </div>
                  </div>
                </div>
              </article>
            );
          })}
      </div>
    </section>
  );
}
