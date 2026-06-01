import { useEffect, useMemo, useState } from "react";
import type { DragEvent as ReactDragEvent, ReactNode } from "react";
import { confirm, openDialog } from "@/lib/tauri";
import {
  Code2,
  Check,
  ChevronDown,
  ChevronRight,
  Database,
  Download,
  Eye,
  EyeOff,
  FolderOpen,
  Globe2,
  GripVertical,
  HardDrive,
  Info,
  Network,
  Plus,
  PlugZap,
  RotateCcw,
  Save,
  Settings2,
  Shield,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  Workflow,
} from "lucide-react";

import {
  clearDownloadRecords,
  deleteEngineSettings,
  getAppSettings,
  getManagedEngineExecutablePath,
  getSystemDownloadDir,
  installLatestEngine,
  listEngineSettings,
  openExternalUrl,
  refreshDownloadTasks,
  saveAppSettings,
  saveEngineSettings,
  testEngineConnection,
  updateEngineTrackers,
  writeLog,
} from "@/lib/api";
import { hasTauriRuntime } from "@/lib/runtime";
import type {
  AppSettings,
  AppSettingsInput,
  DownloadTask,
  EngineKind,
  EngineSettings,
  EngineSettingsInput,
  SourceType,
} from "@shared/types";
import packageJson from "../../package.json";
import logoUrl from "../../logo.png";

const ERROR_AUTO_DISMISS_MS = 10_000;
const DEFAULT_LOCAL_DOWNLOAD_CONCURRENCY = 5;
const DEFAULT_AUTO_CLEAN_DOWNLOAD_TASK_DAYS = 365;
const ARIA2_DEFAULT_RPC_LISTEN_ADDRESS = "127.0.0.1";
const ARIA2_DEFAULT_RPC_LISTEN_PORT = 6800;
const ARIA2_RPC_PATH = "/jsonrpc";
const ARIA2_DEFAULT_BT_LISTEN_PORT = 6881;
const ARIA2_DEFAULT_BT_MAX_PEERS = 55;
const ARIA2_DEFAULT_MAX_CONNECTION_PER_SERVER = 16;
const ARIA2_DEFAULT_SPLIT = 16;
const ARIA2_DEFAULT_MIN_SPLIT_SIZE = "1M";
const ARIA2_DEFAULT_FILE_ALLOCATION = "none";
const ARIA2_DEFAULT_SEED_TIME = 10;
const ARIA2_DEFAULT_SEED_RATIO = 1;
const SPEED_LIMIT_BYTES_PER_MEGABYTE = 1024 * 1024;
const aria2FileAllocationOptions = ["none", "prealloc", "trunc", "falloc"] as const;

const engineOrder: EngineKind[] = ["aria2", "yt-dlp", "qbittorrent"];
const sourceTypes: SourceType[] = ["http", "ftp", "magnet", "torrent"];
type SettingsGroup =
  | "general"
  | "web-access"
  | "privacy"
  | "data"
  | "download-engines"
  | "about";
type ThemeMode = AppSettings["themeMode"];
type Aria2BtToggleKey =
  | "aria2EnableDht"
  | "aria2EnableDht6"
  | "aria2EnablePeerExchange"
  | "aria2EnableLpd";

type DownloadRecordCleanupOption = {
  id: string;
  label: string;
  olderThanDays: number | null;
};

const proxySchemePrefixesAll = [
  "http://",
  "https://",
  "socks5://",
  "socks5h://",
  "socks4://",
  "socks4a://",
];

const proxySchemePrefixesHttpOnly = ["http://", "https://"];

function engineProxySchemePrefixes(engine?: EngineKind): string[] {
  if (engine === "aria2") {
    return proxySchemePrefixesHttpOnly;
  }
  return proxySchemePrefixesAll;
}

function formatProxySchemeHint(prefixes: string[]): string {
  return prefixes.map((prefix) => prefix.replace(/:\/\/$/, "")).join("、");
}

function validateProxyUrl(value: string, engine?: EngineKind): string | null {
  const trimmed = value.trim();
  if (!trimmed) {
    return null;
  }

  const allowed = engineProxySchemePrefixes(engine);
  const lower = trimmed.toLowerCase();
  if (!allowed.some((prefix) => lower.startsWith(prefix))) {
    if (engine === "aria2") {
      return "aria2 仅支持 http / https 代理，不支持 SOCKS";
    }
    return `代理地址需以 ${formatProxySchemeHint(allowed)} 开头`;
  }

  try {
    const url = new URL(trimmed);
    if (!url.hostname) {
      return "代理地址缺少主机名";
    }
  } catch {
    return "代理地址格式无效";
  }

  return null;
}

const defaultTrackerSubscriptionUrl =
  "https://raw.githubusercontent.com/ngosang/trackerslist/master/trackers_best.txt";

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

const aboutHighlights = [
  {
    title: "多协议入口",
    description: "HTTP、FTP、磁力链接与种子文件都能进入同一套任务流。",
    icon: Globe2,
  },
  {
    title: "多引擎调度",
    description: "aria2、yt-dlp、qBittorrent 可并存配置并按优先级协作。",
    icon: Workflow,
  },
  {
    title: "浏览器接力",
    description: "通过 Chromium MV3 扩展把网页里的下载请求快速送到本机。",
    icon: Download,
  },
  {
    title: "桌面与 Web 共用",
    description: "同一套界面既能跑在 Tauri 桌面端，也能经 Web 方式访问。",
    icon: Code2,
  },
] as const;

const aboutTechStack = [
  "Tauri 2",
  "React 19",
  "TypeScript",
  "Tailwind CSS",
  "Rust",
  "SQLite",
  "MV3 Extension",
];

const projectRepositoryUrl = "https://github.com/fonlan/UniDL";

const downloadRecordCleanupOptions: DownloadRecordCleanupOption[] = [
  { id: "day", label: "1 天前", olderThanDays: 1 },
  { id: "week", label: "1 周前", olderThanDays: 7 },
  { id: "month", label: "1 月前", olderThanDays: 30 },
  { id: "quarter", label: "3 月前", olderThanDays: 90 },
  { id: "year", label: "1 年前", olderThanDays: 365 },
  { id: "all", label: "全部记录", olderThanDays: null },
];

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

function defaultAria2RpcConnectionUrl(port = ARIA2_DEFAULT_RPC_LISTEN_PORT) {
  return `http://${ARIA2_DEFAULT_RPC_LISTEN_ADDRESS}:${port}${ARIA2_RPC_PATH}`;
}

function parseAria2RpcConnectionUrl(value: string | null | undefined) {
  const trimmed =
    value === null || value === undefined ? defaultAria2RpcConnectionUrl() : value.trim();
  if (!trimmed) {
    throw new Error("aria2 RPC 连接地址不能为空");
  }
  const normalized = trimmed.endsWith(ARIA2_RPC_PATH)
    ? trimmed
    : `${trimmed.replace(/\/+$/, "")}${ARIA2_RPC_PATH}`;
  return new URL(normalized);
}

function parseAria2RpcConnectionUrlOrDefault(value: string | null | undefined) {
  try {
    return parseAria2RpcConnectionUrl(value);
  } catch {
    return new URL(defaultAria2RpcConnectionUrl());
  }
}

function aria2RpcListenAddress(value: string | null | undefined) {
  return parseAria2RpcConnectionUrl(value).hostname || ARIA2_DEFAULT_RPC_LISTEN_ADDRESS;
}

function aria2RpcListenPort(value: string | null | undefined) {
  const url = parseAria2RpcConnectionUrl(value);
  if (!url.port) {
    throw new Error("aria2 RPC 连接地址必须包含端口");
  }
  const port = Number.parseInt(url.port, 10);
  if (!Number.isInteger(port) || port < 1 || port > 65_535) {
    throw new Error("aria2 RPC 监听端口必须是 1 到 65535 的整数");
  }
  return port;
}

function updateAria2RpcListenAddress(value: string | null | undefined, address: string) {
  const trimmed = address.trim();
  if (!trimmed) {
    return "";
  }
  const url = parseAria2RpcConnectionUrlOrDefault(value);
  url.hostname = trimmed;
  url.pathname = ARIA2_RPC_PATH;
  return url.toString();
}

function updateAria2RpcListenPort(value: string | null | undefined, port: number) {
  const url = parseAria2RpcConnectionUrlOrDefault(value);
  url.pathname = ARIA2_RPC_PATH;
  if (!Number.isFinite(port)) {
    url.port = "";
    return url.toString();
  }

  const normalizedPort = Math.trunc(port);
  if (normalizedPort < 0 || normalizedPort > 65_535) {
    const host =
      url.hostname.includes(":") && !url.hostname.startsWith("[")
        ? `[${url.hostname}]`
        : url.hostname;
    return `${url.protocol}//${host}:${normalizedPort}${ARIA2_RPC_PATH}`;
  }
  url.port = String(normalizedPort);
  return url.toString();
}

function aria2RpcInputPort(value: string | null | undefined) {
  try {
    return aria2RpcListenPort(value);
  } catch {
    return Number.NaN;
  }
}

function aria2RpcInputUrl(value: string | null | undefined) {
  try {
    const url = parseAria2RpcConnectionUrl(value);
    if ((url.protocol !== "http:" && url.protocol !== "https:") || !url.hostname) {
      return null;
    }
    return url;
  } catch {
    return null;
  }
}

function nextAria2RpcListenPort(settings: EngineSettings[]) {
  const ports = settings
    .filter((item) => item.engine === "aria2")
    .map((item) => aria2RpcListenPort(item.connectionUrl));
  if (ports.length === 0) {
    return ARIA2_DEFAULT_RPC_LISTEN_PORT;
  }

  const nextPort = Math.max(...ports) + 1;
  if (nextPort > 65_535) {
    throw new Error("aria2 RPC 监听端口已超过 65535，无法自动递增");
  }
  return nextPort;
}

function nextAria2BtListenPort(settings: EngineSettings[]) {
  const ports = settings
    .filter((item) => item.engine === "aria2")
    .map((item) => item.aria2BtListenPort)
    .filter(Number.isFinite);
  if (ports.length === 0) {
    return ARIA2_DEFAULT_BT_LISTEN_PORT;
  }

  const nextPort = Math.max(...ports) + 1;
  if (nextPort > 65_535) {
    throw new Error("aria2 BT 监听端口已超过 65535，无法自动递增");
  }
  return nextPort;
}

function validateAria2Settings(settings: EngineSettings) {
  if (settings.engine !== "aria2") {
    return null;
  }
  let rpcUrl: URL;
  try {
    rpcUrl = parseAria2RpcConnectionUrl(settings.connectionUrl);
  } catch {
    return "aria2 RPC 连接地址格式无效";
  }
  if (rpcUrl.protocol !== "http:" && rpcUrl.protocol !== "https:") {
    return "aria2 RPC 连接地址需以 http 或 https 开头";
  }
  if (!rpcUrl.hostname) {
    return "aria2 RPC 监听地址不能为空";
  }
  if (!rpcUrl.port) {
    return "aria2 RPC 连接地址必须包含端口";
  }
  const rpcPort = Number.parseInt(rpcUrl.port, 10);
  if (!Number.isInteger(rpcPort) || rpcPort < 1 || rpcPort > 65_535) {
    return "aria2 RPC 监听端口必须是 1 到 65535 的整数";
  }
  if (
    !Number.isInteger(settings.aria2BtListenPort) ||
    settings.aria2BtListenPort < 1 ||
    settings.aria2BtListenPort > 65_535
  ) {
    return "BT 下载监听端口必须是 1 到 65535 的整数";
  }
  if (!Number.isInteger(settings.aria2BtMaxPeers) || settings.aria2BtMaxPeers < 0) {
    return "BT 下载的最大 Peer 数量不能为负数";
  }
  if (
    !Number.isInteger(settings.aria2MaxConnectionPerServer) ||
    settings.aria2MaxConnectionPerServer < 1
  ) {
    return "单服务器最大连接数必须是至少 1 的整数";
  }
  if (!Number.isInteger(settings.aria2Split) || settings.aria2Split < 1) {
    return "分片数必须是至少 1 的整数";
  }
  if (
    settings.aria2MinSplitSize.trim().length === 0 ||
    /\s/.test(settings.aria2MinSplitSize)
  ) {
    return "最小分片大小不能为空，也不能包含空格";
  }
  if (
    !aria2FileAllocationOptions.includes(
      settings.aria2FileAllocation as (typeof aria2FileAllocationOptions)[number],
    )
  ) {
    return "文件预分配方式无效";
  }
  if (!Number.isInteger(settings.aria2SeedTime) || settings.aria2SeedTime < 0) {
    return "下载完成后持续做种时间不能为负数";
  }
  if (!Number.isFinite(settings.aria2SeedRatio) || settings.aria2SeedRatio < 0) {
    return "下载完成后持续做种分享率不能为负数";
  }
  return null;
}

function validateQbittorrentSettings(settings: EngineSettings) {
  if (settings.engine !== "qbittorrent") {
    return null;
  }
  if (
    !Number.isInteger(settings.qbittorrentDownloadLimitBytesPerSec) ||
    settings.qbittorrentDownloadLimitBytesPerSec < 0
  ) {
    return "qBittorrent 下载限速不能为负数";
  }
  if (
    !Number.isInteger(settings.qbittorrentUploadLimitBytesPerSec) ||
    settings.qbittorrentUploadLimitBytesPerSec < 0
  ) {
    return "qBittorrent 上传限速不能为负数";
  }
  if (
    !Number.isFinite(settings.qbittorrentSeedRatioLimit) ||
    settings.qbittorrentSeedRatioLimit < 0
  ) {
    return "qBittorrent 分享率限制不能为负数";
  }
  if (
    !Number.isInteger(settings.qbittorrentSeedTimeLimitMinutes) ||
    settings.qbittorrentSeedTimeLimitMinutes < 0
  ) {
    return "qBittorrent 做种时间限制不能为负数";
  }
  return null;
}

function defaultEngineSettings(
  engine: EngineKind,
  defaultDownloadDir: string,
  executablePath: string | null,
  aria2BtListenPort = ARIA2_DEFAULT_BT_LISTEN_PORT,
  aria2RpcListenPort = ARIA2_DEFAULT_RPC_LISTEN_PORT,
): EngineSettings {
  return {
    id: crypto.randomUUID(),
    engine,
    name: engineLabels[engine],
    enabled: false,
    executablePath,
    defaultDownloadDir,
    defaultArgs: "",
    connectionUrl:
      engine === "aria2"
        ? defaultAria2RpcConnectionUrl(aria2RpcListenPort)
        : engine === "qbittorrent"
          ? "http://127.0.0.1:8080"
          : null,
    username: null,
    password: null,
    remotePath: engine === "qbittorrent" ? "" : null,
    supportedSourceTypes: supportedSourceTypes(engine),
    preferredDomains: [],
    trackerSubscriptionUrl: engine === "aria2" ? defaultTrackerSubscriptionUrl : null,
    trackers: [],
    proxyUrl: null,
    userAgent: null,
    speedLimitBytesPerSec: 0,
    qbittorrentDownloadLimitBytesPerSec: 0,
    qbittorrentUploadLimitBytesPerSec: 0,
    qbittorrentSeedRatioLimit: 0,
    qbittorrentSeedTimeLimitMinutes: 0,
    aria2EnableDht: true,
    aria2EnableDht6: true,
    aria2EnablePeerExchange: true,
    aria2EnableLpd: true,
    aria2BtListenPort,
    aria2BtMaxPeers: ARIA2_DEFAULT_BT_MAX_PEERS,
    aria2MaxConnectionPerServer: ARIA2_DEFAULT_MAX_CONNECTION_PER_SERVER,
    aria2Split: ARIA2_DEFAULT_SPLIT,
    aria2MinSplitSize: ARIA2_DEFAULT_MIN_SPLIT_SIZE,
    aria2FileAllocation: ARIA2_DEFAULT_FILE_ALLOCATION,
    aria2SeedTime: ARIA2_DEFAULT_SEED_TIME,
    aria2SeedRatio: ARIA2_DEFAULT_SEED_RATIO,
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

function normalizeSpeedLimitBytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) {
    return 0;
  }
  return Math.trunc(value);
}

function speedLimitMegabytes(value: number) {
  const bytes = normalizeSpeedLimitBytes(value);
  return bytes > 0 ? Number((bytes / SPEED_LIMIT_BYTES_PER_MEGABYTE).toFixed(2)) : 0;
}

function speedLimitBytesFromMegabytes(value: number) {
  if (!Number.isFinite(value) || value <= 0) {
    return 0;
  }
  return Math.round(value * SPEED_LIMIT_BYTES_PER_MEGABYTE);
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
    preferredDomains: normalizePreferredDomains(settings.preferredDomains),
    trackerSubscriptionUrl: emptyToNull(settings.trackerSubscriptionUrl ?? ""),
    trackers: normalizeTrackers(settings.trackers),
    proxyUrl: emptyToNull(settings.proxyUrl ?? ""),
    userAgent: emptyToNull(settings.userAgent ?? ""),
    speedLimitBytesPerSec: normalizeSpeedLimitBytes(settings.speedLimitBytesPerSec),
    qbittorrentDownloadLimitBytesPerSec: normalizeSpeedLimitBytes(
      settings.qbittorrentDownloadLimitBytesPerSec,
    ),
    qbittorrentUploadLimitBytesPerSec: normalizeSpeedLimitBytes(
      settings.qbittorrentUploadLimitBytesPerSec,
    ),
    qbittorrentSeedRatioLimit: Number.isFinite(settings.qbittorrentSeedRatioLimit)
      ? settings.qbittorrentSeedRatioLimit
      : 0,
    qbittorrentSeedTimeLimitMinutes: Math.max(
      0,
      Math.trunc(settings.qbittorrentSeedTimeLimitMinutes),
    ),
    aria2EnableDht: settings.aria2EnableDht,
    aria2EnableDht6: settings.aria2EnableDht6,
    aria2EnablePeerExchange: settings.aria2EnablePeerExchange,
    aria2EnableLpd: settings.aria2EnableLpd,
    aria2BtListenPort: settings.aria2BtListenPort,
    aria2BtMaxPeers: settings.aria2BtMaxPeers,
    aria2MaxConnectionPerServer: Math.trunc(settings.aria2MaxConnectionPerServer),
    aria2Split: Math.trunc(settings.aria2Split),
    aria2MinSplitSize: settings.aria2MinSplitSize.trim(),
    aria2FileAllocation: settings.aria2FileAllocation.trim().toLowerCase(),
    aria2SeedTime: settings.aria2SeedTime,
    aria2SeedRatio: settings.aria2SeedRatio,
    priority: settings.priority,
  };
}

function aria2BtTogglePatch(
  key: Aria2BtToggleKey,
  checked: boolean,
): Partial<EngineSettings> {
  switch (key) {
    case "aria2EnableDht":
      return { aria2EnableDht: checked };
    case "aria2EnableDht6":
      return { aria2EnableDht6: checked };
    case "aria2EnablePeerExchange":
      return { aria2EnablePeerExchange: checked };
    case "aria2EnableLpd":
      return { aria2EnableLpd: checked };
  }
}

function normalizePreferredDomains(domains: string[]) {
  return domains
    .map((domain) => domain.trim().toLowerCase())
    .filter((domain) => domain.length > 0);
}

function preferredDomainsText(domains: string[]) {
  return domains.join("\n");
}

function parsePreferredDomains(value: string) {
  return normalizePreferredDomains(value.split(/[\s,;]+/));
}

function normalizeTrackers(trackers: string[]) {
  const normalized: string[] = [];
  for (const tracker of trackers) {
    const trimmed = tracker.trim();
    if (!trimmed || !trimmed.includes("://")) {
      continue;
    }
    if (!normalized.some((item) => item.toLowerCase() === trimmed.toLowerCase())) {
      normalized.push(trimmed);
    }
  }
  return normalized;
}

function trackersText(trackers: string[]) {
  return trackers.join("\n");
}

function parseTrackers(value: string) {
  return normalizeTrackers(value.split(/[\s,;]+/));
}

function normalizeAutoCleanDownloadTaskDays(value: number) {
  if (!Number.isFinite(value)) {
    return DEFAULT_AUTO_CLEAN_DOWNLOAD_TASK_DAYS;
  }
  return Math.max(1, Math.trunc(value));
}

function normalizeLocalDownloadConcurrency(value: number) {
  if (!Number.isFinite(value)) {
    return DEFAULT_LOCAL_DOWNLOAD_CONCURRENCY;
  }
  return Math.max(1, Math.trunc(value));
}

function isDirty(saved: EngineSettings, draft: EngineSettings) {
  return JSON.stringify(toInput(saved)) !== JSON.stringify(toInput(draft));
}

function toAppInput(settings: AppSettings): AppSettingsInput {
  return {
    themeMode: settings.themeMode,
    webAccessEnabled: settings.webAccessEnabled,
    webAccessPassword: settings.webAccessPassword,
    webAccessUrl: settings.webAccessUrl,
    privateDownloadDomains: normalizePreferredDomains(settings.privateDownloadDomains),
    appProxyUrl: settings.appProxyUrl.trim(),
    torrentFileAssociationEnabled: settings.torrentFileAssociationEnabled,
    autoStartEnabled: settings.autoStartEnabled,
    autoStartMinimizedToTray: settings.autoStartMinimizedToTray,
    closeToTrayEnabled: settings.closeToTrayEnabled,
    downloadCompletionNotificationEnabled: settings.downloadCompletionNotificationEnabled,
    preventSleepWhenDownloadingEnabled: settings.preventSleepWhenDownloadingEnabled,
    preventSleepWhenWebAccessEnabled: settings.preventSleepWhenWebAccessEnabled,
    localDownloadConcurrency: normalizeLocalDownloadConcurrency(
      settings.localDownloadConcurrency,
    ),
    autoCleanDownloadTasksEnabled: settings.autoCleanDownloadTasksEnabled,
    autoCleanDownloadTasksDays: normalizeAutoCleanDownloadTaskDays(
      settings.autoCleanDownloadTasksDays,
    ),
  };
}

function isAppDirty(saved: AppSettings, draft: AppSettings) {
  return JSON.stringify(toAppInput(saved)) !== JSON.stringify(toAppInput(draft));
}

function requiredInputClass(isMissingRequired: boolean) {
  return isMissingRequired
    ? "border-rose-200 bg-rose-50/30 text-slate-900 focus:border-rose-400 focus:ring-2 focus:ring-rose-100"
    : "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100";
}

function missingEngineRequiredFields(settings: EngineSettings) {
  const missing: string[] = [];
  if (!settings.name.trim()) {
    missing.push("名称");
  }
  if (settings.engine === "aria2" || settings.engine === "yt-dlp") {
    if (!settings.executablePath?.trim()) {
      missing.push("可执行文件");
    }
    if (!settings.defaultDownloadDir.trim()) {
      missing.push("默认下载目录");
    }
  }
  if (settings.engine === "qbittorrent") {
    if (!settings.connectionUrl?.trim()) {
      missing.push("连接地址");
    }
    if (!settings.username?.trim()) {
      missing.push("用户名");
    }
    if (!settings.password?.trim()) {
      missing.push("密码");
    }
  }
  return missing;
}

function Field({
  label,
  value,
  type = "text",
  required = false,
  onChange,
}: {
  label: string;
  value: string;
  type?: "text" | "password";
  required?: boolean;
  onChange: (value: string) => void;
}) {
  const [isPasswordVisible, setIsPasswordVisible] = useState(false);
  const isPassword = type === "password";
  const isMissingRequired = required && !value.trim();
  const passwordToggleLabel = isPasswordVisible ? "隐藏密码" : "显示密码";

  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <div className="relative">
        <input
          type={isPassword && isPasswordVisible ? "text" : type}
          value={value}
          onChange={(event) => onChange(event.currentTarget.value)}
          aria-invalid={isMissingRequired || undefined}
          className={classNames(
            "h-9 w-full rounded-md border px-3 text-sm outline-none transition",
            requiredInputClass(isMissingRequired),
            isPassword && "pr-10",
          )}
        />
        {isPassword && (
          <button
            type="button"
            title={passwordToggleLabel}
            aria-label={passwordToggleLabel}
            onClick={() => setIsPasswordVisible((current) => !current)}
            className="absolute inset-y-0 right-0 grid w-9 place-items-center text-slate-500 transition hover:text-slate-700"
          >
            {isPasswordVisible ? <EyeOff size={16} /> : <Eye size={16} />}
          </button>
        )}
      </div>
    </label>
  );
}

function NumberField({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <input
        type="number"
        value={Number.isFinite(value) ? value : ""}
        min={min}
        max={max}
        step={step}
        onChange={(event) => onChange(event.currentTarget.valueAsNumber)}
        className="h-9 w-full rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
      />
    </label>
  );
}

function DirectoryField({
  label,
  value,
  required = false,
  onChange,
  onBrowse,
}: {
  label: string;
  value: string;
  required?: boolean;
  onChange: (value: string) => void;
  onBrowse: () => void;
}) {
  const isMissingRequired = required && !value.trim();

  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <div className="flex min-w-0 items-center gap-2">
        <input
          value={value}
          onChange={(event) => onChange(event.currentTarget.value)}
          aria-invalid={isMissingRequired || undefined}
          className={classNames(
            "h-9 min-w-0 flex-1 rounded-md border px-3 text-sm outline-none transition",
            requiredInputClass(isMissingRequired),
          )}
        />
        <button
          type="button"
          title="浏览下载目录"
          aria-label="浏览下载目录"
          onClick={onBrowse}
          className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-slate-200 bg-white text-slate-700 transition hover:border-slate-300 hover:bg-slate-50"
        >
          <FolderOpen size={15} />
        </button>
      </div>
    </label>
  );
}

async function pickDownloadDirectory(currentPath: string) {
  const selected = await openDialog({
    directory: true,
    multiple: false,
    defaultPath: currentPath.trim() || undefined,
    title: "选择下载目录",
  });

  return typeof selected === "string" ? selected : null;
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
  required = false,
  onChange,
  onClick,
  buttonTitle,
  buttonDisabled,
  buttonLabel,
}: {
  label: string;
  value: string;
  required?: boolean;
  onChange: (value: string) => void;
  onClick: () => void;
  buttonTitle: string;
  buttonDisabled?: boolean;
  buttonLabel: ReactNode;
}) {
  const isMissingRequired = required && !value.trim();

  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
      <span className="font-medium">{label}</span>
      <div className="flex min-w-0 items-center gap-2">
        <input
          value={value}
          onChange={(event) => onChange(event.currentTarget.value)}
          aria-invalid={isMissingRequired || undefined}
          className={classNames(
            "h-9 min-w-0 flex-1 rounded-md border px-3 text-sm outline-none transition",
            requiredInputClass(isMissingRequired),
          )}
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

function SettingsSwitch({
  checked,
  label,
  description,
  onToggle,
}: {
  checked: boolean;
  label: string;
  description: string;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border border-slate-200 bg-slate-50/50 px-3 py-3">
      <div className="min-w-0">
        <div className="text-sm font-medium text-slate-800">{label}</div>
        <div className="mt-1 text-xs leading-5 text-slate-500">{description}</div>
      </div>
      <label className="shrink-0 cursor-pointer">
        <input
          type="checkbox"
          checked={checked}
          onChange={() => onToggle()}
          className="peer sr-only"
        />
        <span className="sr-only">{label}</span>
        <span
          className={classNames(
            "inline-flex h-8 items-center gap-2 rounded-full border px-2.5 text-xs font-medium transition peer-focus-visible:ring-2 peer-focus-visible:ring-emerald-100",
            checked
              ? "border-emerald-200 bg-emerald-50 text-emerald-800"
              : "border-slate-200 bg-white text-slate-500",
          )}
        >
          <span
            className={classNames(
              "h-3.5 w-3.5 rounded-full transition",
              checked ? "bg-emerald-600" : "bg-slate-300",
            )}
          />
          {checked ? "已启用" : "已关闭"}
        </span>
      </label>
    </div>
  );
}

function AboutMetaRow({
  label,
  value,
}: {
  label: string;
  value: ReactNode;
}) {
  return (
    <div className="flex items-start justify-between gap-3 border-b border-slate-100/80 py-2 last:border-b-0 last:pb-0">
      <span className="text-xs uppercase tracking-[0.12em] text-slate-400">{label}</span>
      <span className="break-all text-right text-sm font-medium text-slate-700">{value}</span>
    </div>
  );
}

type EngineSettingsViewProps = {
  onDownloadRecordsCleared?: (tasks: DownloadTask[]) => void;
  onThemeModeChange?: (themeMode: ThemeMode) => void;
};

export default function EngineSettingsView({
  onDownloadRecordsCleared,
  onThemeModeChange,
}: EngineSettingsViewProps) {
  const [activeGroup, setActiveGroup] = useState<SettingsGroup>("download-engines");
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
  const [testingEngineId, setTestingEngineId] = useState<string | null>(null);
  const [updatingTrackersEngineId, setUpdatingTrackersEngineId] = useState<string | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [savedApp, setSavedApp] = useState(false);
  const [savedEngineId, setSavedEngineId] = useState<string | null>(null);
  const [testedEngineId, setTestedEngineId] = useState<string | null>(null);
  const [draggedEngineId, setDraggedEngineId] = useState<string | null>(null);
  const [expandedAdvanced, setExpandedAdvanced] = useState<Set<string>>(new Set());
  const [clearingRecordsOptionId, setClearingRecordsOptionId] = useState<string | null>(
    null,
  );
  const [cleanupResult, setCleanupResult] = useState<string | null>(null);

  function toggleAdvanced(settingsId: string) {
    setExpandedAdvanced((current) => {
      const next = new Set(current);
      if (next.has(settingsId)) {
        next.delete(settingsId);
      } else {
        next.add(settingsId);
      }
      return next;
    });
  }

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

  const engineRequiredErrors = useMemo(() => {
    const errors = new Map<string, string>();
    for (const draft of draftSettings) {
      const missing = missingEngineRequiredFields(draft);
      if (missing.length > 0) {
        errors.set(draft.id, `请填写必填项：${missing.join("、")}`);
      }
    }
    return errors;
  }, [draftSettings]);

  const engineProxyErrors = useMemo(() => {
    const errors = new Map<string, string>();
    for (const draft of draftSettings) {
      const error = validateProxyUrl(draft.proxyUrl ?? "", draft.engine);
      if (error) {
        errors.set(draft.id, error);
      }
    }
    return errors;
  }, [draftSettings]);

  const engineAria2Errors = useMemo(() => {
    const errors = new Map<string, string>();
    for (const draft of draftSettings) {
      const error = validateAria2Settings(draft);
      if (error) {
        errors.set(draft.id, error);
      }
    }

    const rpcPorts = new Map<number, string[]>();
    for (const draft of draftSettings) {
      if (draft.engine !== "aria2" || errors.has(draft.id)) {
        continue;
      }
      const port = aria2RpcListenPort(draft.connectionUrl);
      rpcPorts.set(port, [...(rpcPorts.get(port) ?? []), draft.id]);
    }
    for (const duplicatedIds of rpcPorts.values()) {
      if (duplicatedIds.length <= 1) {
        continue;
      }
      for (const settingsId of duplicatedIds) {
        errors.set(settingsId, "多个 aria2 下载引擎不能使用相同 RPC 监听端口");
      }
    }
    return errors;
  }, [draftSettings]);

  const engineQbittorrentErrors = useMemo(() => {
    const errors = new Map<string, string>();
    for (const draft of draftSettings) {
      const error = validateQbittorrentSettings(draft);
      if (error) {
        errors.set(draft.id, error);
      }
    }
    return errors;
  }, [draftSettings]);

  const hasEngineRequiredErrors = engineRequiredErrors.size > 0;
  const hasEngineProxyErrors = engineProxyErrors.size > 0;
  const hasEngineAria2Errors = engineAria2Errors.size > 0;
  const hasEngineQbittorrentErrors = engineQbittorrentErrors.size > 0;
  const hasEngineErrors =
    hasEngineRequiredErrors ||
    hasEngineProxyErrors ||
    hasEngineAria2Errors ||
    hasEngineQbittorrentErrors;

  const hasDirtySettings = appDirty || dirtySettings;
  const engineSettingsCount = draftSettings.length;
  const enabledEnginesCount = useMemo(
    () => draftSettings.filter((item) => item.enabled).length,
    [draftSettings],
  );
  const runtimeLabel = hasTauriRuntime() ? "Tauri 桌面模式" : "Web 控制台模式";
  const webAccessLabel = draftAppSettings?.webAccessEnabled
    ? "远程访问已开启"
    : "远程访问按需开启";
  const aboutStats = [
    {
      label: "支持来源",
      value: `${sourceTypes.length} 种`,
      description: sourceTypes.map((type) => sourceLabels[type]).join(" / "),
    },
    {
      label: "支持引擎",
      value: `${engineOrder.length} 类`,
      description: engineOrder.map((engine) => engineLabels[engine]).join(" / "),
    },
    {
      label: "当前启用",
      value: `${enabledEnginesCount} 个`,
      description:
        engineSettingsCount > 0 ? `共配置 ${engineSettingsCount} 个下载引擎` : "尚未配置下载引擎",
    },
  ];

  const openProjectRepository = () => {
    setError(null);
    void openExternalUrl(projectRepositoryUrl).catch((nextError) => {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    });
  };

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

  function updateAppDraft(patch: Partial<AppSettings>) {
    setSavedApp(false);
    setDraftAppSettings((current) => (current ? { ...current, ...patch } : current));
  }

  function updateThemeMode(themeMode: ThemeMode) {
    updateAppDraft({ themeMode });
    onThemeModeChange?.(themeMode);
  }

  async function saveAppAccess() {
    if (!draftAppSettings) {
      return;
    }

    setIsSavingApp(true);
    setError(null);
    setSavedApp(false);

    try {
      void writeLog("info", "saving app access settings");
      const next = await saveAppSettings(toAppInput(draftAppSettings));
      setSavedAppSettings(next);
      setDraftAppSettings(next);
      onThemeModeChange?.(next.themeMode);
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
    onThemeModeChange?.(savedAppSettings.themeMode);
  }

  function updateDraft(settingsId: string, patch: Partial<EngineSettings>) {
    setTestedEngineId(null);
    setDraftSettings((current) =>
      current.map((item) => (item.id === settingsId ? { ...item, ...patch } : item)),
    );
  }

  async function addEngineSettings(engine: EngineKind) {
    setError(null);

    try {
      let defaultDownloadDir = "";
      let executablePath: string | null = null;
      if (isLocalDownloadEngine(engine)) {
        defaultDownloadDir = await getSystemDownloadDir();
        executablePath = await getManagedEngineExecutablePath(engine);
      }

      setSavedEngineId(null);
      setTestedEngineId(null);
      setIsAddMenuOpen(false);
      setDraftSettings((current) => {
        const aria2BtListenPort =
          engine === "aria2"
            ? nextAria2BtListenPort(current)
            : ARIA2_DEFAULT_BT_LISTEN_PORT;
        const aria2RpcListenPort =
          engine === "aria2"
            ? nextAria2RpcListenPort(current)
            : ARIA2_DEFAULT_RPC_LISTEN_PORT;
        const next = {
          ...defaultEngineSettings(
            engine,
            defaultDownloadDir,
            executablePath,
            aria2BtListenPort,
            aria2RpcListenPort,
          ),
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
    setTestedEngineId(null);
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

  function handleDragStart(event: ReactDragEvent<HTMLElement>, settingsId: string) {
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
      void writeLog("info", `deleting engine settings: id=${settingsId}`);
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
      void writeLog("info", `saving engine settings: id=${settingsId}`);
      const next = await saveEngineSettings(toInput(draft));
      setSavedSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setDraftSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setSavedEngineId(next.id);
      setTestedEngineId(null);
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
    setTestedEngineId(null);

    try {
      void writeLog("info", "saving all engine settings");
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
      sortSettings(current.map((item) => (item.id === settingsId ? { ...saved } : item))),
    );
    setTestedEngineId(null);
  }

  async function installLatest(settingsId: string) {
    setInstallingEngineId(settingsId);
    setError(null);

    try {
      void writeLog("info", `installing latest engine: id=${settingsId}`);
      const next = await installLatestEngine(settingsId);
      setSavedSettings((current) =>
        sortSettings([
          ...current.filter((item) => item.id !== next.settings.id),
          next.settings,
        ]),
      );
      setDraftSettings((current) =>
        sortSettings([
          ...current.filter((item) => item.id !== next.settings.id),
          next.settings,
        ]),
      );
      setSavedEngineId(next.settings.id);
      setTestedEngineId(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setInstallingEngineId(null);
    }
  }

  async function testConnection(settingsId: string) {
    const draft = draftSettings.find((item) => item.id === settingsId);
    if (!draft) {
      return;
    }

    setTestingEngineId(settingsId);
    setError(null);
    setTestedEngineId(null);

    try {
      void writeLog("info", `testing engine connection: id=${settingsId}`);
      await testEngineConnection(toInput(draft));
      setTestedEngineId(settingsId);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setTestingEngineId(null);
    }
  }

  async function updateTrackers(settingsId: string) {
    const draft = draftSettings.find((item) => item.id === settingsId);
    if (!draft) {
      return;
    }

    const subscriptionUrls =
      draft.trackerSubscriptionUrl?.trim() || defaultTrackerSubscriptionUrl;
    setUpdatingTrackersEngineId(settingsId);
    setError(null);
    setSavedEngineId(null);

    try {
      void writeLog("info", `updating engine trackers: id=${settingsId}`);
      const next = await updateEngineTrackers(settingsId, subscriptionUrls);
      setSavedSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setDraftSettings((current) =>
        sortSettings([...current.filter((item) => item.id !== next.id), next]),
      );
      setSavedEngineId(next.id);
      setTestedEngineId(null);
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setUpdatingTrackersEngineId(null);
    }
  }

  async function cleanupDownloadRecords(option: DownloadRecordCleanupOption) {
    const scope =
      option.olderThanDays === null
        ? "全部已结束下载记录"
        : `${option.label}的已结束下载记录`;
    const confirmed = await confirm(
      `确定要清除${scope}？\n\n此操作只删除下载列表记录，不会删除已下载文件；排队中、下载中和暂停中的任务会保留。`,
      {
        title: "清理下载记录",
        kind: "warning",
        okLabel: "清除",
        cancelLabel: "取消",
      },
    );
    if (!confirmed) {
      return;
    }

    setClearingRecordsOptionId(option.id);
    setCleanupResult(null);
    setError(null);

    try {
      void writeLog(
        "info",
        `clearing download records: olderThanDays=${option.olderThanDays ?? "all"}`,
      );
      const deletedCount = await clearDownloadRecords(option.olderThanDays);
      const nextTasks = await refreshDownloadTasks();
      onDownloadRecordsCleared?.(nextTasks);
      setCleanupResult(
        deletedCount > 0 ? `已清除 ${deletedCount} 条下载记录` : "没有符合条件的下载记录",
      );
    } catch (nextError) {
      setError(nextError instanceof Error ? nextError.message : String(nextError));
    } finally {
      setClearingRecordsOptionId(null);
    }
  }

  return (
    <section className="min-h-0 flex-1 overflow-auto bg-surface">
      <div className="mx-auto flex max-w-6xl flex-col gap-5 px-5 py-6">
        <div className="flex min-h-10 items-end justify-between gap-3">
          <div className="min-w-0">
            <div className="flex items-center gap-2 text-slate-950">
              <Settings2 size={18} className="text-slate-500" />
              <h1 className="text-lg font-semibold tracking-tight">设置</h1>
            </div>
            <div className="mt-1.5 inline-flex items-center gap-1.5 text-xs text-slate-500">
              <span
                className={classNames(
                  "h-1.5 w-1.5 rounded-full",
                  hasDirtySettings ? "bg-amber-500" : "bg-emerald-500",
                )}
              />
              {hasDirtySettings ? "有未保存更改" : "设置已保存"}
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
          <div className="grid min-h-0 gap-6 lg:grid-cols-[208px_1fr]">
            <aside className="min-w-0">
              <nav className="flex gap-2 overflow-x-auto rounded-xl border border-slate-200 bg-white p-3 shadow-sm lg:flex-col lg:gap-4 lg:overflow-visible">
                <div className="flex flex-col gap-1 lg:contents">
                  <div className="hidden px-2 text-[10px] font-semibold uppercase tracking-[0.14em] text-slate-400 lg:block">
                    常用
                  </div>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("download-engines")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "download-engines"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <HardDrive
                      size={16}
                      className={
                        activeGroup === "download-engines"
                          ? "text-emerald-700"
                          : "text-slate-500"
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">下载引擎</span>
                    <span
                      className={classNames(
                        "shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium tabular-nums",
                        activeGroup === "download-engines"
                          ? "bg-white/70 text-emerald-700"
                          : "bg-slate-100 text-slate-600",
                      )}
                    >
                      {enabledEnginesCount}/{engineSettingsCount}
                    </span>
                  </button>
                </div>

                <div className="hidden h-px bg-slate-100 lg:block" />

                <div className="flex flex-col gap-1 lg:contents">
                  <div className="hidden px-2 text-[10px] font-semibold uppercase tracking-[0.14em] text-slate-400 lg:block">
                    应用
                  </div>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("general")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "general"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <SlidersHorizontal
                      size={16}
                      className={
                        activeGroup === "general" ? "text-emerald-700" : "text-slate-500"
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">常规</span>
                    <span
                      className={classNames(
                        "h-1.5 w-1.5 shrink-0 rounded-full",
                        draftAppSettings?.appProxyUrl.trim() ||
                          draftAppSettings?.torrentFileAssociationEnabled ||
                          draftAppSettings?.autoStartEnabled ||
                          draftAppSettings?.autoStartMinimizedToTray ||
                          draftAppSettings?.closeToTrayEnabled ||
                          draftAppSettings?.downloadCompletionNotificationEnabled ||
                          draftAppSettings?.preventSleepWhenDownloadingEnabled ||
                          draftAppSettings?.localDownloadConcurrency !==
                          DEFAULT_LOCAL_DOWNLOAD_CONCURRENCY
                          ? "bg-emerald-500"
                          : "bg-slate-300",
                      )}
                    />
                  </button>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("data")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "data"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <Database
                      size={16}
                      className={
                        activeGroup === "data" ? "text-emerald-700" : "text-slate-500"
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">数据</span>
                    <span
                      className={classNames(
                        "h-1.5 w-1.5 shrink-0 rounded-full",
                        draftAppSettings?.autoCleanDownloadTasksEnabled || cleanupResult
                          ? "bg-emerald-500"
                          : "bg-slate-300",
                      )}
                    />
                  </button>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("web-access")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "web-access"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <Globe2
                      size={16}
                      className={
                        activeGroup === "web-access"
                          ? "text-emerald-700"
                          : "text-slate-500"
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">Web 访问</span>
                    <span
                      className={classNames(
                        "h-1.5 w-1.5 shrink-0 rounded-full",
                        draftAppSettings?.webAccessEnabled ||
                          draftAppSettings?.preventSleepWhenWebAccessEnabled
                          ? "bg-emerald-500"
                          : "bg-slate-300",
                      )}
                    />
                  </button>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("privacy")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "privacy"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <Shield
                      size={16}
                      className={
                        activeGroup === "privacy" ? "text-emerald-700" : "text-slate-500"
                      }
                    />
                    <span className="min-w-0 flex-1 truncate">隐私</span>
                    {(draftAppSettings?.privateDownloadDomains.length ?? 0) > 0 && (
                      <span
                        className={classNames(
                          "shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium tabular-nums",
                          activeGroup === "privacy"
                            ? "bg-white/70 text-emerald-700"
                            : "bg-slate-100 text-slate-600",
                        )}
                      >
                        {draftAppSettings?.privateDownloadDomains.length}
                      </span>
                    )}
                  </button>
                  <button
                    type="button"
                    onClick={() => setActiveGroup("about")}
                    className={classNames(
                      "flex min-w-44 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition lg:min-w-0",
                      activeGroup === "about"
                        ? "bg-emerald-50 text-emerald-800"
                        : "text-slate-700 hover:bg-slate-50",
                    )}
                  >
                    <Info
                      size={16}
                      className={activeGroup === "about" ? "text-emerald-700" : "text-slate-500"}
                    />
                    <span className="min-w-0 flex-1 truncate">关于</span>
                    <span
                      className={classNames(
                        "shrink-0 rounded-full px-1.5 py-0.5 text-[10px] font-medium tabular-nums",
                        activeGroup === "about"
                          ? "bg-white/70 text-emerald-700"
                          : "bg-slate-100 text-slate-600",
                      )}
                    >
                      v{packageJson.version}
                    </span>
                  </button>
                </div>
              </nav>
            </aside>

            <div className="min-w-0">
              {activeGroup === "about" && (
                <article className="overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
                  <div className="relative overflow-hidden border-b border-slate-100 bg-[linear-gradient(135deg,#fff8ef_0%,#ffffff_48%,#e8f6ef_100%)] px-5 py-5">
                    <div className="absolute inset-x-0 top-0 h-px bg-[linear-gradient(90deg,rgba(15,23,42,0)_0%,rgba(5,150,105,0.35)_50%,rgba(15,23,42,0)_100%)]" />
                    <div className="absolute -left-12 top-5 h-28 w-28 rounded-full bg-amber-200/40 blur-3xl" />
                    <div className="absolute right-0 top-0 h-32 w-32 rounded-full bg-emerald-200/50 blur-3xl" />

                    <div className="relative">
                      <div className="min-w-0">
                        <div className="inline-flex items-center gap-2 rounded-full border border-slate-200/80 bg-white/80 px-3 py-1 text-[11px] font-medium uppercase tracking-[0.18em] text-slate-600 backdrop-blur">
                          <Sparkles size={13} className="text-emerald-600" />
                          Unified Download Manager
                        </div>

                        <div className="mt-4 flex items-start gap-4">
                          <div className="grid h-16 w-16 shrink-0 place-items-center rounded-[1.25rem] border border-slate-200 bg-slate-950/95 shadow-[0_16px_40px_-24px_rgba(15,23,42,0.85)]">
                            <img src={logoUrl} alt="UniDL" className="h-10 w-10 object-contain" />
                          </div>

                          <div className="min-w-0">
                            <h2 className="about-editorial-heading text-[2rem] leading-none text-slate-950">
                              UniDL
                            </h2>
                            <p className="mt-3 max-w-2xl text-sm leading-6 text-slate-700">
                              把网页捕获、本地下载引擎和远程访问统一进一个桌面工作台，
                              让多源下载不再分散在浏览器、命令行和不同客户端之间。
                            </p>

                            <div className="mt-4 flex flex-wrap gap-2 text-xs text-slate-600">
                              <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-200 bg-white/80 px-3 py-1.5">
                                <span className="h-2 w-2 rounded-full bg-emerald-500" />v{packageJson.version}
                              </span>
                              <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-200 bg-white/80 px-3 py-1.5">
                                <span className="h-2 w-2 rounded-full bg-slate-400" />{runtimeLabel}
                              </span>
                              <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-200 bg-white/80 px-3 py-1.5">
                                <span className="h-2 w-2 rounded-full bg-amber-500" />MIT 协议
                              </span>
                              <span className="inline-flex items-center gap-1.5 rounded-full border border-slate-200 bg-white/80 px-3 py-1.5">
                                <span className="h-2 w-2 rounded-full bg-sky-500" />{webAccessLabel}
                              </span>
                            </div>
                          </div>
                        </div>
                      </div>
                    </div>
                  </div>

                  <div className="grid gap-5 px-5 py-5 xl:grid-cols-[minmax(0,1.2fr)_minmax(300px,0.8fr)]">
                    <div className="grid gap-5">
                      <div className="grid gap-3 sm:grid-cols-3">
                        {aboutStats.map((item) => (
                          <div
                            key={item.label}
                            className="rounded-2xl border border-slate-200 bg-slate-50/70 px-4 py-4 shadow-[inset_0_1px_0_rgba(255,255,255,0.8)]"
                          >
                            <div className="text-[11px] font-semibold uppercase tracking-[0.16em] text-slate-400">
                              {item.label}
                            </div>
                            <div className="mt-3 text-2xl font-semibold tracking-tight text-slate-950">
                              {item.value}
                            </div>
                            <div className="mt-2 text-xs leading-5 text-slate-500">
                              {item.description}
                            </div>
                          </div>
                        ))}
                      </div>

                      <section className="rounded-2xl border border-slate-200 bg-white p-4 shadow-sm">
                        <div className="flex items-center gap-2 text-sm font-semibold text-slate-900">
                          <Workflow size={16} className="text-emerald-600" />
                          核心能力
                        </div>
                        <div className="mt-4 grid gap-3 sm:grid-cols-2">
                          {aboutHighlights.map((item) => {
                            const Icon = item.icon;
                            return (
                              <div
                                key={item.title}
                                className="rounded-2xl border border-slate-200 bg-[linear-gradient(180deg,#ffffff_0%,#f8fafc_100%)] p-4"
                              >
                                <div className="flex items-center gap-2">
                                  <div className="grid h-9 w-9 place-items-center rounded-xl border border-emerald-100 bg-emerald-50 text-emerald-700">
                                    <Icon size={16} />
                                  </div>
                                  <div className="text-sm font-medium text-slate-900">{item.title}</div>
                                </div>
                                <div className="mt-3 text-sm leading-6 text-slate-600">
                                  {item.description}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      </section>
                    </div>

                    <div className="grid gap-5">
                      <section className="rounded-2xl border border-slate-200 bg-slate-50/70 p-4 shadow-sm">
                        <div className="flex items-center gap-2 text-sm font-semibold text-slate-900">
                          <Code2 size={16} className="text-emerald-600" />
                          技术栈
                        </div>
                        <div className="mt-4 flex flex-wrap gap-2">
                          {aboutTechStack.map((item) => (
                            <span
                              key={item}
                              className="inline-flex items-center rounded-full border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-700"
                            >
                              {item}
                            </span>
                          ))}
                        </div>
                        <div className="mt-4 rounded-2xl border border-slate-200 bg-white px-4 py-3 text-xs leading-6 text-slate-500">
                          前端使用 React 19、TypeScript 与 Tailwind CSS；桌面壳基于 Tauri 2，后端服务与任务编排由 Rust 提供。
                        </div>
                      </section>

                      <section className="rounded-2xl border border-slate-200 bg-white p-4 shadow-sm">
                        <div className="flex items-center gap-2 text-sm font-semibold text-slate-900">
                          <Info size={16} className="text-emerald-600" />
                          项目信息
                        </div>
                        <div className="mt-4">
                          <AboutMetaRow label="产品名" value="UniDL" />
                          <AboutMetaRow
                            label="项目地址"
                            value={
                              <button
                                type="button"
                                onClick={openProjectRepository}
                                className="text-right text-emerald-700 underline decoration-emerald-300 underline-offset-2 transition hover:text-emerald-800"
                              >
                                {projectRepositoryUrl}
                              </button>
                            }
                          />
                          <AboutMetaRow label="版本" value={`v${packageJson.version}`} />
                          <AboutMetaRow label="当前模式" value={runtimeLabel} />
                          <AboutMetaRow label="桌面壳" value="Tauri 2" />
                          <AboutMetaRow label="扩展形态" value="Chromium MV3" />
                          <AboutMetaRow label="协议" value="MIT" />
                        </div>
                      </section>

                      <section className="rounded-2xl border border-emerald-100 bg-emerald-50/70 p-4 shadow-sm">
                        <div className="flex items-center gap-2 text-sm font-semibold text-emerald-900">
                          <Sparkles size={16} className="text-emerald-700" />
                          一句话说明
                        </div>
                        <div className="mt-3 text-sm leading-6 text-emerald-900/85">
                          一个面向高频下载场景的统一工作台，把浏览器里的“发现”、本地引擎的“执行”和任务列表里的“管理”连成同一条链路。
                        </div>
                      </section>
                    </div>
                  </div>
                </article>
              )}

              {activeGroup === "general" &&
                draftAppSettings &&
                (() => {
                  const proxyError = validateProxyUrl(draftAppSettings.appProxyUrl);
                  return (
                    <article className="overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
                      <div className="border-b border-slate-100 px-5 py-4">
                        <h2 className="truncate text-sm font-semibold text-slate-950">
                          常规
                        </h2>
                        <p className="mt-1 text-xs text-slate-500">
                          程序自身访问外网（下载 aria2c / yt-dlp、Tracker
                          订阅等）时使用的代理。仅影响 UniDL
                          本身的请求，不影响下载引擎下载文件。
                        </p>
                      </div>

                      <div className="grid gap-4 px-5 py-5">
                        <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                          <span className="font-medium">应用代理地址</span>
                          <div className="flex min-w-0 items-center gap-2">
                            <Network size={15} className="shrink-0 text-slate-400" />
                            <input
                              value={draftAppSettings.appProxyUrl}
                              placeholder="留空表示直连，例如 http://127.0.0.1:7890 或 socks5://127.0.0.1:1080"
                              onChange={(event) =>
                                updateAppDraft({ appProxyUrl: event.currentTarget.value })
                              }
                              className={classNames(
                                "h-9 min-w-0 flex-1 rounded-md border px-3 text-sm outline-none transition",
                                proxyError
                                  ? "border-rose-300 bg-white text-rose-700 focus:border-rose-500 focus:ring-2 focus:ring-rose-100"
                                  : "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100",
                              )}
                            />
                          </div>
                          <span
                            className={classNames(
                              "text-xs",
                              proxyError ? "text-rose-600" : "text-slate-500",
                            )}
                          >
                            {proxyError ??
                              "支持 http / https / socks4 / socks4a / socks5 / socks5h。"}
                          </span>
                        </label>

                        <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                          <span className="font-medium">本地下载并发任务数</span>
                          <div className="flex min-w-0 items-center gap-2">
                            <Download size={15} className="shrink-0 text-slate-400" />
                            <input
                              type="number"
                              min={1}
                              step={1}
                              value={draftAppSettings.localDownloadConcurrency}
                              onChange={(event) =>
                                updateAppDraft({
                                  localDownloadConcurrency:
                                    normalizeLocalDownloadConcurrency(
                                      event.currentTarget.valueAsNumber,
                                    ),
                                })
                              }
                              onBlur={() =>
                                updateAppDraft({
                                  localDownloadConcurrency:
                                    normalizeLocalDownloadConcurrency(
                                      draftAppSettings.localDownloadConcurrency,
                                    ),
                                })
                              }
                              className="h-9 min-w-0 flex-1 rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                            />
                          </div>
                          <span className="text-xs text-slate-500">
                            只限制 aria2 和 yt-dlp 这类本地下载引擎，qBittorrent
                            不受影响。
                          </span>
                        </label>

                        <div className="grid gap-3">
                          <SettingsSwitch
                            checked={draftAppSettings.themeMode === "dark"}
                            label="深色模式"
                            description="切换 UniDL 主界面和设置页的深色外观。"
                            onToggle={() =>
                              updateThemeMode(
                                draftAppSettings.themeMode === "dark"
                                  ? "light"
                                  : "dark",
                              )
                            }
                          />
                          <SettingsSwitch
                            checked={draftAppSettings.torrentFileAssociationEnabled}
                            label="关联 .torrent 文件"
                            description="开启后，双击种子文件会直接交给 UniDL 创建下载任务。"
                            onToggle={() =>
                              updateAppDraft({
                                torrentFileAssociationEnabled:
                                  !draftAppSettings.torrentFileAssociationEnabled,
                              })
                            }
                          />
                          <SettingsSwitch
                            checked={draftAppSettings.autoStartEnabled}
                            label="开机自启动"
                            description="登录系统后自动启动 UniDL。"
                            onToggle={() =>
                              updateAppDraft({
                                autoStartEnabled: !draftAppSettings.autoStartEnabled,
                              })
                            }
                          />
                          <SettingsSwitch
                            checked={draftAppSettings.autoStartMinimizedToTray}
                            label="自启动后隐藏到系统托盘"
                            description="仅在开机自启动拉起程序时生效，避免显示主窗口。"
                            onToggle={() =>
                              updateAppDraft({
                                autoStartMinimizedToTray:
                                  !draftAppSettings.autoStartMinimizedToTray,
                              })
                            }
                          />
                          <SettingsSwitch
                            checked={draftAppSettings.closeToTrayEnabled}
                            label="关闭按钮隐藏到系统托盘"
                            description="点击窗口关闭按钮时保留后台运行，仅隐藏主窗口。"
                            onToggle={() =>
                              updateAppDraft({
                                closeToTrayEnabled: !draftAppSettings.closeToTrayEnabled,
                              })
                            }
                          />
                          <SettingsSwitch
                            checked={
                              draftAppSettings.downloadCompletionNotificationEnabled
                            }
                            label="下载完成时弹出系统通知"
                            description="任务完成后通过系统通知提醒。"
                            onToggle={() =>
                              updateAppDraft({
                                downloadCompletionNotificationEnabled:
                                  !draftAppSettings.downloadCompletionNotificationEnabled,
                              })
                            }
                          />
                          <SettingsSwitch
                            checked={draftAppSettings.preventSleepWhenDownloadingEnabled}
                            label="有活动下载时阻止系统休眠"
                            description="存在排队中或下载中的任务时阻止系统进入休眠。"
                            onToggle={() =>
                              updateAppDraft({
                                preventSleepWhenDownloadingEnabled:
                                  !draftAppSettings.preventSleepWhenDownloadingEnabled,
                              })
                            }
                          />
                        </div>
                      </div>

                      <div className="flex items-center justify-between gap-3 border-t border-slate-100 bg-slate-50/50 px-5 py-3">
                        <div className="min-w-0 text-xs text-slate-500">
                          {savedApp ? (
                            <span className="inline-flex items-center gap-1.5 text-emerald-700">
                              <Check size={13} />
                              已保存到本地
                            </span>
                          ) : appDirty ? (
                            "修改后请记得保存"
                          ) : draftAppSettings.appProxyUrl.trim() ? (
                            "当前已配置应用代理"
                          ) : (
                            "未配置应用代理，将使用系统直连"
                          )}
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            disabled={!appDirty || isSavingApp}
                            onClick={resetAppAccess}
                            className={classNames(
                              "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm transition",
                              (!appDirty || isSavingApp) &&
                              "cursor-not-allowed border-slate-200 bg-slate-50 text-slate-400",
                              appDirty &&
                              !isSavingApp &&
                              "border-slate-200 bg-white text-slate-700 hover:border-slate-300",
                            )}
                          >
                            <RotateCcw size={14} />
                            撤销
                          </button>
                          <button
                            type="button"
                            disabled={!appDirty || isSavingApp || Boolean(proxyError)}
                            onClick={() => void saveAppAccess()}
                            className={classNames(
                              "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                              (!appDirty || isSavingApp || proxyError) &&
                              "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                              appDirty &&
                              !isSavingApp &&
                              !proxyError &&
                              "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100",
                            )}
                          >
                            <Save size={14} />
                            保存
                          </button>
                        </div>
                      </div>
                    </article>
                  );
                })()}

              {activeGroup === "web-access" && draftAppSettings && (
                <article className="overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
                  <div className="flex items-start justify-between gap-3 border-b border-slate-100 px-5 py-4">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        Web 访问
                      </h2>
                      <p className="mt-1 text-xs text-slate-500">
                        通过浏览器访问 UniDL 控制台，便于反代后远程使用。
                      </p>
                    </div>

                    <label className="shrink-0 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={draftAppSettings.webAccessEnabled}
                        onChange={(event) =>
                          updateAppDraft({
                            webAccessEnabled: event.currentTarget.checked,
                          })
                        }
                        className="peer sr-only"
                      />
                      <span className="sr-only">Web 访问</span>
                      <span
                        className={classNames(
                          "inline-flex h-8 items-center gap-2 rounded-full border px-2.5 text-xs font-medium transition peer-focus-visible:ring-2 peer-focus-visible:ring-emerald-100",
                          draftAppSettings.webAccessEnabled
                            ? "border-emerald-200 bg-emerald-50 text-emerald-800"
                            : "border-slate-200 bg-slate-50 text-slate-500",
                        )}
                      >
                        <span
                          className={classNames(
                            "h-3.5 w-3.5 rounded-full transition",
                            draftAppSettings.webAccessEnabled
                              ? "bg-emerald-600"
                              : "bg-slate-300",
                          )}
                        />
                        {draftAppSettings.webAccessEnabled ? "已启用" : "已关闭"}
                      </span>
                    </label>
                  </div>

                  <div className="grid gap-4 px-5 py-5 md:grid-cols-2">
                    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                      <span className="font-medium">访问地址</span>
                      <input
                        value={draftAppSettings.webAccessUrl}
                        readOnly={draftAppSettings.webAccessEnabled}
                        onChange={(event) =>
                          updateAppDraft({ webAccessUrl: event.currentTarget.value })
                        }
                        className={classNames(
                          "h-9 rounded-md border px-3 text-sm outline-none transition",
                          draftAppSettings.webAccessEnabled
                            ? "border-slate-200 bg-slate-50 text-slate-700"
                            : "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100",
                        )}
                      />
                      {draftAppSettings.webAccessEnabled && (
                        <span className="text-xs text-slate-500">
                          启用状态下不可编辑，请先关闭后再修改。
                        </span>
                      )}
                    </label>
                    <Field
                      label="访问密码"
                      type="password"
                      value={draftAppSettings.webAccessPassword}
                      onChange={(value) => updateAppDraft({ webAccessPassword: value })}
                    />
                  </div>

                  <div className="border-t border-slate-100 px-5 py-5">
                    <SettingsSwitch
                      checked={draftAppSettings.preventSleepWhenWebAccessEnabled}
                      label="启用 Web 访问时阻止系统休眠"
                      description="Web 访问开启期间保持系统唤醒，便于持续远程访问。"
                      onToggle={() =>
                        updateAppDraft({
                          preventSleepWhenWebAccessEnabled:
                            !draftAppSettings.preventSleepWhenWebAccessEnabled,
                        })
                      }
                    />
                  </div>

                  <div className="flex items-center justify-between gap-3 border-t border-slate-100 bg-slate-50/50 px-5 py-3">
                    <div className="min-w-0 text-xs text-slate-500">
                      {savedApp ? (
                        <span className="inline-flex items-center gap-1.5 text-emerald-700">
                          <Check size={13} />
                          已保存到本地
                        </span>
                      ) : appDirty ? (
                        "修改后请记得保存"
                      ) : (
                        "当前为已保存配置"
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={resetAppAccess}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-50 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-slate-200 bg-white text-slate-700 hover:border-slate-300",
                        )}
                      >
                        <RotateCcw size={14} />
                        撤销
                      </button>
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={() => void saveAppAccess()}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100",
                        )}
                      >
                        <Save size={14} />
                        保存
                      </button>
                    </div>
                  </div>
                </article>
              )}

              {activeGroup === "data" && draftAppSettings && (
                <article className="overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
                  <div className="flex items-start justify-between gap-3 border-b border-slate-100 px-5 py-4">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        数据
                      </h2>
                      <p className="mt-1 text-xs text-slate-500">
                        清理下载列表中的历史记录，保留仍在进行的任务。
                      </p>
                    </div>
                  </div>

                  <div className="grid gap-5 px-5 py-5">
                    <div className="grid gap-3">
                      <SettingsSwitch
                        checked={draftAppSettings.autoCleanDownloadTasksEnabled}
                        label="自动清理下载任务"
                        description="开启后会在启动、保存设置和后台检查时清理过期的已结束任务记录。"
                        onToggle={() =>
                          updateAppDraft({
                            autoCleanDownloadTasksEnabled:
                              !draftAppSettings.autoCleanDownloadTasksEnabled,
                          })
                        }
                      />

                      <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                        <span className="font-medium">清理范围</span>
                        <div className="flex min-w-0 flex-wrap items-center gap-2">
                          <input
                            type="number"
                            min={1}
                            step={1}
                            inputMode="numeric"
                            disabled={!draftAppSettings.autoCleanDownloadTasksEnabled}
                            value={draftAppSettings.autoCleanDownloadTasksDays}
                            onChange={(event) =>
                              updateAppDraft({
                                autoCleanDownloadTasksDays:
                                  normalizeAutoCleanDownloadTaskDays(
                                    Number.parseInt(event.currentTarget.value, 10),
                                  ),
                              })
                            }
                            className={classNames(
                              "h-9 w-32 rounded-md border px-3 text-sm outline-none transition",
                              draftAppSettings.autoCleanDownloadTasksEnabled
                                ? "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                                : "border-slate-200 bg-slate-50 text-slate-400",
                            )}
                          />
                          <span className="text-sm text-slate-600">
                            天前的已结束任务记录
                          </span>
                        </div>
                        <span className="text-xs text-slate-500">
                          默认 365 天；只删除列表记录，不删除磁盘文件。
                        </span>
                      </label>
                    </div>

                    <div className="border-t border-slate-100 pt-5">
                      <div className="flex items-start gap-3">
                        <div className="grid h-9 w-9 shrink-0 place-items-center rounded-md border border-rose-100 bg-rose-50 text-rose-700">
                          <Trash2 size={16} />
                        </div>
                        <div className="min-w-0">
                          <div className="text-sm font-medium text-slate-800">
                            下载任务清理
                          </div>
                          <div className="mt-1 text-xs leading-5 text-slate-500">
                            只删除列表记录，不删除磁盘文件；排队中、下载中和暂停中的任务不会被清理。
                          </div>
                        </div>
                      </div>

                      <div className="mt-4 grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                        {downloadRecordCleanupOptions.map((option) => {
                          const isAll = option.olderThanDays === null;
                          const isCurrent = clearingRecordsOptionId === option.id;
                          const isClearing = clearingRecordsOptionId !== null;

                          return (
                            <button
                              key={option.id}
                              type="button"
                              disabled={isClearing}
                              onClick={() => void cleanupDownloadRecords(option)}
                              className={classNames(
                                "inline-flex h-9 items-center justify-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                                isClearing &&
                                "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                                !isClearing &&
                                !isAll &&
                                "border-slate-200 bg-white text-slate-700 hover:border-slate-300 hover:bg-slate-50",
                                !isClearing &&
                                isAll &&
                                "border-rose-200 bg-rose-50 text-rose-700 hover:bg-rose-100",
                              )}
                            >
                              <Trash2 size={14} />
                              {isCurrent ? "清理中" : `清除${option.label}`}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center justify-between gap-3 border-t border-slate-100 bg-slate-50/50 px-5 py-3">
                    <div className="min-w-0 text-xs text-slate-500">
                      {savedApp ? (
                        <span className="inline-flex items-center gap-1.5 text-emerald-700">
                          <Check size={13} />
                          已保存到本地
                        </span>
                      ) : appDirty ? (
                        "修改后请记得保存"
                      ) : cleanupResult ? (
                        cleanupResult
                      ) : draftAppSettings.autoCleanDownloadTasksEnabled ? (
                        `将自动清理 ${draftAppSettings.autoCleanDownloadTasksDays} 天前的已结束任务记录`
                      ) : (
                        "自动清理已关闭；手动清理前会再次确认。"
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={resetAppAccess}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-50 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-slate-200 bg-white text-slate-700 hover:border-slate-300",
                        )}
                      >
                        <RotateCcw size={14} />
                        撤销
                      </button>
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={() => void saveAppAccess()}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100",
                        )}
                      >
                        <Save size={14} />
                        保存
                      </button>
                    </div>
                  </div>
                </article>
              )}

              {activeGroup === "privacy" && draftAppSettings && (
                <article className="overflow-hidden rounded-xl border border-slate-200 bg-white shadow-sm">
                  <div className="flex items-start justify-between gap-3 border-b border-slate-100 px-5 py-4">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        隐私
                      </h2>
                      <p className="mt-1 text-xs text-slate-500">
                        匹配域名的下载完成后不会保留在任务列表中，敏感站点的下载记录将自动清理。
                      </p>
                    </div>
                  </div>

                  <div className="px-5 py-5">
                    <TextAreaField
                      label="不保留下载记录的域名（每行一个）"
                      value={preferredDomainsText(
                        draftAppSettings.privateDownloadDomains,
                      )}
                      onChange={(value) =>
                        updateAppDraft({
                          privateDownloadDomains: parsePreferredDomains(value),
                        })
                      }
                    />
                    <p className="mt-2 text-xs text-slate-500">
                      支持 example.com、*.example.com；子域名会自动匹配。
                    </p>
                  </div>

                  <div className="flex items-center justify-between gap-3 border-t border-slate-100 bg-slate-50/50 px-5 py-3">
                    <div className="min-w-0 text-xs text-slate-500">
                      {savedApp ? (
                        <span className="inline-flex items-center gap-1.5 text-emerald-700">
                          <Check size={13} />
                          已保存到本地
                        </span>
                      ) : appDirty ? (
                        "修改后请记得保存"
                      ) : (
                        `当前 ${draftAppSettings.privateDownloadDomains.length} 个域名`
                      )}
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={resetAppAccess}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-50 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-slate-200 bg-white text-slate-700 hover:border-slate-300",
                        )}
                      >
                        <RotateCcw size={14} />
                        撤销
                      </button>
                      <button
                        type="button"
                        disabled={!appDirty || isSavingApp}
                        onClick={() => void saveAppAccess()}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                          (!appDirty || isSavingApp) &&
                          "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400",
                          appDirty &&
                          !isSavingApp &&
                          "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100",
                        )}
                      >
                        <Save size={14} />
                        保存
                      </button>
                    </div>
                  </div>
                </article>
              )}

              {activeGroup === "download-engines" && (
                <div className="flex flex-col gap-4">
                  <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-slate-200 bg-white px-5 py-4 shadow-sm">
                    <div className="min-w-0">
                      <h2 className="truncate text-sm font-semibold text-slate-950">
                        下载引擎
                      </h2>
                      <div className="mt-1 text-xs text-slate-500">
                        {engineSettingsCount > 0
                          ? `共 ${engineSettingsCount} 个引擎，已启用 ${enabledEnginesCount} 个 · 拖拽手柄调整优先级`
                          : "尚未添加任何下载引擎"}
                      </div>
                    </div>

                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        disabled={!dirtySettings || isSavingEngines || hasEngineErrors}
                        onClick={() => void saveAllEngines()}
                        title={hasEngineErrors ? "请先修正引擎设置错误" : undefined}
                        className={classNames(
                          "inline-flex h-9 items-center gap-2 rounded-md border px-3 text-sm font-medium transition",
                          (!dirtySettings || isSavingEngines || hasEngineErrors) &&
                          "cursor-not-allowed border-slate-200 bg-slate-50 text-slate-400",
                          dirtySettings &&
                          !isSavingEngines &&
                          !hasEngineErrors &&
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
                          className="inline-flex h-9 items-center gap-2 rounded-md border border-emerald-200 bg-emerald-50 px-3 text-sm font-medium text-emerald-800 transition hover:bg-emerald-100"
                        >
                          <Plus size={15} />
                          <span>添加引擎</span>
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

                  {draftSettings.length === 0 ? (
                    <div className="grid min-h-32 place-items-center rounded-xl border border-dashed border-slate-300 bg-white text-sm text-slate-500">
                      点击右上角“添加引擎”开始配置
                    </div>
                  ) : (
                    <div className="flex flex-col gap-3">
                      {sortSettings(draftSettings).map((draft, index) => {
                        const saved = savedById.get(draft.id);
                        const dirty = saved ? isDirty(saved, draft) : true;
                        const usesConnection = draft.engine === "qbittorrent";
                        const usesExecutable =
                          draft.engine === "aria2" || draft.engine === "yt-dlp";
                        const canInstall = usesExecutable && Boolean(saved);
                        const isSaving = savingEngineId === draft.id;
                        const isInstalling = installingEngineId === draft.id;
                        const isTesting = testingEngineId === draft.id;
                        const isUpdatingTrackers = updatingTrackersEngineId === draft.id;
                        const isDeleting = deletingEngineId === draft.id;
                        const isBusy =
                          isSaving ||
                          isInstalling ||
                          isTesting ||
                          isUpdatingTrackers ||
                          isDeleting;
                        const isAdvancedOpen = expandedAdvanced.has(draft.id);
                        const isDragging = draggedEngineId === draft.id;
                        const cardRequiredError =
                          engineRequiredErrors.get(draft.id) ?? null;
                        const cardProxyError = engineProxyErrors.get(draft.id) ?? null;
                        const cardAria2Error = engineAria2Errors.get(draft.id) ?? null;
                        const cardQbittorrentError =
                          engineQbittorrentErrors.get(draft.id) ?? null;
                        const cardAria2RpcError =
                          cardAria2Error && cardAria2Error.includes("RPC")
                            ? cardAria2Error
                            : null;
                        const cardAria2TransferError =
                          cardAria2Error &&
                            ["单服务器", "分片", "最小分片", "文件预分配"].some((keyword) =>
                              cardAria2Error.includes(keyword),
                            )
                            ? cardAria2Error
                            : null;
                        const cardAria2BtError =
                          cardAria2Error && !cardAria2RpcError && !cardAria2TransferError
                            ? cardAria2Error
                            : null;
                        const cardSettingsError =
                          cardRequiredError ??
                          cardProxyError ??
                          cardAria2Error ??
                          cardQbittorrentError;
                        const aria2RpcUrl =
                          draft.engine === "aria2"
                            ? aria2RpcInputUrl(draft.connectionUrl)
                            : null;

                        return (
                          <div
                            key={draft.id}
                            onDragOver={handleDragOver}
                            onDrop={(event) => handleDrop(event, draft.id)}
                            className={classNames(
                              "overflow-hidden rounded-xl border bg-white shadow-sm transition",
                              isDragging
                                ? "border-emerald-300 opacity-70"
                                : "border-slate-200",
                            )}
                          >
                            <div
                              className={classNames(
                                "flex items-center justify-between gap-3 border-l-[3px] px-4 py-3",
                                draft.enabled
                                  ? "border-l-emerald-500"
                                  : "border-l-slate-200",
                              )}
                            >
                              <div className="flex min-w-0 items-center gap-3">
                                <span
                                  title="拖拽调整优先级"
                                  draggable
                                  onDragStart={(event) =>
                                    handleDragStart(event, draft.id)
                                  }
                                  onDragEnd={() => setDraggedEngineId(null)}
                                  className="grid h-7 w-5 cursor-grab select-none place-items-center text-slate-300 hover:text-slate-500"
                                >
                                  <GripVertical size={14} />
                                </span>
                                <div className="grid h-7 w-7 shrink-0 place-items-center rounded-md border border-slate-200 bg-slate-50 text-xs font-semibold tabular-nums text-slate-700">
                                  {index + 1}
                                </div>
                                <div className="min-w-0">
                                  <div className="flex items-center gap-2">
                                    <span className="truncate text-sm font-semibold text-slate-950">
                                      {draft.name || engineLabels[draft.engine]}
                                    </span>
                                    <span className="shrink-0 rounded-md bg-slate-100 px-1.5 py-0.5 text-[10px] font-medium text-slate-600">
                                      {engineLabels[draft.engine]}
                                    </span>
                                  </div>
                                  <div className="mt-0.5 truncate text-xs text-slate-500">
                                    {draft.updatedAt
                                      ? `更新于 ${draft.updatedAt}`
                                      : "尚未保存"}
                                  </div>
                                </div>
                              </div>

                              <div className="flex items-center gap-2">
                                {savedEngineId === draft.id && (
                                  <InstalledBadge label="已保存" />
                                )}
                                {testedEngineId === draft.id && (
                                  <InstalledBadge label="连接成功" />
                                )}
                                <label
                                  className="relative inline-flex h-6 w-11 cursor-pointer items-center"
                                  title={draft.enabled ? "停用" : "启用"}
                                >
                                  <input
                                    type="checkbox"
                                    aria-label={`${draft.enabled ? "停用" : "启用"
                                      } ${draft.name || engineLabels[draft.engine]}`}
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
                                <span className="ml-1 hidden h-5 w-px bg-slate-200 sm:block" />
                                <SmallIconButton
                                  title="撤销改动"
                                  disabled={!dirty || isBusy}
                                  onClick={() => resetEngine(draft.id)}
                                >
                                  <RotateCcw size={15} />
                                </SmallIconButton>
                                <SmallIconButton
                                  title={
                                    cardSettingsError
                                      ? `请先修正设置：${cardSettingsError}`
                                      : "保存改动"
                                  }
                                  disabled={
                                    !dirty || isBusy || Boolean(cardSettingsError)
                                  }
                                  onClick={() => void saveEngine(draft.id)}
                                >
                                  <Save size={15} />
                                </SmallIconButton>
                              </div>
                            </div>

                            <div className="flex flex-wrap items-center gap-2 border-t border-slate-100 bg-slate-50/50 px-4 py-2.5">
                              <span className="text-xs font-medium text-slate-500">
                                支持来源
                              </span>
                              {supportedSourceTypes(draft.engine).map((sourceType) => {
                                const checked =
                                  draft.supportedSourceTypes.includes(sourceType);
                                return (
                                  <label
                                    key={sourceType}
                                    className={classNames(
                                      "inline-flex h-7 cursor-pointer items-center gap-1 rounded-full border px-2.5 text-xs transition",
                                      checked
                                        ? "border-emerald-300 bg-emerald-50 text-emerald-800"
                                        : "border-slate-200 bg-white text-slate-500 hover:bg-slate-50",
                                    )}
                                  >
                                    <input
                                      type="checkbox"
                                      checked={checked}
                                      onChange={(event) => {
                                        const nextSourceTypes = event.currentTarget
                                          .checked
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
                                      className="sr-only"
                                    />
                                    {checked && <Check size={11} />}
                                    <span>{sourceLabels[sourceType]}</span>
                                  </label>
                                );
                              })}
                            </div>

                            <div className="grid gap-4 px-4 py-4 md:grid-cols-2">
                              <Field
                                label="名称"
                                value={draft.name}
                                required
                                onChange={(value) =>
                                  updateDraft(draft.id, { name: value })
                                }
                              />
                              {usesExecutable && (
                                <IconField
                                  label="可执行文件"
                                  value={draft.executablePath ?? ""}
                                  required
                                  onChange={(value) =>
                                    updateDraft(draft.id, { executablePath: value })
                                  }
                                  onClick={() => void installLatest(draft.id)}
                                  buttonTitle="下载最新版"
                                  buttonDisabled={isInstalling || !canInstall || dirty}
                                  buttonLabel={<Download size={15} />}
                                />
                              )}
                              {draft.engine !== "qbittorrent" && (
                                <div className="md:col-span-2">
                                  <DirectoryField
                                    label="默认下载目录"
                                    value={draft.defaultDownloadDir}
                                    required
                                    onChange={(value) =>
                                      updateDraft(draft.id, { defaultDownloadDir: value })
                                    }
                                    onBrowse={() => {
                                      void (async () => {
                                        const selected = await pickDownloadDirectory(
                                          draft.defaultDownloadDir,
                                        );
                                        if (selected) {
                                          updateDraft(draft.id, {
                                            defaultDownloadDir: selected,
                                          });
                                        }
                                      })();
                                    }}
                                  />
                                </div>
                              )}
                              {usesConnection && (
                                <div className="flex flex-col gap-1.5 md:col-span-2">
                                  <IconField
                                    label="连接地址"
                                    value={draft.connectionUrl ?? ""}
                                    required
                                    onChange={(value) =>
                                      updateDraft(draft.id, { connectionUrl: value })
                                    }
                                    onClick={() => void testConnection(draft.id)}
                                    buttonTitle={isTesting ? "正在测试连接" : "测试连接"}
                                    buttonDisabled={isBusy}
                                    buttonLabel={<PlugZap size={15} />}
                                  />
                                  <span className="text-xs text-slate-500">
                                    远程引擎 Web/API 地址，点击右侧按钮可测试连通性。
                                  </span>
                                </div>
                              )}
                              {draft.engine === "qbittorrent" && (
                                <>
                                  <Field
                                    label="用户名"
                                    value={draft.username ?? ""}
                                    required
                                    onChange={(value) =>
                                      updateDraft(draft.id, { username: value })
                                    }
                                  />
                                  <Field
                                    label="密码"
                                    type="password"
                                    value={draft.password ?? ""}
                                    required
                                    onChange={(value) =>
                                      updateDraft(draft.id, { password: value })
                                    }
                                  />
                                  <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700 md:col-span-2">
                                    <span className="font-medium">连接代理</span>
                                    <div className="flex min-w-0 items-center gap-2">
                                      <Network
                                        size={15}
                                        className="shrink-0 text-slate-400"
                                      />
                                      <input
                                        value={draft.proxyUrl ?? ""}
                                        placeholder="留空表示直连，例如 http://127.0.0.1:7890 或 socks5h://127.0.0.1:1080"
                                        onChange={(event) =>
                                          updateDraft(draft.id, {
                                            proxyUrl: event.currentTarget.value,
                                          })
                                        }
                                        className={classNames(
                                          "h-9 min-w-0 flex-1 rounded-md border px-3 text-sm outline-none transition",
                                          cardProxyError
                                            ? "border-rose-400 bg-rose-50/40 text-rose-700 focus:border-rose-500 focus:ring-2 focus:ring-rose-100"
                                            : "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100",
                                        )}
                                      />
                                    </div>
                                    <span
                                      className={classNames(
                                        "text-xs",
                                        cardProxyError
                                          ? "text-rose-600"
                                          : "text-slate-500",
                                      )}
                                    >
                                      {cardProxyError ??
                                        "仅用于 UniDL 连接 qBittorrent Web API，不会修改 qBittorrent 的下载代理设置。支持 http / https / socks4 / socks4a / socks5 / socks5h。"}
                                    </span>
                                  </label>
                                  <div className="flex flex-col gap-1.5 md:col-span-2">
                                    <Field
                                      label="远程保存路径"
                                      value={draft.remotePath ?? ""}
                                      onChange={(value) =>
                                        updateDraft(draft.id, { remotePath: value })
                                      }
                                    />
                                    <span className="text-xs text-slate-500">
                                      qBittorrent
                                      任务只使用远程保存路径，不使用本机默认下载目录。
                                    </span>
                                  </div>
                                </>
                              )}
                              {(draft.engine === "aria2" || draft.engine === "yt-dlp") &&
                                (() => {
                                  const hint =
                                    draft.engine === "aria2"
                                      ? "aria2 任务通过 --all-proxy 走此代理，仅支持 http / https，不支持 SOCKS。留空则不使用代理。"
                                      : "yt-dlp 任务通过 --proxy 走此代理，支持 http / https / socks4 / socks4a / socks5 / socks5h。留空则不使用代理。";
                                  const placeholder =
                                    draft.engine === "aria2"
                                      ? "留空表示直连，例如 http://127.0.0.1:7890"
                                      : "留空表示直连，例如 http://127.0.0.1:7890 或 socks5h://127.0.0.1:1080";
                                  return (
                                    <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700 md:col-span-2">
                                      <span className="font-medium">任务代理</span>
                                      <div className="flex min-w-0 items-center gap-2">
                                        <Network
                                          size={15}
                                          className="shrink-0 text-slate-400"
                                        />
                                        <input
                                          value={draft.proxyUrl ?? ""}
                                          placeholder={placeholder}
                                          onChange={(event) =>
                                            updateDraft(draft.id, {
                                              proxyUrl: event.currentTarget.value,
                                            })
                                          }
                                          className={classNames(
                                            "h-9 min-w-0 flex-1 rounded-md border px-3 text-sm outline-none transition",
                                            cardProxyError
                                              ? "border-rose-400 bg-rose-50/40 text-rose-700 focus:border-rose-500 focus:ring-2 focus:ring-rose-100"
                                              : "border-slate-200 bg-white text-slate-900 focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100",
                                          )}
                                        />
                                      </div>
                                      <span
                                        className={classNames(
                                          "text-xs",
                                          cardProxyError
                                            ? "text-rose-600"
                                            : "text-slate-500",
                                        )}
                                      >
                                        {cardProxyError ?? hint}
                                      </span>
                                    </label>
                                  );
                                })()}
                            </div>

                            <div className="border-t border-slate-100">
                              <button
                                type="button"
                                onClick={() => toggleAdvanced(draft.id)}
                                className="flex w-full items-center justify-between gap-3 px-4 py-2.5 text-left text-sm text-slate-700 transition hover:bg-slate-50"
                              >
                                <span className="inline-flex items-center gap-1.5 font-medium">
                                  {isAdvancedOpen ? (
                                    <ChevronDown size={14} className="text-slate-400" />
                                  ) : (
                                    <ChevronRight size={14} className="text-slate-400" />
                                  )}
                                  高级设置
                                </span>
                                <span className="truncate text-xs text-slate-400">
                                  {[
                                    draft.engine === "aria2" ? "连接与分片" : null,
                                    draft.engine === "aria2" ? "BT 参数" : null,
                                    draft.engine === "aria2" ? "BT 发现" : null,
                                    draft.engine === "aria2" ? "RPC 监听" : null,
                                    draft.engine === "aria2" ? "Tracker 订阅" : null,
                                    draft.engine === "qbittorrent" ? "限速与做种" : null,
                                    draft.engine === "qbittorrent" ? "连接代理" : null,
                                    usesExecutable ? "User-Agent" : null,
                                    usesExecutable ? "限速" : null,
                                    "偏好域名",
                                    "默认参数",
                                    "删除引擎",
                                  ]
                                    .filter(Boolean)
                                    .join(" · ")}
                                </span>
                              </button>

                              {isAdvancedOpen && (
                                <div className="flex flex-col gap-4 border-t border-slate-100 bg-slate-50/40 px-4 py-4">
                                  {usesExecutable && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          User-Agent
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          作用于此引擎创建的新下载任务；留空使用引擎默认值。
                                        </div>
                                      </div>
                                      <Field
                                        label="User-Agent"
                                        value={draft.userAgent ?? ""}
                                        onChange={(value) =>
                                          updateDraft(draft.id, { userAgent: value })
                                        }
                                      />
                                    </div>
                                  )}

                                  {usesExecutable && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          下载限速
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          作用于此引擎创建的新下载任务；填 0 表示不限速。
                                        </div>
                                      </div>
                                      <NumberField
                                        label="下载限速（MB/s）"
                                        value={speedLimitMegabytes(
                                          draft.speedLimitBytesPerSec,
                                        )}
                                        min={0}
                                        step={0.1}
                                        onChange={(value) =>
                                          updateDraft(draft.id, {
                                            speedLimitBytesPerSec:
                                              speedLimitBytesFromMegabytes(value),
                                          })
                                        }
                                      />
                                    </div>
                                  )}

                                  {draft.engine === "qbittorrent" && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          限速与做种
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          作用于 UniDL 创建的新 qBittorrent 任务；填 0
                                          表示不覆盖 qBittorrent 端默认值。
                                        </div>
                                      </div>
                                      <div className="grid gap-3 sm:grid-cols-2">
                                        <NumberField
                                          label="下载限速（MB/s）"
                                          value={speedLimitMegabytes(
                                            draft.qbittorrentDownloadLimitBytesPerSec,
                                          )}
                                          min={0}
                                          step={0.1}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              qbittorrentDownloadLimitBytesPerSec:
                                                speedLimitBytesFromMegabytes(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="上传限速（MB/s）"
                                          value={speedLimitMegabytes(
                                            draft.qbittorrentUploadLimitBytesPerSec,
                                          )}
                                          min={0}
                                          step={0.1}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              qbittorrentUploadLimitBytesPerSec:
                                                speedLimitBytesFromMegabytes(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="分享率限制（%）"
                                          value={
                                            Number.isFinite(
                                              draft.qbittorrentSeedRatioLimit,
                                            )
                                              ? draft.qbittorrentSeedRatioLimit * 100
                                              : Number.NaN
                                          }
                                          min={0}
                                          step={1}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              qbittorrentSeedRatioLimit: value / 100,
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="做种时间限制（分钟）"
                                          value={draft.qbittorrentSeedTimeLimitMinutes}
                                          min={0}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              qbittorrentSeedTimeLimitMinutes:
                                                Math.trunc(value),
                                            })
                                          }
                                        />
                                      </div>
                                      {cardQbittorrentError && (
                                        <div className="text-xs text-rose-600">
                                          {cardQbittorrentError}
                                        </div>
                                      )}
                                    </div>
                                  )}

                                  {draft.engine === "aria2" && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          连接与分片
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          配置 aria2 的
                                          max-connection-per-server、split、min-split-size
                                          和 file-allocation。
                                        </div>
                                      </div>
                                      <div className="grid gap-3 sm:grid-cols-2">
                                        <NumberField
                                          label="单服务器最大连接数"
                                          value={draft.aria2MaxConnectionPerServer}
                                          min={1}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2MaxConnectionPerServer:
                                                Math.trunc(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="分片数"
                                          value={draft.aria2Split}
                                          min={1}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2Split: Math.trunc(value),
                                            })
                                          }
                                        />
                                        <Field
                                          label="最小分片大小"
                                          value={draft.aria2MinSplitSize}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2MinSplitSize: value,
                                            })
                                          }
                                        />
                                        <label className="flex min-w-0 flex-col gap-1.5 text-sm text-slate-700">
                                          <span className="font-medium">文件预分配</span>
                                          <select
                                            value={draft.aria2FileAllocation}
                                            onChange={(event) =>
                                              updateDraft(draft.id, {
                                                aria2FileAllocation:
                                                  event.currentTarget.value,
                                              })
                                            }
                                            className="h-9 w-full rounded-md border border-slate-200 bg-white px-3 text-sm text-slate-900 outline-none transition focus:border-emerald-600 focus:ring-2 focus:ring-emerald-100"
                                          >
                                            {aria2FileAllocationOptions.map((option) => (
                                              <option key={option} value={option}>
                                                {option}
                                              </option>
                                            ))}
                                          </select>
                                        </label>
                                      </div>
                                      {cardAria2TransferError && (
                                        <div className="text-xs text-rose-600">
                                          {cardAria2TransferError}
                                        </div>
                                      )}
                                    </div>
                                  )}

                                  {draft.engine === "aria2" &&
                                    (() => {
                                      const options: Array<{
                                        key: Aria2BtToggleKey;
                                        label: string;
                                        description: string;
                                        checked: boolean;
                                      }> = [
                                          {
                                            key: "aria2EnableDht",
                                            label: "启用 DHT",
                                            description: "对应 aria2 的 enable-dht",
                                            checked: draft.aria2EnableDht,
                                          },
                                          {
                                            key: "aria2EnableDht6",
                                            label: "启用 IPv6 DHT",
                                            description: "对应 aria2 的 enable-dht6",
                                            checked: draft.aria2EnableDht6,
                                          },
                                          {
                                            key: "aria2EnablePeerExchange",
                                            label: "启用 PeX 节点交换",
                                            description:
                                              "对应 aria2 的 enable-peer-exchange",
                                            checked: draft.aria2EnablePeerExchange,
                                          },
                                          {
                                            key: "aria2EnableLpd",
                                            label: "启用本地端点发现",
                                            description: "对应 aria2 的 bt-enable-lpd",
                                            checked: draft.aria2EnableLpd,
                                          },
                                        ];

                                      return (
                                        <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                          <div>
                                            <div className="text-sm font-medium text-slate-800">
                                              BT/磁链发现
                                            </div>
                                            <div className="mt-1 text-xs text-slate-500">
                                              仅提供 aria2 支持的 DHT、IPv6 DHT、PeX
                                              和本地端点发现开关；uTP、UPnP/NAT-PMP 不受
                                              aria2 官方开关支持。
                                            </div>
                                          </div>
                                          <div className="grid gap-2 sm:grid-cols-2">
                                            {options.map((option) => (
                                              <label
                                                key={option.key}
                                                className="flex cursor-pointer items-start gap-2 rounded-md border border-slate-200 bg-slate-50/50 px-3 py-2 text-sm text-slate-700 transition hover:bg-slate-50"
                                              >
                                                <input
                                                  type="checkbox"
                                                  checked={option.checked}
                                                  onChange={(event) =>
                                                    updateDraft(
                                                      draft.id,
                                                      aria2BtTogglePatch(
                                                        option.key,
                                                        event.currentTarget.checked,
                                                      ),
                                                    )
                                                  }
                                                  className="mt-0.5 h-4 w-4 rounded border-slate-300 text-emerald-600 focus:ring-emerald-100"
                                                />
                                                <span className="min-w-0">
                                                  <span className="block font-medium text-slate-800">
                                                    {option.label}
                                                  </span>
                                                  <span className="mt-0.5 block text-xs text-slate-500">
                                                    {option.description}
                                                  </span>
                                                </span>
                                              </label>
                                            ))}
                                          </div>
                                        </div>
                                      );
                                    })()}

                                  {draft.engine === "aria2" && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          RPC 监听
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          修改监听端口会同步更新此引擎连接地址中的端口；多个
                                          aria2 引擎必须使用不同 RPC 端口。
                                        </div>
                                      </div>
                                      <IconField
                                        label="连接地址"
                                        value={draft.connectionUrl ?? ""}
                                        onChange={(value) =>
                                          updateDraft(draft.id, { connectionUrl: value })
                                        }
                                        onClick={() => void testConnection(draft.id)}
                                        buttonTitle={
                                          isTesting ? "正在测试连接" : "测试连接"
                                        }
                                        buttonDisabled={isBusy}
                                        buttonLabel={<PlugZap size={15} />}
                                      />
                                      <div className="text-xs text-slate-500">
                                        aria2 JSON-RPC 地址，修改后会影响 RPC
                                        监听地址和端口。
                                      </div>
                                      <div className="grid gap-3 sm:grid-cols-2">
                                        <Field
                                          label="RPC 监听地址"
                                          value={
                                            aria2RpcUrl
                                              ? aria2RpcListenAddress(draft.connectionUrl)
                                              : ""
                                          }
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              connectionUrl: updateAria2RpcListenAddress(
                                                draft.connectionUrl,
                                                value,
                                              ),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="RPC 监听端口"
                                          value={aria2RpcInputPort(draft.connectionUrl)}
                                          min={1}
                                          max={65_535}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              connectionUrl: updateAria2RpcListenPort(
                                                draft.connectionUrl,
                                                value,
                                              ),
                                            })
                                          }
                                        />
                                      </div>
                                      {cardAria2RpcError && (
                                        <div className="text-xs text-rose-600">
                                          {cardAria2RpcError}
                                        </div>
                                      )}
                                    </div>
                                  )}

                                  {draft.engine === "aria2" && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div>
                                        <div className="text-sm font-medium text-slate-800">
                                          BT 参数
                                        </div>
                                        <div className="mt-1 text-xs text-slate-500">
                                          控制 aria2 BT 下载监听端口、最大 Peer
                                          数量以及下载完成后的做种策略。
                                        </div>
                                      </div>
                                      <div className="grid gap-3 sm:grid-cols-2">
                                        <NumberField
                                          label="BT 下载监听端口"
                                          value={draft.aria2BtListenPort}
                                          min={1}
                                          max={65_535}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2BtListenPort: Math.trunc(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="BT 下载最大 Peer 数量"
                                          value={draft.aria2BtMaxPeers}
                                          min={0}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2BtMaxPeers: Math.trunc(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="完成后持续做种时间（分钟）"
                                          value={draft.aria2SeedTime}
                                          min={0}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2SeedTime: Math.trunc(value),
                                            })
                                          }
                                        />
                                        <NumberField
                                          label="完成后持续做种分享率（%）"
                                          value={
                                            Number.isFinite(draft.aria2SeedRatio)
                                              ? draft.aria2SeedRatio * 100
                                              : Number.NaN
                                          }
                                          min={0}
                                          onChange={(value) =>
                                            updateDraft(draft.id, {
                                              aria2SeedRatio: value / 100,
                                            })
                                          }
                                        />
                                      </div>
                                      {cardAria2BtError && (
                                        <div className="text-xs text-rose-600">
                                          {cardAria2BtError}
                                        </div>
                                      )}
                                    </div>
                                  )}

                                  {draft.engine === "aria2" && (
                                    <div className="grid gap-3 rounded-md border border-slate-200 bg-white p-3">
                                      <div className="flex flex-wrap items-center justify-between gap-2">
                                        <div>
                                          <div className="text-sm font-medium text-slate-800">
                                            Tracker 订阅
                                          </div>
                                          <div className="mt-1 text-xs text-slate-500">
                                            已保存 {draft.trackers.length} 个
                                            tracker，磁链任务会自动追加。
                                          </div>
                                        </div>
                                        <button
                                          type="button"
                                          disabled={isUpdatingTrackers || dirty || !saved}
                                          onClick={() => void updateTrackers(draft.id)}
                                          className={classNames(
                                            "inline-flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs font-medium transition",
                                            isUpdatingTrackers || dirty || !saved
                                              ? "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400"
                                              : "border-emerald-200 bg-emerald-50 text-emerald-800 hover:bg-emerald-100",
                                          )}
                                        >
                                          <Download size={14} />
                                          {isUpdatingTrackers ? "更新中" : "自动更新"}
                                        </button>
                                      </div>
                                      <TextAreaField
                                        label="订阅地址（每行一个）"
                                        value={draft.trackerSubscriptionUrl ?? ""}
                                        onChange={(value) =>
                                          updateDraft(draft.id, {
                                            trackerSubscriptionUrl: value,
                                          })
                                        }
                                      />
                                      <TextAreaField
                                        label="Tracker 列表（每行一个）"
                                        value={trackersText(draft.trackers)}
                                        onChange={(value) =>
                                          updateDraft(draft.id, {
                                            trackers: parseTrackers(value),
                                          })
                                        }
                                      />
                                      <div className="text-xs text-slate-500">
                                        可添加多个 GitHub raw
                                        或纯文本订阅源，更新时会合并并自动去重。
                                      </div>
                                    </div>
                                  )}

                                  <TextAreaField
                                    label="偏好域名（每行一个）"
                                    value={preferredDomainsText(draft.preferredDomains)}
                                    onChange={(value) =>
                                      updateDraft(draft.id, {
                                        preferredDomains: parsePreferredDomains(value),
                                      })
                                    }
                                  />

                                  <TextAreaField
                                    label="默认参数"
                                    value={draft.defaultArgs}
                                    onChange={(value) =>
                                      updateDraft(draft.id, { defaultArgs: value })
                                    }
                                  />
                                  {draft.engine === "aria2" && (
                                    <div className="text-xs text-slate-500">
                                      UniDL 仍内置 aria2
                                      断点续传参数；连接与分片参数请优先使用上方专用设置。
                                    </div>
                                  )}

                                  <div className="flex items-center justify-between gap-3 rounded-md border border-rose-100 bg-rose-50/40 px-3 py-2.5">
                                    <div className="min-w-0">
                                      <div className="text-sm font-medium text-rose-800">
                                        删除此下载引擎
                                      </div>
                                      <div className="mt-0.5 text-xs text-rose-600/80">
                                        操作不可撤销，将移除该引擎的全部配置。
                                      </div>
                                    </div>
                                    <button
                                      type="button"
                                      disabled={isBusy}
                                      onClick={() => void deleteEngine(draft.id)}
                                      className={classNames(
                                        "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border px-3 text-xs font-medium transition",
                                        isBusy
                                          ? "cursor-not-allowed border-slate-200 bg-slate-100 text-slate-400"
                                          : "border-rose-200 bg-white text-rose-700 hover:bg-rose-50",
                                      )}
                                    >
                                      <Trash2 size={13} />
                                      删除
                                    </button>
                                  </div>
                                </div>
                              )}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
