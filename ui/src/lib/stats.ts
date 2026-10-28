/** Cumulative traffic counters (GET /stats) and the instancing findings,
 *  polled for the Devices table and map. */
import { useEffect, useState } from "react";
import { api } from "./api";
import type { Finding } from "./types";

export interface SourceTotals {
  source: number;
  frames: number;
  bytes: number;
  messages: number;
  incomplete: number;
  first_us: number;
  last_us: number;
}

export interface PgnTotals {
  pgn: number;
  source: number;
  messages: number;
  frames: number;
  bytes: number;
  frames_per_msg: number;
  first_us: number;
  last_us: number;
}

export interface Stats {
  since_us: number;
  now_us: number;
  sources: SourceTotals[];
  pgns: PgnTotals[];
}

const EMPTY: Stats = { since_us: 0, now_us: 0, sources: [], pgns: [] };

export function useStats(everyMs = 2000): [Stats, () => void] {
  const [s, setS] = useState<Stats>(EMPTY);
  const [n, setN] = useState(0);
  useEffect(() => {
    let alive = true;
    const tick = () =>
      api<Stats>("/stats")
        .then((r) => alive && setS(r))
        .catch(() => {});
    tick();
    const t = setInterval(tick, everyMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [everyMs, n]);
  return [s, () => setN((x) => x + 1)];
}

/** Non-info findings from the instancing analysis. */
export function useFindings(everyMs = 5000): Finding[] {
  const [f, setF] = useState<Finding[]>([]);
  useEffect(() => {
    let alive = true;
    const tick = () =>
      api<Finding[]>("/analysis/instances")
        .then((r) => alive && setF(r.filter((x) => x.severity !== "info")))
        .catch(() => {});
    tick();
    const t = setInterval(tick, everyMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [everyMs]);
  return f;
}

/** PGN number → name from the PGN database (loaded once). */
let pgnNames: Map<number, string> | null = null;
let pgnNamesP: Promise<Map<number, string>> | null = null;
export function usePgnNames(): Map<number, string> {
  const [m, setM] = useState<Map<number, string>>(pgnNames ?? new Map());
  useEffect(() => {
    if (pgnNames) return;
    pgnNamesP ??= api<{ pgns: { pgn: number; name: string }[] }>("/pgn-db").then((r) => {
      const map = new Map<number, string>();
      for (const p of r.pgns) if (!map.has(p.pgn)) map.set(p.pgn, p.name);
      pgnNames = map;
      return map;
    });
    pgnNamesP.then(setM).catch(() => {});
  }, []);
  return m;
}

export const fmtCount = (n: number) => Math.round(n).toLocaleString();

export function fmtDuration(us: number): string {
  const s = Math.max(0, Math.round(us / 1e6));
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}
