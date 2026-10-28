/** Live readings per device, independent of the Traffic filter: polls each
 *  online device's latest messages and turns them into plain summaries. */
import { useEffect, useState } from "react";
import { api } from "./api";
import { friendly } from "./format";
import { useApp } from "./store";
import type { Device, Field } from "./types";

export interface LiveMsg {
  pgn: number;
  name: string;
  key: string | null;
  count: number;
  last_us: number;
  dst: number;
  data: string;
  fields: Field[];
}

export type LiveMap = Record<string, LiveMsg[]>;

/** Poll the latest messages of every online device (or just `only`). */
export function useLive(only?: string | null, everyMs = 2000): LiveMap {
  const devices = useApp((s) => s.devices);
  const [live, setLive] = useState<LiveMap>({});
  const keys = (only ? devices.filter((d) => d.key === only) : devices.filter((d) => d.online && d.name)).map((d) => d.key).join(",");
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      const ks = keys ? keys.split(",") : [];
      const res = await Promise.all(ks.map((k) => api<LiveMsg[]>(`/devices/${k}/messages`).then((m) => [k, m] as const).catch(() => [k, [] as LiveMsg[]] as const)));
      if (alive) setLive(Object.fromEntries(res));
    };
    tick();
    const t = setInterval(tick, everyMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [keys, everyMs]);
  return live;
}

const f = (m: LiveMsg | undefined, id: string): Field | undefined => m?.fields.find((x) => x.id === id && x.value !== null);
const num = (m: LiveMsg | undefined, id: string): number | null => {
  const v = f(m, id)?.value;
  return typeof v === "number" ? v : null;
};
const show = (m: LiveMsg | undefined, id: string) => {
  const x = f(m, id);
  return x ? friendly(x) : null;
};

export function fmtLatLon(lat: number, lon: number): string {
  const dm = (v: number, pos: string, neg: string, w: number) => {
    const a = Math.abs(v);
    const d = Math.floor(a);
    return `${String(d).padStart(w, "0")}°${((a - d) * 60).toFixed(2).padStart(5, "0")}′${v >= 0 ? pos : neg}`;
  };
  return `${dm(lat, "N", "S", 2)} ${dm(lon, "E", "W", 3)}`;
}

export interface Reading {
  label: string;
  value: string;
  /** numeric value in SI units, for history charts */
  raw?: number | null;
  unit?: string;
  field?: Field;
  pgn: number;
}

/** Headline readings of a device, most important first. */
export function readings(msgs: LiveMsg[] | undefined): Reading[] {
  if (!msgs) return [];
  const by = (pgn: number) => msgs.filter((m) => m.pgn === pgn);
  const one = (pgn: number) => by(pgn)[0];
  const out: Reading[] = [];
  const add = (label: string, m: LiveMsg | undefined, id: string, pgn: number) => {
    const x = f(m, id);
    if (x) out.push({ label, value: friendly(x), raw: typeof x.value === "number" ? x.value : null, field: x, pgn });
  };
  const pos = one(129025) ?? one(129029);
  if (pos && num(pos, "latitude") != null && num(pos, "longitude") != null) out.push({ label: "Position", value: fmtLatLon(num(pos, "latitude")!, num(pos, "longitude")!), pgn: pos.pgn });
  add("Speed over ground", one(129026), "sog", 129026);
  const hdg = one(127250);
  if (hdg && num(hdg, "heading") != null) {
    const ref = f(hdg, "reference")?.text?.startsWith("M") ? " M" : f(hdg, "reference")?.text?.startsWith("T") ? " T" : "";
    out.push({ label: "Heading", value: `${String(((num(hdg, "heading")! * 180) / Math.PI).toFixed(1)).padStart(5, "0")}°${ref}`, raw: num(hdg, "heading"), pgn: 127250, field: f(hdg, "heading") });
  }
  add("Rate of turn", one(127251), "rate", 127251);
  add("Depth", one(128267), "depth", 128267);
  add("Speed through water", one(128259), "speedWaterReferenced", 128259);
  const wind = one(130306);
  if (wind && num(wind, "windSpeed") != null) {
    out.push({ label: `Wind, ${(f(wind, "reference")?.text ?? "").toLowerCase() || "speed"}`, value: show(wind, "windSpeed")!, raw: num(wind, "windSpeed"), field: f(wind, "windSpeed"), pgn: 130306 });
    if (num(wind, "windAngle") != null) out.push({ label: "Wind angle", value: show(wind, "windAngle")!, raw: num(wind, "windAngle"), pgn: 130306 });
  }
  for (const m of [...by(130316), ...by(130312)]) {
    const src = f(m, "source")?.text ?? "Temperature";
    add(src, m, "temperature", m.pgn);
  }
  const env = one(130311) ?? one(130310);
  if (env) {
    add(f(env, "temperatureSource")?.text ?? "Temperature", env, "temperature", env.pgn);
    add("Air pressure", env, "atmosphericPressure", env.pgn);
  }
  for (const m of by(127488)) add(by(127488).length > 1 ? `Engine ${f(m, "instance")?.value}` : "Engine", m, "speed", 127488);
  for (const m of by(127489)) {
    add("Oil pressure", m, "oilPressure", 127489);
    add("Coolant", m, "temperature", 127489);
  }
  for (const m of by(127508)) {
    add(`Battery ${f(m, "instance")?.value ?? ""}`.trim(), m, "voltage", 127508);
    add("Current", m, "current", 127508);
  }
  for (const m of by(127505)) {
    const lvl = num(m, "level"), cap = f(m, "capacity");
    if (lvl != null) out.push({ label: `${f(m, "type")?.text ?? "Tank"} ${f(m, "instance")?.value ?? ""}`.trim(), value: `${lvl.toFixed(0)}%${cap ? ` of ${friendly(cap)}` : ""}`, raw: lvl, pgn: 127505 });
  }
  for (const m of by(127501)) {
    const ind = m.fields.filter((x) => /^indicator\d+$/.test(x.id) && (x.text === "On" || x.text === "Off"));
    if (ind.length) out.push({ label: `Bank ${f(m, "instance")?.value ?? ""}`.trim(), value: `${ind.filter((x) => x.text === "On").length} of ${ind.length} on`, pgn: 127501 });
  }
  return out;
}

export function summary(msgs: LiveMsg[] | undefined, n = 3): string {
  return readings(msgs)
    .slice(0, n)
    .map((r) => r.value)
    .join(" · ");
}

export type System = "Navigation" | "Engine and power" | "Tanks" | "Switching" | "Displays and gateways" | "Other";
export const SYSTEMS: System[] = ["Navigation", "Engine and power", "Tanks", "Switching", "Displays and gateways", "Other"];

/** Which part of the boat a device belongs to, from what it sends. */
export function systemOf(d: Device): System {
  const tx = new Set([...Object.keys(d.observed).map(Number), ...(d.tx_pgns ?? [])]);
  const has = (...p: number[]) => p.some((x) => tx.has(x));
  if (has(127501, 127502, 65284) || d.name?.device_class === 30) return "Switching";
  if (has(127505)) return "Tanks";
  if (has(127488, 127489, 127508, 127506, 127507)) return "Engine and power";
  if (has(129025, 129026, 129029, 127250, 127251, 128267, 128259, 130306, 130310, 130311, 130312, 130316)) return "Navigation";
  if (d.name && (d.name.device_class === 120 || d.name.device_class === 25)) return "Displays and gateways";
  return "Other";
}
