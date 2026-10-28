// REST helpers. A token (headless mode with --token) is read from ?token= once
// and kept in sessionStorage.

const tokenKey = "keelwire.token";

export function getToken(): string | null {
  try {
    const q = new URLSearchParams(location.search).get("token");
    if (q) sessionStorage.setItem(tokenKey, q);
    return sessionStorage.getItem(tokenKey);
  } catch {
    return null;
  }
}

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
  }
}

export async function api<T = unknown>(path: string, init: RequestInit & { json?: unknown } = {}): Promise<T> {
  const headers: Record<string, string> = { ...(init.headers as Record<string, string>) };
  const t = getToken();
  if (t) headers["Authorization"] = `Bearer ${t}`;
  let body = init.body;
  if (init.json !== undefined) {
    headers["Content-Type"] = "application/json";
    body = JSON.stringify(init.json);
  }
  const r = await fetch(`/api/v1${path}`, { ...init, headers, body });
  const text = await r.text();
  const data = text ? safeJson(text) : null;
  if (!r.ok) {
    const d = (data ?? {}) as { error?: string; message?: string };
    throw new ApiError(r.status, d.error ?? "error", d.message ?? `${r.status} ${r.statusText}`);
  }
  return data as T;
}

function safeJson(t: string): unknown {
  try {
    return JSON.parse(t);
  } catch {
    return t;
  }
}

export const post = <T = unknown>(path: string, json?: unknown) => api<T>(path, { method: "POST", json: json ?? {} });
export const patch = <T = unknown>(path: string, json: unknown) => api<T>(path, { method: "PATCH", json });
export const put = <T = unknown>(path: string, json: unknown) => api<T>(path, { method: "PUT", json });
export const del = <T = unknown>(path: string) => api<T>(path, { method: "DELETE" });

export function downloadUrl(path: string): string {
  const t = getToken();
  return `/api/v1${path}${t ? (path.includes("?") ? "&" : "?") + "token=" + encodeURIComponent(t) : ""}`;
}

export function wsUrl(): string {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const t = getToken();
  return `${proto}://${location.host}/ws${t ? "?token=" + encodeURIComponent(t) : ""}`;
}
