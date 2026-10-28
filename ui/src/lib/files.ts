// Saving and sharing files. In a browser or the desktop app a file is
// downloaded; on iPhone/iPad (where an app's web view cannot download) the
// app saves it and opens the system share sheet instead (docs/IOS-SPEC.md §6).
import { api, downloadUrl, getToken } from "./api";
import { useApp } from "./store";

/** The host app offers a share sheet (iPhone / iPad). */
export function canShare(): boolean {
  return !!useApp.getState().status?.can_share;
}

/** Phone or tablet app (iOS / Android). */
export function isMobileApp(): boolean {
  const p = useApp.getState().status?.platform;
  return p === "ios" || p === "android";
}

function report(e: unknown) {
  useApp.getState().toast("error", (e as Error).message);
}

/** Save a file made in the interface (CSV, settings). */
export async function saveBlob(name: string, blob: Blob) {
  if (canShare()) {
    try {
      const t = getToken();
      const r = await fetch(`/api/v1/native/save?name=${encodeURIComponent(name)}`, { method: "POST", headers: t ? { Authorization: `Bearer ${t}` } : {}, body: blob });
      if (!r.ok) throw new Error((await r.json().catch(() => ({}))).message ?? `${r.status}`);
    } catch (e) {
      report(e);
    }
    return;
  }
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
}

/** Save a file the server makes (boat file, capture): `path` is an API path. */
export async function saveFromApi(path: string, name: string) {
  if (canShare()) {
    try {
      const t = getToken();
      const r = await fetch(`/api/v1${path}`, { headers: t ? { Authorization: `Bearer ${t}` } : {} });
      if (!r.ok) throw new Error(`${r.status} ${r.statusText}`);
      await saveBlob(name, await r.blob());
    } catch (e) {
      report(e);
    }
    return;
  }
  const a = document.createElement("a");
  a.href = downloadUrl(path);
  a.download = name;
  a.click();
}

/** Share saved session files ("<boat>/<session>/<file>") or download one. */
export async function shareHistory(paths: string[]) {
  if (canShare()) {
    try {
      await api("/native/share", { method: "POST", json: { paths } });
    } catch (e) {
      report(e);
    }
    return;
  }
  for (const p of paths) {
    const a = document.createElement("a");
    a.href = downloadUrl(`/history/file?path=${encodeURIComponent(p)}`);
    a.download = p.split("/").pop() ?? "file";
    a.click();
  }
}

/** Share a capture file (Settings → Recordings). */
export async function shareCapture(file: string) {
  if (canShare()) {
    try {
      await api("/native/share", { method: "POST", json: { captures: [file] } });
    } catch (e) {
      report(e);
    }
    return;
  }
  const a = document.createElement("a");
  a.href = downloadUrl(`/captures/${encodeURIComponent(file)}`);
  a.download = file;
  a.click();
}
