const DEFAULT_SETTINGS = {
  apiBaseUrl: "http://127.0.0.1:18080",
  interceptEnabled: false,
  cancelOriginal: true,
  minCaptureSizeMb: 3,
  skipCaptureDomains: [],
  lastEvent: "",
};

function parseSource(value, suggestedFileName) {
  const source = String(value ?? "").trim();
  if (!source) {
    throw new Error("Download source is required");
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
      fileName: parseNetworkFileName(source, suggestedFileName) ?? "http-download",
    };
  }
  if (/^ftp:\/\//i.test(source)) {
    return {
      sourceType: "ftp",
      source,
      fileName: parseNetworkFileName(source, suggestedFileName) ?? "ftp-download",
    };
  }
  if (source.toLowerCase().split(/[?#]/)[0].endsWith(".torrent")) {
    return {
      sourceType: "torrent",
      source,
      fileName:
        cleanFileName(suggestedFileName) ?? parsePathName(source) ?? "download.torrent",
    };
  }
  throw new Error("Unsupported download source");
}

function parseMagnetName(value) {
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

function parseUrlName(value) {
  try {
    const url = new URL(value);
    return cleanFileName(url.pathname.split("/").filter(Boolean).at(-1));
  } catch {
    return parsePathName(value);
  }
}

function parseNetworkFileName(source, suggestedFileName) {
  const parsedUrl = new URL(source);
  const suggested = cleanFileName(suggestedFileName);
  const hostname = parsedUrl.hostname.toLowerCase();
  if (suggested && suggested.toLowerCase() !== hostname) {
    return suggested;
  }

  return cleanFileName(parsedUrl.pathname.split("/").filter(Boolean).at(-1));
}

function parsePathName(value) {
  return cleanFileName(
    String(value).split(/[?#]/)[0].split(/[\\/]/).filter(Boolean).at(-1),
  );
}

function cleanFileName(value) {
  const name = String(value ?? "")
    .split(/[\\/]/)
    .filter(Boolean)
    .at(-1)
    ?.trim();
  if (!name) {
    return null;
  }
  try {
    return decodeURIComponent(name);
  } catch {
    return name;
  }
}

function normalizeHttpUrl(value) {
  const source = String(value ?? "").trim();
  return /^https?:\/\//i.test(source) ? source : null;
}

function normalizeSettings(settings) {
  const next = { ...DEFAULT_SETTINGS, ...settings };
  next.apiBaseUrl = trimBaseUrl(next.apiBaseUrl);
  next.interceptEnabled = Boolean(next.interceptEnabled);
  next.cancelOriginal = Boolean(next.cancelOriginal);
  const minCaptureSizeMb = Number(next.minCaptureSizeMb);
  next.minCaptureSizeMb =
    Number.isFinite(minCaptureSizeMb) && minCaptureSizeMb >= 0
      ? minCaptureSizeMb
      : DEFAULT_SETTINGS.minCaptureSizeMb;
  next.skipCaptureDomains = normalizeDomainList(next.skipCaptureDomains);
  next.lastEvent = String(next.lastEvent ?? "");
  return next;
}

function normalizeDomainList(value) {
  const entries = Array.isArray(value) ? value : String(value ?? "").split(/[\n,]/);
  const domains = [];
  for (const entry of entries) {
    const domain = normalizeDomainEntry(entry);
    if (domain && !domains.includes(domain)) {
      domains.push(domain);
    }
  }
  return domains;
}

function normalizeDomainEntry(value) {
  const text = String(value ?? "")
    .trim()
    .toLowerCase();
  if (!text) {
    return null;
  }
  try {
    const url = new URL(/^\w+:\/\//.test(text) ? text : "http://" + text);
    return trimDomain(url.hostname);
  } catch {
    return trimDomain(text.split(/[/?#]/)[0]);
  }
}

function trimDomain(value) {
  const domain = String(value ?? "")
    .trim()
    .replace(/^\*\./, "")
    .replace(/^\.+|\.+$/g, "");
  return domain || null;
}

function trimBaseUrl(value) {
  let text = String(value ?? "").trim();
  while (text.endsWith("/")) {
    text = text.slice(0, -1);
  }
  if (!text) {
    throw new Error("UniDL API URL is required");
  }
  const url = new URL(text);
  if (url.hostname === "localhost") {
    url.hostname = "127.0.0.1";
  }
  text = url.toString();
  while (text.endsWith("/")) {
    text = text.slice(0, -1);
  }
  return text;
}
