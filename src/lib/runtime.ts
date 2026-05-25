export function hasTauriRuntime() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function isWebRuntime() {
  return !hasTauriRuntime();
}

const WEB_TOKEN_KEY = "unidl.web.token";

export function getWebToken() {
  if (typeof window === "undefined") {
    return null;
  }

  return window.localStorage.getItem(WEB_TOKEN_KEY);
}

export function setWebToken(token: string) {
  if (typeof window === "undefined") {
    throw new Error("web token storage is unavailable");
  }

  window.localStorage.setItem(WEB_TOKEN_KEY, token);
}

export function clearWebToken() {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.removeItem(WEB_TOKEN_KEY);
}

async function parseWebError(response: Response) {
  try {
    const payload = (await response.json()) as { error?: string };
    if (payload.error) {
      return payload.error;
    }
  } catch {
    return `${response.status} ${response.statusText}`.trim();
  }

  return `${response.status} ${response.statusText}`.trim();
}

export async function webRequest(path: string, init?: RequestInit) {
  const headers = new Headers(init?.headers ?? {});
  const token = getWebToken();
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
  }

  const response = await fetch(path, { ...init, headers });
  if (response.status === 401) {
    clearWebToken();
  }
  if (!response.ok) {
    throw new Error(await parseWebError(response));
  }

  return response;
}

export async function webJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await webRequest(path, init);
  if (response.status === 204) {
    return undefined as T;
  }
  return (await response.json()) as T;
}

export async function webLogin(password: string) {
  const response = await fetch("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ password }),
  });

  if (!response.ok) {
    throw new Error(await parseWebError(response));
  }

  const payload = (await response.json()) as { token: string };
  setWebToken(payload.token);
  return payload.token;
}
