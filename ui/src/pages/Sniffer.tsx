import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Pause, Play, Trash2, X, Columns3, Circle, Square } from "lucide-react";
import { useApp, deviceLabel, load, save, type MsgAgg } from "../lib/store";
import { StackedBars, TimeArea, type TimePoint } from "../lib/charts";
import { Hex } from "../components/Hex";
import { fmtTime, friendly } from "../lib/format";
import { api, post } from "../lib/api";
import type { Field, Packet } from "../lib/types";

type Col = "seq" | "time" | "delta" | "prio" | "pgn" | "name" | "src" | "dst" | "len" | "data" | "summary";
const COLS: { id: Col; label: string; width: string }[] = [
  { id: "seq", label: "Seq", width: "72px" },
  { id: "time", label: "Time", width: "92px" },
  { id: "delta", label: "Δt ms", width: "64px" },
  { id: "prio", label: "Pri", width: "34px" },
  { id: "pgn", label: "PGN", width: "64px" },
  { id: "name", label: "Message", width: "minmax(150px, 1.1fr)" },
  { id: "src", label: "Source", width: "minmax(110px, 0.7fr)" },
  { id: "dst", label: "Dst", width: "40px" },
  { id: "len", label: "Len", width: "40px" },
  { id: "data", label: "Data", width: "minmax(220px, 1.4fr)" },
  { id: "summary", label: "Decoded", width: "minmax(200px, 2fr)" },
];
const DEFAULT_COLS: Col[] = ["time", "prio", "pgn", "name", "src", "dst", "data", "summary"];

const EXAMPLES = ["pgn == 127501", "src == 0x14 && pgn in [127501, 65284]", 'name ~ "heading"', "!(pgn in [129025, 127250])", "tx"];

const NM_PGNS = "59392, 59904, 60928, 60160, 60416, 65240, 126208, 126464, 126993, 126996, 126998";

/** Compose the server filter from the chips and the free expression. */
function compose(src: number | null, hideNm: boolean, expr: string): string {
  const parts = [src != null ? `src == ${src}` : "", hideNm ? `!(pgn in [${NM_PGNS}])` : "", expr.trim()].filter(Boolean);
  return parts.length > 1 ? parts.map((x) => `(${x})`).join(" && ") : (parts[0] ?? "");
}

export function Sniffer() {
  const { packets, paused, setPaused, filter, setFilter, filterError, clearPackets, devices, dropped, latest, timeline, status, toast } = useApp();
  // A filter set elsewhere ("Show its traffic") arrives as a plain expression.
  const initial = useMemo(() => {
    const m = /^src == (\d+)$/.exec(filter.trim());
    if (m) return { src: Number(m[1]), expr: "" };
    if (filter.includes(`!(pgn in [${NM_PGNS}])`)) return { src: null as number | null, expr: "" };
    return { src: null as number | null, expr: filter };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const [srcChip, setSrcChip] = useState<number | null>(initial.src);
  const [hideNm, setHideNm] = useState<boolean>(() => load("keelwire.hidenm", false));
  const [mode, setModeRaw] = useState<"latest" | "frames">(() => load("keelwire.trafficmode", "latest"));
  const setMode = (m: "latest" | "frames") => {
    save("keelwire.trafficmode", m);
    setModeRaw(m);
  };
  const [selKey, setSelKey] = useState<string | null>(null);
  const [draft, setDraft] = useState(initial.expr);
  const applyFilter = (src: number | null, nm: boolean, expr: string) => {
    const f = compose(src, nm, expr);
    if (f !== filter) setFilter(f);
  };
  const [cols, setCols] = useState<Col[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("keelwire.cols") || "null") || DEFAULT_COLS;
    } catch {
      return DEFAULT_COLS;
    }
  });
  const [showCols, setShowCols] = useState(false);
  const [selected, setSelected] = useState<Packet | null>(null);
  const [follow, setFollow] = useState(true);
  const parent = useRef<HTMLDivElement>(null);

  const bySrc = useMemo(() => {
    const m = new Map<number, string>();
    for (const d of devices) if (d.address != null) m.set(d.address, deviceLabel(d));
    return m;
  }, [devices]);

  const prevTs = useMemo(() => {
    // Δt per (pgn, src): time since previous message of the same kind.
    const last = new Map<string, number>();
    const out = new Float64Array(packets.length);
    packets.forEach((p, i) => {
      const k = `${p.pgn}:${p.src}`;
      const l = last.get(k);
      out[i] = l === undefined ? NaN : (p.ts - l) / 1000;
      last.set(k, p.ts);
    });
    return out;
  }, [packets]);

  const v = useVirtualizer({ count: packets.length, getScrollElement: () => parent.current, estimateSize: () => 26, overscan: 20 });

  useEffect(() => {
    if (follow && packets.length) v.scrollToIndex(packets.length - 1, { align: "end" });
  }, [packets.length, follow, v]);

  const template = COLS.filter((c) => cols.includes(c.id)).map((c) => c.width).join(" ");

  const toggleCol = (c: Col) => {
    const next = cols.includes(c) ? cols.filter((x) => x !== c) : COLS.map((x) => x.id).filter((x) => x === c || cols.includes(x));
    setCols(next);
    try {
      localStorage.setItem("keelwire.cols", JSON.stringify(next));
    } catch {
      /* ignore */
    }
  };

  const cell = (p: Packet, c: Col, i: number) => {
    switch (c) {
      case "seq":
        return <span className="text-muted">{p.seq}</span>;
      case "time":
        return fmtTime(p.ts);
      case "delta":
        return Number.isNaN(prevTs[i]) ? "" : prevTs[i].toFixed(0);
      case "prio":
        return p.prio;
      case "pgn":
        return <span className="font-semibold">{p.pgn}</span>;
      case "name":
        return <span className="truncate">{p.name}</span>;
      case "src":
        return (
          <span className="truncate">
            <span className="font-mono text-[12px] text-muted">{p.src}</span> {p.tx ? <span className="text-caution">this app</span> : bySrc.get(p.src) ?? ""}
          </span>
        );
      case "dst":
        return p.dst === 255 ? <span className="text-muted">all</span> : p.dst;
      case "len":
        return p.len;
      case "data":
        return <Hex key={p.seq} data={p.data} diff={p.diff} />;
      case "summary":
        return <span className="truncate text-muted">{p.summary}</span>;
    }
  };

  const srcs = Object.entries(
    timeline.reduce<Record<string, number>>((acc, b) => {
      for (const [k, v] of Object.entries(b.bySrc)) acc[k] = (acc[k] ?? 0) + v;
      return acc;
    }, {}),
  )
    .sort((a, b) => b[1] - a[1])
    .slice(0, 4)
    .map(([k]) => Number(k));
  const padded = Array.from({ length: Math.max(0, 60 - timeline.length) }, () => ({ t: 0, bySrc: {} as Record<number, number> })).concat(timeline);
  const buckets = padded.map((b) => {
    const top = srcs.map((a) => b.bySrc[a] ?? 0);
    const rest = Object.entries(b.bySrc).reduce((a, [k, v]) => (srcs.includes(Number(k)) ? a : a + v), 0);
    return [...top, rest];
  });
  const rows = Object.values(latest).sort((a, b) => a.pgn - b.pgn || a.src - b.src);
  const selAgg = selKey ? latest[selKey] : null;
  const cap = status?.capture;

  return (
    <div className="flex h-full min-h-0">
      <div className={`relative min-w-0 flex-1 flex-col ${selected || selAgg ? "hidden lg:flex" : "flex"}`}>
        <div className="flex flex-col gap-3 px-4 pb-3 pt-5 sm:px-7">
          <div className="flex flex-wrap items-center gap-2">
            <h1 className="mr-2 text-2xl font-semibold tracking-tight">Traffic</h1>
            <span className="flex-1" />
            <div className="seg" role="group" aria-label="Show">
              <button aria-pressed={mode === "latest"} onClick={() => setMode("latest")}>
                Latest per message
              </button>
              <button aria-pressed={mode === "frames"} onClick={() => setMode("frames")}>
                Every frame
              </button>
            </div>
            <button className="btn" type="button" onClick={() => setPaused(!paused)} aria-pressed={paused}>
              {paused ? <Play size={14} /> : <Pause size={14} />} {paused ? "Resume" : "Pause"}
            </button>
            <button className="btn" type="button" onClick={clearPackets} title="Clear the list" aria-label="Clear">
              <Trash2 size={14} />
            </button>
            {cap ? (
              <button className="btn !border-port/50 !text-port" onClick={() => post("/capture/stop").then(() => toast("ok", `Saved ${cap.file}`)).catch((e) => toast("error", e.message))}>
                <Square size={13} fill="currentColor" /> Stop
              </button>
            ) : (
              <button className="btn" onClick={() => post("/capture/start", { format: "pcapng" }).then(() => toast("info", "Recording to a Wireshark file (find it under Settings → Captures)")).catch((e) => toast("error", e.message))}>
                <Circle size={13} className="text-port" fill="currentColor" /> Record
              </button>
            )}
          </div>

          <div className="card px-4 py-3">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <span className="label">Messages per second, last 60 s</span>
              <span className="label">{packets.length.toLocaleString()} kept{dropped ? `, ${dropped.toLocaleString()} skipped to keep up` : ""}</span>
            </div>
            <div className="mt-2">
              <StackedBars buckets={buckets} series={[...srcs.map((a) => `${devices.find((d) => d.address === a) ? deviceLabel(devices.find((d) => d.address === a)) : "Address " + a} · ${a}`), "Others"]} height={72} label="Messages per second by sender over the last minute" />
            </div>
          </div>

          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              applyFilter(srcChip, hideNm, draft);
            }}
          >
            <label className={`chip h-8 cursor-pointer gap-1 pr-1 text-[13px] ${srcChip != null ? "bg-sea/10 text-sea" : "border hairline bg-panel text-muted"}`}>
              <span>From</span>
              <select
                className="cursor-pointer bg-transparent pr-1 font-medium outline-none"
                value={srcChip ?? ""}
                aria-label="Only messages from"
                onChange={(e) => {
                  const v = e.target.value === "" ? null : Number(e.target.value);
                  setSrcChip(v);
                  applyFilter(v, hideNm, draft);
                }}
              >
                <option value="">any device</option>
                {devices
                  .filter((d) => d.address != null)
                  .sort((a, b) => a.address! - b.address!)
                  .map((d) => (
                    <option key={d.key} value={d.address!}>
                      {deviceLabel(d)} · {d.address}
                    </option>
                  ))}
              </select>
            </label>
            <button
              type="button"
              aria-pressed={hideNm}
              className={`chip h-8 text-[13px] ${hideNm ? "bg-sea/10 text-sea" : "border hairline bg-panel text-muted"}`}
              onClick={() => {
                save("keelwire.hidenm", !hideNm);
                setHideNm(!hideNm);
                applyFilter(srcChip, !hideNm, draft);
              }}
            >
              Hide network management
            </button>
            <span className="flex-1" />
            <label className="sr-only" htmlFor="flt">
              Filter expression
            </label>
            <input
              id="flt"
              className={`input h-8 min-w-0 flex-[1_1_260px] font-mono text-[12.5px] sm:max-w-sm ${filterError ? "!border-port" : ""}`}
              placeholder='Expression, e.g. pgn == 127501 or name ~ "depth"'
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              list="filter-examples"
            />
            <datalist id="filter-examples">
              {EXAMPLES.map((e) => (
                <option key={e} value={e} />
              ))}
            </datalist>
            <button className="btn btn-sm" type="submit">
              Apply
            </button>
            {filter && (
              <button
                className="btn btn-sm"
                type="button"
                onClick={() => {
                  setDraft("");
                  setSrcChip(null);
                  setHideNm(false);
                  save("keelwire.hidenm", false);
                  setFilter("");
                }}
              >
                <X size={13} /> Clear
              </button>
            )}
            {mode === "frames" && (
              <div className="relative">
                <button className="btn btn-sm" type="button" onClick={() => setShowCols(!showCols)} aria-expanded={showCols}>
                  <Columns3 size={13} /> Columns
                </button>
                {showCols && (
                  <div className="absolute right-0 z-20 mt-1 w-44 rounded-xl border hairline bg-panel p-2 shadow-xl">
                    {COLS.map((c) => (
                      <label key={c.id} className="flex items-center gap-2 px-1 py-0.5 text-sm">
                        <input type="checkbox" checked={cols.includes(c.id)} onChange={() => toggleCol(c.id)} /> {c.label}
                      </label>
                    ))}
                  </div>
                )}
              </div>
            )}
            {filterError && <div className="basis-full text-sm text-port">Filter not applied: {filterError}</div>}
          </form>
        </div>

        {mode === "latest" ? (
          <LatestTable rows={rows} bySrc={bySrc} sel={selKey} onSel={(k) => { setSelKey(k); setSelected(null); }} empty={<Empty filter={filter} />} />
        ) : (
        <div className="mx-4 mb-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border hairline bg-panel sm:mx-7">
        <div className="grid border-b hairline bg-shoal text-xs font-medium text-muted" style={{ gridTemplateColumns: template }}>
          {COLS.filter((c) => cols.includes(c.id)).map((c) => (
            <div key={c.id} className="truncate px-2 py-1.5">{c.label}</div>
          ))}
        </div>

        <div
          ref={parent}
          className="scroll-thin min-h-0 flex-1 overflow-auto"
          onScroll={(e) => {
            const el = e.currentTarget;
            setFollow(el.scrollHeight - el.scrollTop - el.clientHeight < 40);
          }}
        >
          {packets.length === 0 ? (
            <Empty filter={filter} />
          ) : (
            <div style={{ height: v.getTotalSize(), position: "relative" }}>
              {v.getVirtualItems().map((r) => {
                const p = packets[r.index];
                const sel = selected?.seq === p.seq;
                return (
                  <div
                    key={r.key}
                    role="row"
                    tabIndex={0}
                    onClick={() => setSelected(p)}
                    onKeyDown={(e) => e.key === "Enter" && setSelected(p)}
                    className={`absolute left-0 right-0 grid cursor-pointer items-center border-b border-rule/40 text-[13px] ${sel ? "bg-sea/10" : p.tx ? "bg-caution/10" : ""} hover:bg-shoal/60`}
                    style={{ top: r.start, height: 26, gridTemplateColumns: template }}
                  >
                    {COLS.filter((c) => cols.includes(c.id)).map((c) => (
                      <div key={c.id} className="flex min-w-0 items-center overflow-hidden whitespace-nowrap px-2.5">
                        {cell(p, c.id, r.index)}
                      </div>
                    ))}
                  </div>
                );
              })}
            </div>
          )}
        </div>
        {!follow && packets.length > 0 && (
          <button className="btn btn-primary absolute bottom-5 left-1/2 -translate-x-1/2 shadow-lg" onClick={() => setFollow(true)}>
            Jump to latest
          </button>
        )}
        </div>
        )}
      </div>
      {mode === "frames" && selected && <Detail p={selected} onClose={() => setSelected(null)} source={bySrc.get(selected.src)} />}
      {mode === "latest" && selAgg && <Detail p={selAgg.last} agg={selAgg} onClose={() => setSelKey(null)} source={bySrc.get(selAgg.src)} />}
    </div>
  );
}

function Empty({ filter }: { filter: string }) {
  const status = useApp((s) => s.status);
  return (
    <div className="mx-auto max-w-md p-10 text-center text-muted">
      {!status?.interface && !status?.offline ? (
        <>
          <p className="text-lg text-ink">No bus connected</p>
          <p className="mt-1">Choose an interface in Settings, or start the simulator to try things out.</p>
          <StartSim />
        </>
      ) : filter ? (
        <p>No messages match this filter yet.</p>
      ) : (
        <p>Waiting for traffic…</p>
      )}
    </div>
  );
}

function StartSim() {
  const toast = useApp((s) => s.toast);
  return (
    <button
      className="btn btn-primary mt-4"
      onClick={() => post("/interface", { uri: "sim:" }).catch((e) => toast("error", e.message))}
    >
      Start simulator
    </button>
  );
}

const NM_SET = new Set(NM_PGNS.split(", ").map(Number));

function rateText(a: MsgAgg): string {
  const span = a.times.length > 1 ? (a.times[a.times.length - 1] - a.times[0]) / 1e6 : 0;
  // One-off answers (product info, PGN lists) aren't periodic.
  if (a.times.length < 3 || span < 1.5) return NM_SET.has(a.pgn) || a.count < 3 ? "on request" : "—";
  const r = (a.times.length - 1) / span;
  if (r >= 0.95) return `${r.toFixed(r >= 10 ? 0 : 1)}/s`;
  const every = 1 / r;
  return every >= 50 ? `${(every / 60).toFixed(0)}/min` : `every ${every.toFixed(0)} s`;
}

/** One line per message type and sender, updated in place. */
function LatestTable({ rows, bySrc, sel, onSel, empty }: { rows: MsgAgg[]; bySrc: Map<number, string>; sel: string | null; onSel: (k: string) => void; empty: React.ReactNode }) {
  if (!rows.length) return <div className="flex-1">{empty}</div>;
  return (
    <div className="scroll-thin mx-4 mb-4 min-h-0 flex-1 overflow-auto rounded-xl border hairline bg-panel sm:mx-7">
      <table className="w-full text-[13px]">
        <thead className="sticky top-0 z-10 bg-shoal text-left text-xs text-muted">
          <tr>
            <th className="px-3 py-2 font-medium">PGN</th>
            <th className="px-3 py-2 font-medium">Message</th>
            <th className="px-3 py-2 font-medium">From</th>
            <th className="px-3 py-2 font-medium">Rate</th>
            <th className="hidden px-3 py-2 text-right font-medium md:table-cell">Count</th>
            <th className="hidden px-3 py-2 font-medium md:table-cell">Latest value</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((a) => (
            <tr key={a.key} onClick={() => onSel(a.key)} className={`cursor-pointer border-t hairline ${sel === a.key ? "bg-sea/10" : "hover:bg-shoal/60"}`} aria-selected={sel === a.key}>
              <td className="px-3 py-2 font-mono text-muted">{a.pgn}</td>
              <td className="max-w-[260px] truncate px-3 py-2 font-medium">{a.name}</td>
              <td className="whitespace-nowrap px-3 py-2">
                {a.last.tx ? <span className="text-caution">this app</span> : (bySrc.get(a.src) ?? "—")} <span className="font-mono text-xs text-muted">{a.src}</span>
              </td>
              <td className="whitespace-nowrap px-3 py-2 font-mono">{rateText(a)}</td>
              <td className="hidden px-3 py-2 text-right font-mono text-muted md:table-cell">{a.count.toLocaleString()}</td>
              <td className="hidden max-w-[420px] truncate px-3 py-2 text-muted md:table-cell">{a.last.summary}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

interface FieldDefLite {
  order: number;
  id: string;
  name: string;
  bits: number | null;
}

const FIELD_COLORS = ["var(--c1)", "var(--c2)", "var(--c3)", "var(--c4)"];

function Detail({ p, agg, onClose, source }: { p: Packet; agg?: MsgAgg; onClose: () => void; source?: string }) {
  const [fields, setFields] = useState<Field[] | null>(null);
  const [defs, setDefs] = useState<FieldDefLite[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const hist = useRef<{ key: string; pts: TimePoint[] }>({ key: "", pts: [] });
  const hkey = `${p.pgn}|${p.src}`;
  if (hist.current.key !== hkey) hist.current = { key: hkey, pts: [] };
  useEffect(() => {
    post<{ fields: Field[] }>("/decode", { pgn: p.pgn, data: p.data })
      .then((d) => {
        setFields(d.fields);
        setErr(null);
      })
      .catch((e) => setErr(e.message));
  }, [p.pgn, p.data]);
  useEffect(() => {
    setDefs(null);
    api<{ fallback: boolean; fields: FieldDefLite[] }[]>(`/pgn-db/${p.pgn}`)
      .then((r) => setDefs(r.find((x) => !x.fallback)?.fields ?? null))
      .catch(() => setDefs(null));
  }, [p.pgn]);

  // Byte ranges of the fixed-length leading fields, for the colour-coded byte map.
  const spans: { id: string; from: number; to: number; color: string }[] = [];
  if (defs) {
    let bit = 0;
    let k = 0;
    for (const f of defs) {
      if (f.bits == null) break;
      const from = Math.floor(bit / 8), to = Math.floor((bit + f.bits - 1) / 8);
      if (!/^(reserved|spare)/i.test(f.id)) spans.push({ id: f.id, from, to, color: /^sid$/i.test(f.id) ? "var(--muted)" : FIELD_COLORS[k++ % FIELD_COLORS.length] });
      bit += f.bits;
    }
  }
  const colorOf = (byte: number) => spans.find((s) => byte >= s.from && byte <= s.to)?.color;
  const spanOf = (id: string) => spans.find((s) => s.id === id);

  // Live history of the first numeric field while this panel is open.
  const numeric = fields?.find((f) => typeof f.value === "number" && f.unit && !/^sid$/i.test(f.id));
  if (numeric) {
    const t = Math.floor(Date.now() / 1000);
    const disp = parseFloat(friendly(numeric).replace(/[^\d.+-].*$/, ""));
    const pts = hist.current.pts;
    if (Number.isFinite(disp) && (!pts.length || pts[pts.length - 1].t !== t)) pts.push({ t, v: disp });
    if (pts.length > 300) pts.shift();
  }
  const unit = numeric ? friendly(numeric).replace(/^[-+\d.,\s°]*/, "").trim() : "";
  const bytes = p.data.match(/../g) ?? [];
  return (
    <aside className="scroll-thin flex w-full shrink-0 flex-col gap-5 overflow-auto border-l hairline bg-panel px-5 py-5 lg:w-[420px] sm:px-6">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="label font-mono">
            PGN {p.pgn} · priority {p.prio} · {p.frames > 1 ? `${p.frames} frames` : "single frame"}
          </p>
          <h2 className="mt-1 text-[22px] font-semibold leading-tight tracking-tight">{p.name}</h2>
          <p className="mt-0.5 text-muted">
            From {source ?? "address"} {p.src} · to {p.dst === 255 ? "everyone" : p.dst}
            {agg ? ` · ${rateText(agg)}` : ""}
          </p>
        </div>
        <button className="btn !px-2.5" onClick={onClose} aria-label="Close details">
          <X size={16} />
        </button>
      </div>

      <div>
        <h3 className="mb-2 text-sm font-semibold">Fields</h3>
        {err && <p className="text-sm text-port">{err}</p>}
        <dl className="grid grid-cols-[auto_1fr_auto] items-baseline gap-x-3 gap-y-1.5 text-[13px]">
          {fields
            ?.filter((f) => !/^reserved|^spare/i.test(f.id))
            .map((f, i) => {
              const sp = spanOf(f.id);
              return (
                <div key={i} className="contents">
                  <dt className="flex items-center gap-2 text-muted">
                    <span className="h-2.5 w-2.5 shrink-0 rounded-sm" style={{ background: sp ? `rgb(${sp.color})` : "transparent" }} aria-hidden />
                    {f.name}
                  </dt>
                  <dd className="min-w-0 font-mono">{Array.isArray(f.value) ? <ListValue items={f.value as [string, unknown][][]} /> : f.value === null ? <span className="text-muted">not available</span> : friendly(f)}</dd>
                  <dd className="font-mono text-xs text-muted">{sp ? (sp.from === sp.to ? `byte ${sp.from}` : `bytes ${sp.from}–${sp.to}`) : ""}</dd>
                </div>
              );
            })}
        </dl>
      </div>

      <div>
        <h3 className="mb-2 text-sm font-semibold">
          Bytes <span className="font-normal text-muted">({p.len})</span>
        </h3>
        <div className="flex flex-wrap gap-1.5">
          {bytes.map((b, i) => {
            const c = colorOf(i);
            const changed = p.diff?.[i];
            return (
              <div key={i} className="flex flex-col items-center gap-1">
                <span
                  className="flex h-8 w-10 items-center justify-center rounded-md font-mono text-[12.5px]"
                  style={{
                    background: c ? `rgb(${c} / 0.16)` : "rgb(var(--shoal))",
                    color: c ? `rgb(${c})` : "rgb(var(--muted))",
                    boxShadow: changed ? `0 0 0 2px rgb(var(--panel)), 0 0 0 3.5px rgb(var(--${changed > 0 ? "stbd" : "port"}))` : undefined,
                  }}
                  title={`byte ${i}${changed ? (changed > 0 ? ", went up" : ", went down") : ""}`}
                >
                  {b.toUpperCase()}
                </span>
                <span className="font-mono text-[10px] text-muted">{i}</span>
              </div>
            );
          })}
        </div>
        <p className="mt-2 text-xs text-muted">Outlined bytes changed since the previous message: green went up, red went down.</p>
      </div>

      {numeric && hist.current.pts.length > 1 && (
        <div>
          <h3 className="mb-1 text-sm font-semibold">{numeric.name}, since you opened this</h3>
          <TimeArea points={hist.current.pts} height={120} unit={unit} digits={2} label={`${numeric.name} history`} />
        </div>
      )}
      <div className="mt-auto flex flex-wrap gap-2 border-t hairline pt-4">
        <button
          className="btn btn-sm"
          onClick={() => {
            // PDU1 (addressed) PGNs carry the destination in the low byte of the id.
            const pf = (p.pgn >> 8) & 0xff;
            const idPgn = pf < 240 ? (p.pgn & 0x3ff00) | p.dst : p.pgn;
            const id = ((p.prio << 26) | (idPgn << 8) | p.src) >>> 0;
            const line = `can0  ${id.toString(16).toUpperCase().padStart(8, "0")}   [${p.len}]  ${(p.data.match(/../g) ?? []).join(" ").toUpperCase()}`;
            navigator.clipboard?.writeText(line);
          }}
        >
          Copy as candump
        </button>
        {source && (
          <button
            className="btn btn-sm"
            onClick={() => {
              const d = useApp.getState().devices.find((x) => x.address === p.src);
              if (d) {
                useApp.getState().selectDevice(d.key);
                useApp.getState().setPage("devices");
              }
            }}
          >
            Open {source}
          </button>
        )}
      </div>
    </aside>
  );
}

function ListValue({ items }: { items: [string, unknown][][] }) {
  return (
    <div className="space-y-1">
      {items.slice(0, 50).map((it, i) => (
        <div key={i} className="rounded bg-shoal/40 px-1.5 py-0.5 text-xs">
          {it.map(([k, v]) => `${k}: ${v === null ? "—" : typeof v === "object" ? JSON.stringify(v) : String(v)}`).join(", ")}
        </div>
      ))}
    </div>
  );
}
