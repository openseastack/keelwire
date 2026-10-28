// Service sessions (SPEC §11.5): state polled from /session, actions, helpers.
import { create } from "zustand";
import { api, del, patch, post } from "./api";
import { useApp } from "./store";

export interface Vessel {
  mmsi: number | null;
  name: string | null;
  callsign: string | null;
  hin: string | null;
  ship_type: string | null;
  length_m: number | null;
  beam_m: number | null;
  ais_class: string | null;
  source: "ais" | "manual" | "none";
  confirmed: boolean;
}

export interface Change {
  ts_ms: number;
  device_key: string | null;
  device: string;
  serial: string | null;
  setting: string;
  before: string | null;
  after: string | null;
  method: string;
  result: "confirmed" | "sent" | "failed";
  error: string | null;
  kind: "setting" | "control" | "raw";
  plain: string | null;
}

export interface SessionEvent {
  ts_ms: number;
  kind: string;
  text: string;
}

export interface Note {
  ts_ms: number;
  text: string;
}

export interface Pause {
  from_ms: number;
  to_ms: number | null;
  reason: string;
}

export interface FindingSnap {
  sig: string;
  severity: string;
  message: string;
  suggestion: string | null;
  devices: string[];
  first_ms: number;
  last_ms: number;
}

export interface Session {
  id: string;
  started_ms: number;
  ended_ms: number | null;
  technician: string | null;
  vessel: Vessel;
  position: { lat: number; lon: number; source: string } | null;
  gateway: string | null;
  paused: Pause | null;
  pauses: Pause[];
  ask_resume: boolean;
  changes: Change[];
  notes: Note[];
  events: SessionEvent[];
  findings: FindingSnap[];
  findings_open: string[];
  capture_files: string[];
  visit_number: number;
  previous: { dir: string; started_ms: number; ended_ms: number | null } | null;
  other_ships: number;
  since_last?: { kind: string; text: string; plain: string }[];
}

export interface AisTarget {
  mmsi: number;
  name: string | null;
  callsign: string | null;
  ship_type: string | null;
  length_m: number | null;
  beam_m: number | null;
  class: string | null;
  own_reports: number;
}

export interface SessionStatus {
  state: "none" | "recording" | "paused";
  reason?: string | null;
  now_ms: number;
  active_ms?: number;
  bytes?: number;
  frames?: number;
  session?: Session;
  changes_ok?: number;
  fixed?: number;
  open?: number;
  technician: string | null;
  ais_own: AisTarget | null;
  other_ships: number;
  sessions_dir: string;
}

export interface EndOptions {
  owner_pdf: boolean;
  technician: "pdf" | "html" | null;
  bundle: boolean;
  strip_other_ais: boolean;
}

export interface EndResult {
  dir: string;
  rel: string;
  files: string[];
}

interface SessionStore {
  status: SessionStatus | null;
  /** Local clock when `status` arrived, to tick the timer between polls. */
  at: number;
  endOpen: boolean;
  lastEnd: EndResult | null;
  setEndOpen: (o: boolean) => void;
  refresh: () => Promise<void>;
  run: (p: Promise<SessionStatus>, ok?: string) => Promise<void>;
}

export const useSession = create<SessionStore>((set, get) => ({
  status: null,
  at: Date.now(),
  endOpen: false,
  lastEnd: null,
  setEndOpen: (endOpen) => set({ endOpen }),
  refresh: async () => {
    try {
      const s = await api<SessionStatus>("/session");
      set({ status: s, at: Date.now() });
    } catch {
      /* server unreachable: keep the last state */
    }
  },
  run: async (p, ok) => {
    try {
      const s = await p;
      set({ status: s, at: Date.now() });
      if (ok) useApp.getState().toast("ok", ok);
    } catch (e) {
      useApp.getState().toast("error", (e as Error).message);
      get().refresh();
    }
  },
}));

let timer: ReturnType<typeof setInterval> | null = null;
export function startSessionPolling() {
  if (timer) return;
  useSession.getState().refresh();
  timer = setInterval(() => useSession.getState().refresh(), 2000);
}

export const sessionApi = {
  start: () => useSession.getState().run(post<SessionStatus>("/session/start"), "Session started"),
  pause: () => useSession.getState().run(post<SessionStatus>("/session/pause")),
  resume: () => useSession.getState().run(post<SessionStatus>("/session/resume")),
  startNew: () => useSession.getState().run(post<SessionStatus>("/session/new"), "Previous session saved; new session started"),
  update: (p: { name?: string; hin?: string; technician?: string; confirm?: boolean; manual?: boolean }) => useSession.getState().run(patch<SessionStatus>("/session", p)),
  note: (text: string) => useSession.getState().run(post<SessionStatus>("/session/notes", { text })),
  deleteNote: (ts: number) => useSession.getState().run(del<SessionStatus>(`/session/notes/${ts}`)),
  end: async (o: EndOptions): Promise<EndResult | null> => {
    try {
      const r = await post<EndResult>("/session/end", o);
      useSession.setState({ lastEnd: r });
      await useSession.getState().refresh();
      return r;
    } catch (e) {
      useApp.getState().toast("error", (e as Error).message);
      return null;
    }
  },
};

/** Elapsed recording time, ticking locally between polls. */
export function liveActive(s: SessionStatus | null, at: number, now: number): number {
  if (!s || s.state === "none") return 0;
  return (s.active_ms ?? 0) + (s.state === "recording" ? Math.max(0, now - at) : 0);
}

export function fmtClock(ms: number): string {
  const t = Math.floor(ms / 1000);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;
  return `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}

export function fmtTime(ms: number, seconds = false): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", ...(seconds ? { second: "2-digit" } : {}), hour12: false });
}

export function fmtDate(ms: number, long = false): string {
  return new Date(ms).toLocaleDateString([], long ? { weekday: "short", day: "numeric", month: "short", year: "numeric" } : { day: "numeric", month: "short", year: "numeric" });
}

export function fmtBytes(b: number): string {
  if (b >= 1e9) return `${(b / 1e9).toFixed(1)} GB`;
  if (b >= 1e6) return `${(b / 1e6).toFixed(b >= 1e8 ? 0 : 1)} MB`;
  return `${Math.max(1, Math.round(b / 1e3))} kB`;
}

export function fmtLatLon(lat: number, lon: number): string {
  const f = (v: number, p: string, n: string, w: number) => {
    const a = Math.abs(v);
    const d = Math.floor(a);
    const m = (a - d) * 60;
    return `${String(d).padStart(w, "0")}°${m.toFixed(2).padStart(5, "0")}′${v >= 0 ? p : n}`;
  };
  return `${f(lat, "N", "S", 2)} ${f(lon, "E", "W", 3)}`;
}

export function vesselName(v: Vessel | null | undefined): string {
  return v?.name?.trim() || "Unnamed boat";
}

export function historyFileUrl(path: string, inline = false): string {
  const t = sessionStorage.getItem("keelwire.token");
  return `/api/v1/history/file?path=${encodeURIComponent(path)}${inline ? "&inline=1" : ""}${t ? `&token=${encodeURIComponent(t)}` : ""}`;
}

export interface TimelineItem {
  ts: number;
  kind: "event" | "change" | "note";
  sub: string;
  text: string;
  change?: Change;
  note?: Note;
}

export function timeline(s: Session): TimelineItem[] {
  const items: TimelineItem[] = [
    ...s.events.map((e) => ({ ts: e.ts_ms, kind: "event" as const, sub: e.kind, text: e.text })),
    ...s.changes.map((c) => ({ ts: c.ts_ms, kind: "change" as const, sub: c.kind, text: c.setting, change: c })),
    ...s.notes.map((n) => ({ ts: n.ts_ms, kind: "note" as const, sub: "note", text: n.text, note: n })),
  ];
  return items.sort((a, b) => a.ts - b.ts);
}
