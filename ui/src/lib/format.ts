import type { Units } from "./types";
import { useApp } from "./store";

export const hex2 = (n: number) => n.toString(16).toUpperCase().padStart(2, "0");

export function fmtTime(us: number): string {
  const s = us / 1e6;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${h > 0 ? h + ":" : ""}${String(m).padStart(2, "0")}:${sec.toFixed(3).padStart(6, "0")}`;
}

export function fmtAgo(us: number, nowUs: number): string {
  const s = Math.max(0, (nowUs - us) / 1e6);
  if (s < 2) return "now";
  if (s < 60) return `${Math.round(s)} s ago`;
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  return `${(s / 3600).toFixed(1)} h ago`;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

export function renderValue(f: { value: unknown; unit?: string; text?: string }): string {
  if (f.text) return f.text;
  const v = f.value;
  if (v === null || v === undefined) return "—";
  if (typeof v === "number") {
    const s = Number.isInteger(v) ? String(v) : Math.abs(v) >= 1000 ? v.toFixed(1) : String(Number(v.toPrecision(6)));
    return f.unit ? `${s} ${f.unit}` : s;
  }
  if (Array.isArray(v)) return `${v.length} items`;
  return String(v);
}


export const METRIC: Units = { depth: "m", distance: "nm", temperature: "C", speed: "kn", volume: "L", pressure: "kPa" };
export const US: Units = { depth: "ft", distance: "nm", temperature: "F", speed: "kn", volume: "gal", pressure: "psi" };

/** Convert SI units from the PGN database into the display units chosen in
 *  Settings. Mirrors keelwire-core `units::format` (used for the Traffic summaries). */
export function friendly(f: { id?: string; value: unknown; unit?: string; text?: string }): string {
  const v = f.value;
  if (typeof v !== "number" || f.text || !f.unit) return renderValue(f);
  const u = useApp.getState().status?.units ?? METRIC;
  const h = (f.id ?? "").toLowerCase();
  const n = (x: number, d: number) => x.toFixed(d);
  switch (f.unit) {
    case "rad":
      return `${n((v * 180) / Math.PI, 1)}°`;
    case "rad/s":
      return `${n(((v * 180) / Math.PI) * 60, 1)}°/min`;
    case "K":
      return u.temperature === "F" ? `${n(((v - 273.15) * 9) / 5 + 32, 1)} °F` : `${n(v - 273.15, 1)} °C`;
    case "m/s":
      return u.speed === "km/h" ? `${n(v * 3.6, 1)} km/h` : u.speed === "mph" ? `${n(v * 2.236936, 1)} mph` : u.speed === "m/s" ? `${n(v, 2)} m/s` : `${n(v * 1.943844, 2)} kn`;
    case "m":
      if (/log|distance|range/.test(h) || Math.abs(v) >= 20000)
        return u.distance === "km" ? `${n(v / 1000, 2)} km` : u.distance === "mi" ? `${n(v / 1609.344, 2)} mi` : `${n(v / 1852, 2)} nm`;
      return u.depth === "ft" ? `${n(v * 3.28084, 1)} ft` : u.depth === "fathom" ? `${n(v / 1.8288, 2)} fathom` : `${n(v, 2)} m`;
    case "L":
      return u.volume === "gal" ? `${n(v / 3.785412, 1)} gal` : u.volume === "impgal" ? `${n(v / 4.54609, 1)} imp gal` : `${n(v, 1)} L`;
    case "L/h":
      return u.volume === "gal" ? `${n(v / 3.785412, 2)} gal/h` : u.volume === "impgal" ? `${n(v / 4.54609, 2)} imp gal/h` : `${n(v, 1)} L/h`;
    case "Pa": {
      const baro = v >= 80000 && v <= 110000 && (h.includes("atmospheric") || (h.includes("pressure") && !/oil|fuel|boost|coolant/.test(h)));
      if (u.pressure === "psi") return `${n(v / 6894.757, 1)} psi`;
      if (u.pressure === "bar") return `${n(v / 100000, 3)} bar`;
      if (u.pressure === "inHg") return `${n(v / 3386.389, 2)} inHg`;
      if (baro) return `${n(v / 100, 1)} hPa`;
      return v >= 10000 ? `${n(v / 1000, 1)} kPa` : `${n(v, 0)} Pa`;
    }
  }
  return renderValue(f);
}
