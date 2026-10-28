/** Small SVG chart kit. No chart library: these draw exactly what the
 *  screens need, use the theme tokens (so Day/Dark/Night just work), show
 *  gaps where data is missing, and give every chart a text alternative. */
import { useEffect, useRef, useState, type ReactNode } from "react";

export function useWidth<T extends HTMLElement>(): [React.RefObject<T>, number] {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver((e) => setW(Math.floor(e[0].contentRect.width)));
    ro.observe(el);
    setW(Math.floor(el.getBoundingClientRect().width));
    return () => ro.disconnect();
  }, []);
  return [ref, w];
}

/** Round up to a "nice" axis maximum. */
export function niceMax(v: number, min = 1): number {
  const x = Math.max(v, min);
  const p = Math.pow(10, Math.floor(Math.log10(x)));
  for (const m of [1, 2, 2.5, 5, 10]) if (x <= m * p) return m * p;
  return 10 * p;
}

/** Split a series into runs of non-null values (gaps are drawn as gaps). */
function runs<T>(pts: T[], has: (p: T) => boolean): T[][] {
  const out: T[][] = [];
  let cur: T[] = [];
  for (const p of pts) {
    if (has(p)) cur.push(p);
    else if (cur.length) {
      out.push(cur);
      cur = [];
    }
  }
  if (cur.length) out.push(cur);
  return out;
}

export interface TimePoint {
  t: number; // unix seconds
  v: number | null;
}

const hhmm = (t: number, secs = false) => {
  const d = new Date(t * 1000);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}${secs ? ":" + p(d.getSeconds()) : ""}`;
};

/** Area/line chart over time with hover readout and visible gaps. */
export function TimeArea({
  points,
  height = 200,
  unit = "",
  digits = 1,
  min,
  maxHint = 1,
  label,
  extra,
  color = "var(--c1)",
}: {
  points: TimePoint[];
  height?: number;
  unit?: string;
  digits?: number;
  min?: number;
  maxHint?: number;
  label: string;
  extra?: (i: number) => string | null;
  color?: string;
}) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const padR = 8, padT = 10, padB = 24;
  const vals = points.map((p) => p.v).filter((v): v is number => v != null);
  const lo = min ?? (vals.length ? Math.min(...vals) : 0);
  const hiRaw = vals.length ? Math.max(...vals) : maxHint;
  const hi = min === 0 ? niceMax(hiRaw, maxHint) : hiRaw + (hiRaw - lo) * 0.15 || lo + 1;
  const loAxis = min === 0 ? 0 : lo - (hi - lo) * 0.1;
  const span0 = hi - (min === 0 ? 0 : lo - (hi - lo) * 0.1);
  const tdig = span0 >= 20 ? 0 : span0 >= 2 ? 1 : span0 >= 0.2 ? 2 : 3;
  const padL = Math.max(32, 12 + 7 * (Math.max(Math.abs(hi), Math.abs(lo)).toFixed(tdig).length + (unit === "%" ? 1 : 0)));
  const iw = Math.max(1, w - padL - padR), ih = height - padT - padB;
  const n = points.length;
  const x = (i: number) => padL + (n <= 1 ? 0 : (i / (n - 1)) * iw);
  const y = (v: number) => padT + ih - ((v - loAxis) / (hi - loAxis || 1)) * ih;
  const idx = points.map((p, i) => ({ ...p, i }));
  const segs = runs(idx, (p) => p.v != null);
  const gaps = runs(idx, (p) => p.v == null);
  const ticks = [loAxis, (loAxis + hi) / 2, hi];
  const fmt = (v: number) => v.toFixed(digits);
  const span = hi - loAxis;
  const tickDigits = span >= 20 ? 0 : span >= 2 ? 1 : span >= 0.2 ? 2 : 3;
  const tickText = (t: number) => (t === 0 ? "0" : t.toFixed(tickDigits));
  const h = hover != null ? idx[hover] : null;
  const onMove = (e: React.MouseEvent<SVGSVGElement>) => {
    const r = (e.currentTarget as SVGSVGElement).getBoundingClientRect();
    const px = e.clientX - r.left - padL;
    if (n < 2 || px < -4 || px > iw + 4) return setHover(null);
    setHover(Math.max(0, Math.min(n - 1, Math.round((px / iw) * (n - 1)))));
  };
  const latest = [...points].reverse().find((p) => p.v != null);
  return (
    <div ref={ref} className="relative w-full">
      {w > 0 && (
        <svg width={w} height={height} role="img" aria-label={`${label}${latest ? `, now ${fmt(latest.v!)}${unit}` : ""}`} onMouseMove={onMove} onMouseLeave={() => setHover(null)} className="block select-none">
          <g className="font-mono" fontSize="11" fill="rgb(var(--muted))">
            {ticks.map((t, i) => (
              <g key={i}>
                <line x1={padL} x2={w - padR} y1={y(t)} y2={y(t)} stroke="rgb(var(--rule))" />
                <text x={padL - 8} y={y(t) + 4} textAnchor="end">
                  {tickText(t)}
                  {unit === "%" ? "%" : ""}
                </text>
              </g>
            ))}
            {n > 1 && (
              <>
                <text x={padL} y={height - 6}>{hhmm(points[0].t)}</text>
                <text x={w - padR} y={height - 6} textAnchor="end">now</text>
              </>
            )}
          </g>
          {gaps.map((g, k) =>
            g.length > 1 || (g[0].i > 0 && g[0].i < n - 1) ? (
              <g key={`g${k}`}>
                <rect x={x(Math.max(0, g[0].i - 1))} y={padT} width={Math.max(2, x(Math.min(n - 1, g[g.length - 1].i + 1)) - x(Math.max(0, g[0].i - 1)))} height={ih} fill="rgb(var(--shoal))" />
                {x(g[g.length - 1].i) - x(g[0].i) > 48 && (
                  <text x={(x(g[0].i) + x(g[g.length - 1].i)) / 2} y={padT + ih / 2} textAnchor="middle" fontSize="11" fill="rgb(var(--muted))">
                    no data
                  </text>
                )}
              </g>
            ) : null,
          )}
          {segs.map((s, k) => {
            const line = s.map((p, j) => `${j ? "L" : "M"}${x(p.i).toFixed(1)} ${y(p.v!).toFixed(1)}`).join(" ");
            const base = y(loAxis);
            return (
              <g key={k}>
                <path d={`${line} L${x(s[s.length - 1].i).toFixed(1)} ${base} L${x(s[0].i).toFixed(1)} ${base} Z`} fill={`rgb(${color} / 0.12)`} />
                <path d={line} fill="none" stroke={`rgb(${color})`} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
                {s.length === 1 && <circle cx={x(s[0].i)} cy={y(s[0].v!)} r={2.5} fill={`rgb(${color})`} />}
              </g>
            );
          })}
          {h && (
            <g pointerEvents="none">
              <line x1={x(h.i)} x2={x(h.i)} y1={padT} y2={padT + ih} stroke="rgb(var(--ink) / 0.35)" strokeDasharray="3 3" />
              {h.v != null && <circle cx={x(h.i)} cy={y(h.v)} r={4.5} fill="rgb(var(--panel))" stroke={`rgb(${color})`} strokeWidth={2} />}
            </g>
          )}
        </svg>
      )}
      {h && w > 0 && (
        <div
          className="pointer-events-none absolute top-1 rounded-lg bg-ink px-2.5 py-1.5 text-paper shadow-lg"
          style={{ left: Math.min(Math.max(x(h.i) + 12, 0), w - 170), minWidth: 120 }}
        >
          <div className="font-mono text-[11px] opacity-75">{hhmm(h.t, true)}</div>
          <div className="text-sm font-semibold">
            {h.v == null ? "No data" : `${fmt(h.v)}${unit === "%" ? "%" : unit ? " " + unit : ""}`}
            {h.v != null && extra?.(h.i) ? ` · ${extra(h.i)}` : ""}
          </div>
        </div>
      )}
    </div>
  );
}

/** Minimal trend line. */
export function Sparkline({ values, width = 72, height = 20, color = "var(--c1)", label }: { values: number[]; width?: number; height?: number; color?: string; label?: string }) {
  if (values.length < 2) return <svg width={width} height={height} aria-hidden="true" />;
  const lo = Math.min(...values), hi = Math.max(...values);
  const span = hi - lo || Math.max(1, hi) * 0.5 || 1;
  const d = values.map((v, i) => `${i ? "L" : "M"}${((i / (values.length - 1)) * (width - 2) + 1).toFixed(1)} ${(height - 2 - ((v - lo) / span) * (height - 4)).toFixed(1)}`).join(" ");
  return (
    <svg width={width} height={height} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
      <path d={d} fill="none" stroke={`rgb(${color})`} strokeWidth={1.5} strokeLinejoin="round" strokeLinecap="round" />
    </svg>
  );
}

/** Ranked horizontal bars. */
export function BarList({ items, unit = "", digits = 1 }: { items: { key: string | number; label: ReactNode; sub?: ReactNode; value: number; onClick?: () => void }[]; unit?: string; digits?: number }) {
  const max = Math.max(...items.map((i) => i.value), 0.0001);
  return (
    <ul className="space-y-3.5">
      {items.map((it) => (
        <li key={it.key}>
          <div className="flex items-baseline justify-between gap-3 text-[13px]">
            <span className="min-w-0 truncate">
              {it.onClick ? (
                <button className="font-semibold hover:underline" onClick={it.onClick}>
                  {it.label}
                </button>
              ) : (
                <span className="font-semibold">{it.label}</span>
              )}{" "}
              {it.sub && <span className="text-muted">{it.sub}</span>}
            </span>
            <span className="font-mono">
              {it.value.toFixed(digits)}
              {unit}
            </span>
          </div>
          <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-shoal">
            <div className="h-full rounded-full bg-c1" style={{ width: `${(it.value / max) * 100}%` }} />
          </div>
        </li>
      ))}
    </ul>
  );
}

export const SERIES = ["var(--c1)", "var(--c2)", "var(--c3)", "var(--c4)", "var(--muted)"];

/** Stacked columns, one per time bucket. */
export function StackedBars({ buckets, series, height = 96, label, selected }: { buckets: number[][]; series: string[]; height?: number; label: string; selected?: [number, number] | null }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const max = niceMax(Math.max(1, ...buckets.map((b) => b.reduce((a, c) => a + c, 0))));
  const n = Math.max(buckets.length, 1);
  const bw = w / n;
  return (
    <div ref={ref} className="w-full">
      {w > 0 && (
        <svg width={w} height={height} role="img" aria-label={label} className="block">
          <line x1={0} x2={w} y1={height - 0.5} y2={height - 0.5} stroke="rgb(var(--rule))" />
          {buckets.map((b, i) => {
            let yy = height;
            return (
              <g key={i}>
                {b.map((v, k) => {
                  const hgt = (v / max) * (height - 4);
                  yy -= hgt;
                  return v > 0 ? <rect key={k} x={i * bw + 1} y={yy} width={Math.max(1, bw - 2)} height={hgt} fill={`rgb(${SERIES[k % SERIES.length]})`} /> : null;
                })}
              </g>
            );
          })}
          {selected && <rect x={selected[0] * bw} y={0} width={(selected[1] - selected[0] + 1) * bw} height={height} fill="rgb(var(--sea) / 0.08)" stroke="rgb(var(--sea) / 0.5)" />}
          <text x={4} y={12} fontSize="11" className="font-mono" fill="rgb(var(--muted))">
            {max}/s
          </text>
        </svg>
      )}
      {series.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs">
          {series.map((s, k) => (
            <span key={s} className="flex items-center gap-1.5">
              <span className="h-2.5 w-2.5 rounded-sm" style={{ background: `rgb(${SERIES[k % SERIES.length]})` }} aria-hidden />
              {s}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

/** One bar split into shares (e.g. each circuit's part of total load). */
export function ShareBar({ parts, unit = "", label }: { parts: { label: string; value: number }[]; unit?: string; label: string }) {
  const total = parts.reduce((a, p) => a + p.value, 0);
  if (total <= 0) return <div className="h-6 rounded-md bg-shoal" aria-label={`${label}: nothing on`} role="img" />;
  const sorted = [...parts].filter((p) => p.value > 0).sort((a, b) => b.value - a.value);
  return (
    <div className="flex h-6 w-full gap-0.5 overflow-hidden rounded-md" role="img" aria-label={`${label}: ${sorted.map((p) => `${p.label} ${p.value.toFixed(1)}${unit}`).join(", ")}`}>
      {sorted.map((p, i) => (
        <div
          key={p.label}
          title={`${p.label}: ${p.value.toFixed(1)}${unit}`}
          className="flex items-center overflow-hidden whitespace-nowrap px-2 text-[11px] font-medium"
          style={{ flex: `${p.value} 1 0`, background: `rgb(var(--c1) / ${Math.max(0.25, 1 - i * 0.17)})`, color: i < 2 ? "rgb(var(--on-sea))" : "transparent" }}
        >
          {p.label} · {p.value.toFixed(1)}
          {unit}
        </div>
      ))}
    </div>
  );
}
