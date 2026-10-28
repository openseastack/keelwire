import { create } from "zustand";
import { wsUrl } from "./api";
import type { Alert, Bank, Device, Metrics, Packet, Status } from "./types";

export type Theme = "day" | "dark" | "night";
export type PanelTab = "overview" | "settings" | "messages" | "identity";
export type Page = "overview" | "session" | "devices" | "sniffer" | "switching" | "boats" | "settings" | "connect";

/** "Latest per message" aggregate for the Traffic view. */
export interface MsgAgg {
  key: string;
  pgn: number;
  src: number;
  name: string;
  count: number;
  first: number;
  last: Packet;
  /** recent timestamps (µs) for the rate */
  times: number[];
}

const MAX_PACKETS = 20000;

export function load<T>(k: string, d: T): T {
  try {
    const v = localStorage.getItem(k);
    return v ? (JSON.parse(v) as T) : d;
  } catch {
    return d;
  }
}
export function save(k: string, v: unknown) {
  try {
    localStorage.setItem(k, JSON.stringify(v));
  } catch {
    /* storage unavailable */
  }
}

interface Toast {
  id: number;
  kind: "ok" | "error" | "info";
  text: string;
}

interface AppState {
  /** Per-source messages/s, last 30 samples (device sparklines). */
  srcHist: Record<number, number[]>;
  /** Per (pgn, src) aggregates since the view was cleared. */
  latest: Record<string, MsgAgg>;
  /** Messages per second per source, last 60 s, for the Traffic timeline. */
  timeline: { t: number; bySrc: Record<number, number> }[];
  connected: boolean;
  status: Status | null;
  devices: Device[];
  metrics: Metrics | null;
  banks: Bank[];
  alerts: Alert[];
  packets: Packet[];
  dropped: number;
  paused: boolean;
  filter: string;
  filterError: string | null;
  theme: Theme;
  page: Page;
  selectedDevice: string | null;
  configDevice: string | null;
  panelTab: PanelTab;
  setPanelTab: (t: PanelTab) => void;
  /** Device keys to highlight (instancing analysis). */
  highlight: string[];
  toasts: Toast[];
  setTheme: (t: Theme) => void;
  setPage: (p: Page) => void;
  selectDevice: (k: string | null) => void;
  openConfig: (k: string | null) => void;
  setHighlight: (keys: string[]) => void;
  setPaused: (p: boolean) => void;
  setFilter: (f: string) => void;
  clearPackets: () => void;
  toast: (kind: Toast["kind"], text: string) => void;
  dismiss: (id: number) => void;
}

let socket: WebSocket | null = null;
let toastId = 0;

export const useApp = create<AppState>((set, get) => ({
  connected: false,
  status: null,
  devices: [],
  metrics: null,
  banks: [],
  alerts: [],
  packets: [],
  dropped: 0,
  paused: false,
  filter: load("keelwire.filter", ""),
  filterError: null,
  srcHist: {},
  latest: {},
  timeline: [],
  theme: ((t: string) => (t === "dusk" ? "dark" : t) as Theme)(load<string>("keelwire.theme", typeof matchMedia !== "undefined" && matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "day")),
  page: ((p: string) => (p === "diagnostics" ? "overview" : p) as Page)(load<string>("keelwire.page", "overview")),
  selectedDevice: null,
  configDevice: null,
  highlight: [],
  toasts: [],
  setTheme: (theme) => {
    save("keelwire.theme", theme);
    document.documentElement.dataset.theme = theme;
    set({ theme });
  },
  setPage: (page) => {
    save("keelwire.page", page);
    set({ page });
  },
  selectDevice: (selectedDevice) => set({ selectedDevice }),
  // "Configure" now opens the device panel on its Settings tab.
  openConfig: (k) => (k ? set({ selectedDevice: k, page: "devices", panelTab: "settings" }) : set({ configDevice: null })),
  panelTab: "overview",
  setPanelTab: (panelTab) => set({ panelTab }),
  setHighlight: (highlight) => set({ highlight }),
  setPaused: (paused) => set({ paused }),
  setFilter: (filter) => {
    save("keelwire.filter", filter);
    set({ filter, filterError: null, packets: [] });
    subscribePackets(filter);
  },
  clearPackets: () => set({ packets: [], dropped: 0, latest: {}, timeline: [] }),
  toast: (kind, text) => {
    const id = ++toastId;
    set({ toasts: [...get().toasts, { id, kind, text }] });
    setTimeout(() => get().dismiss(id), kind === "error" ? 8000 : 4000);
  },
  dismiss: (id) => set({ toasts: get().toasts.filter((t) => t.id !== id) }),
}));

function send(o: unknown) {
  if (socket && socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify(o));
}

function subscribePackets(filter: string) {
  send({ op: "subscribe", topic: "packets", filter: filter.trim() || undefined });
}

export function connect() {
  if (socket) return;
  const ws = new WebSocket(wsUrl());
  socket = ws;
  ws.onopen = () => {
    useApp.setState({ connected: true });
    subscribePackets(useApp.getState().filter);
    for (const topic of ["devices", "metrics", "alerts"]) send({ op: "subscribe", topic });
  };
  ws.onclose = () => {
    socket = null;
    useApp.setState({ connected: false });
    setTimeout(connect, 1500);
  };
  ws.onmessage = (ev) => {
    let m: { topic: string; [k: string]: unknown };
    try {
      m = JSON.parse(ev.data as string);
    } catch {
      return;
    }
    const s = useApp.getState();
    switch (m.topic) {
      case "packets": {
        const items = m.items as Packet[];
        const dropped = (m.dropped as number) ?? 0;
        if (s.paused) {
          if (dropped) useApp.setState({ dropped: s.dropped + dropped });
          return;
        }
        let next = s.packets.length + items.length > MAX_PACKETS ? s.packets.slice(s.packets.length + items.length - MAX_PACKETS) : s.packets.slice();
        next = next.concat(items);
        // Aggregates for "Latest per message" and the timeline.
        const latest = { ...s.latest };
        const timeline = s.timeline.slice();
        for (const p of items) {
          const k = `${p.pgn}|${p.src}`;
          const a = latest[k];
          const times = a ? a.times.concat(p.ts).slice(-20) : [p.ts];
          latest[k] = { key: k, pgn: p.pgn, src: p.src, name: p.name, count: (a?.count ?? 0) + 1, first: a?.first ?? p.ts, last: p, times };
          const sec = Math.floor(Date.now() / 1000);
          let b = timeline[timeline.length - 1];
          if (!b || b.t !== sec) {
            b = { t: sec, bySrc: {} };
            timeline.push(b);
            if (timeline.length > 60) timeline.shift();
          }
          b.bySrc[p.src] = (b.bySrc[p.src] ?? 0) + 1;
        }
        useApp.setState({ packets: next, dropped: s.dropped + dropped, latest, timeline });
        break;
      }
      case "devices":
        useApp.setState({ devices: m.items as Device[] });
        break;
      case "metrics": {
        const met = m.data as Metrics;
        const srcHist: Record<number, number[]> = {};
        for (const [addr, fps] of met.per_source ?? []) srcHist[addr] = (s.srcHist[addr] ?? []).concat(fps).slice(-30);
        useApp.setState({ metrics: met, banks: m.banks as Bank[], srcHist });
        break;
      }
      case "status":
        useApp.setState({ status: m.data as Status });
        break;
      case "alerts": {
        const items = m.items as Alert[];
        useApp.setState({ alerts: s.alerts.concat(items).slice(-300) });
        break;
      }
      case "filter_error":
        useApp.setState({ filterError: m.message as string });
        break;
    }
  };
}

export function deviceLabel(d: Device | undefined | null): string {
  if (!d) return "—";
  const model = d.model || d.product?.model_id;
  if (model) return model;
  if (d.manufacturer) return `${d.manufacturer} device`;
  return d.name ? "Unidentified device" : `Source ${d.address}`;
}
