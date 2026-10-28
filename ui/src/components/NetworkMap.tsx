/**
 * Network map: the backbone with a drop per device, terminators at the ends
 * and the power tee. The physical order is never guessed: until a technician
 * arranges it, devices are grouped by type and the map says so.
 */
import { useEffect, useMemo, useState, type CSSProperties, type DragEvent, type ReactNode } from "react";
import { Info, Check, Maximize2, Plus } from "lucide-react";
import { deviceLabel, load, save } from "../lib/store";
import { summary, systemOf, SYSTEMS, type LiveMap } from "../lib/live";
import { useWidth } from "../lib/charts";
import { devInst, dataInfo, ratesBySource } from "../lib/device";
import type { Device, Metrics, Status } from "../lib/types";

type End = "unset" | "present" | "missing";
interface Layout {
  mode: "alternating" | "oneside";
  fields: string[];
  order: string[] | null;
  ends: { a: End; b: End };
  /** Power tee sits after device index `tee` (-1 = before the first). */
  tee: number | null;
}

const FIELDS: { id: string; label: string }[] = [
  { id: "rate", label: "Address and messages/s" },
  { id: "reading", label: "Live reading" },
  { id: "share", label: "Share of bus" },
  { id: "instance", label: "Instance" },
  { id: "len", label: "LEN" },
  { id: "sw", label: "Software version" },
  { id: "serial", label: "Serial number" },
];
const MAX_FIELDS = 3;
const DEFAULT: Layout = { mode: "alternating", fields: ["rate", "reading"], order: null, ends: { a: "unset", b: "unset" }, tee: null };
const KEY = "keelwire.map";

const C = {
  sea: "rgb(var(--sea))",
  caution: "rgb(var(--caution))",
  port: "rgb(var(--port))",
  stbd: "rgb(var(--stbd))",
  muted: "rgb(var(--muted))",
  ink: "rgb(var(--ink))",
  rule: "rgb(var(--rule))",
};

interface Props {
  devices: Device[];
  all: Device[];
  live: LiveMap;
  metrics: Metrics | null;
  status: Status | null;
  attention: Set<string>;
  selected: string | null;
  onSelect: (k: string) => void;
}

export function NetworkMap({ devices, all, live, metrics, status, attention, selected, onSelect }: Props) {
  const [layout, setLayoutRaw] = useState<Layout>(() => ({ ...DEFAULT, ...load<Partial<Layout>>(KEY, {}) }));
  const setLayout = (l: Layout) => {
    save(KEY, l);
    setLayoutRaw(l);
  };
  const [editing, setEditing] = useState<Layout | null>(null);
  const [fieldsOpen, setFieldsOpen] = useState(false);
  const [ref, width] = useWidth<HTMLDivElement>();
  const L = editing ?? layout;

  const share = useMemo(() => new Map((metrics?.per_source ?? []).map(([a, , pct]) => [a, pct])), [metrics]);
  const rates = useMemo(() => ratesBySource(metrics?.per_pgn), [metrics]);

  // Order: technician's order (new devices appended), else grouped by type.
  const ordered = useMemo(() => {
    if (L.order) {
      const pos = new Map(L.order.map((k, i) => [k, i]));
      return [...devices].sort((a, b) => (pos.get(a.key) ?? 1e6 + (a.address ?? 0)) - (pos.get(b.key) ?? 1e6 + (b.address ?? 0)));
    }
    return [...devices].sort((a, b) => SYSTEMS.indexOf(systemOf(a)) - SYSTEMS.indexOf(systemOf(b)) || (a.address ?? 999) - (b.address ?? 999));
  }, [devices, L.order]);
  const placed = new Set(L.order ?? []);

  const len = all.filter((d) => d.online).reduce((a, d) => a + (d.product?.load_equivalency ?? 0), 0);
  const noLen = all.filter((d) => d.online && d.product?.load_equivalency == null).length;
  const errorsReported = !!status?.caps?.error_frames;

  const phone = width > 0 && width < 560;
  const cardH = 54 + 18 * L.fields.length;

  const card = (d: Device) => {
    const own = status?.own_address != null && d.address === status.own_address;
    const attn = attention.has(d.key);
    const r = d.address != null ? rates.get(d.address) ?? 0 : 0;
    const lines: { id: string; text: string; cls: string }[] = [];
    for (const f of L.fields) {
      if (f === "rate") lines.push({ id: f, text: `@${d.address ?? "—"} · ${d.online ? `${r.toFixed(1)}/s` : "—"}`, cls: "font-mono text-[11.5px] text-sea" });
      if (f === "reading") lines.push({ id: f, text: own ? "This analyzer · you’re here" : d.online ? summary(live[d.key], 2) || d.config?.installation_description1 || "—" : "Not responding", cls: `text-[12px] ${attn ? "text-caution font-medium" : d.online ? "" : "text-muted"}` });
      if (f === "share") lines.push({ id: f, text: `${(d.address != null ? share.get(d.address) ?? 0 : 0).toFixed(1)} % of bus`, cls: "text-[12px] text-muted" });
      if (f === "instance") lines.push({ id: f, text: `Device ${devInst(d) ?? "—"}${dataInfo(d).instances ? ` · data ${dataInfo(d).instances}` : ""}`, cls: "text-[12px] text-muted" });
      if (f === "len") lines.push({ id: f, text: d.product?.load_equivalency != null ? `LEN ${d.product.load_equivalency} (${d.product.load_equivalency * 50} mA)` : "LEN not reported", cls: "text-[12px] text-muted" });
      if (f === "sw") lines.push({ id: f, text: `Software ${d.product?.software_version || "—"}`, cls: "text-[12px] text-muted" });
      if (f === "serial") lines.push({ id: f, text: `Serial ${d.product?.serial_code || "—"}`, cls: "font-mono text-[11.5px] text-muted" });
    }
    return { own, attn, r, lines };
  };

  const dropStyle = (d: Device, r: number, attn: boolean): { color: string; width: number; dashed: boolean } => {
    if (!d.online) return { color: C.muted, width: 2, dashed: true };
    return { color: attn ? C.caution : C.sea, width: r < 1 ? 1.5 : r < 5 ? 2 : r < 15 ? 3 : 4, dashed: false };
  };

  // Drag to reorder (edit mode).
  const [dragKey, setDragKey] = useState<string | null>(null);
  const onDrop = (target: string) => {
    if (!editing || !dragKey || dragKey === target) return;
    const keys = ordered.map((d) => d.key).filter((k) => k !== dragKey);
    keys.splice(keys.indexOf(target) + (ordered.findIndex((d) => d.key === dragKey) < ordered.findIndex((d) => d.key === target) ? 1 : 0), 0, dragKey);
    setEditing({ ...editing, order: keys });
    setDragKey(null);
  };
  const cycle = (e: End): End => (e === "unset" ? "present" : e === "present" ? "missing" : "unset");
  const startEdit = () => setEditing({ ...layout, order: ordered.map((d) => d.key) });

  useEffect(() => {
    if (!fieldsOpen) return;
    const close = () => setFieldsOpen(false);
    window.addEventListener("click", close);
    return () => window.removeEventListener("click", close);
  }, [fieldsOpen]);

  const cardBox = (d: Device, style: CSSProperties) => {
    const c = card(d);
    const sel = d.key === selected;
    return (
      <button
        key={`c${d.key}`}
        type="button"
        draggable={!!editing}
        onDragStart={() => setDragKey(d.key)}
        onDragOver={(e: DragEvent) => editing && e.preventDefault()}
        onDrop={() => onDrop(d.key)}
        onClick={() => !editing && onSelect(d.key)}
        title={editing ? "Drag to its place on the backbone" : `${deviceLabel(d)}: open details`}
        className={`absolute flex flex-col gap-0.5 overflow-hidden rounded-[10px] border bg-panel px-3 py-2 text-left shadow-sm transition-shadow ${editing ? "cursor-grab active:cursor-grabbing" : "hover:shadow-md"} ${sel ? "ring-2 ring-sea" : ""} ${dragKey === d.key ? "opacity-50" : ""}`}
        style={{ ...style, borderColor: c.attn ? C.caution : c.own ? C.sea : undefined, borderWidth: c.attn || c.own ? 1.5 : 1 }}
      >
        <span className="flex items-center justify-between gap-2">
          <span className="truncate text-[13.5px] font-semibold">{deviceLabel(d)}</span>
          <span role="img" aria-label={!d.online ? "Not responding" : c.attn ? "Needs attention" : "Online"} className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: !d.online ? C.muted : c.attn ? C.caution : C.stbd }} />
        </span>
        <span className="truncate text-[12px] text-muted">{[d.manufacturer, d.device_function_text].filter(Boolean).join(" · ") || "Unidentified sender"}</span>
        {c.lines.map((l) => (
          <span key={l.id} className={`truncate ${l.cls}`}>
            {l.text}
          </span>
        ))}
        {editing && L.order && !placed.has(d.key) && <span className="text-[11px] font-semibold text-caution">New: drag into place</span>}
      </button>
    );
  };

  const terminator = (end: "a" | "b", x: number, y: number, label: string, labelAbove = false) => {
    const s = L.ends[end];
    const box = s === "present" ? { border: `1.5px solid ${C.ink}`, background: "rgb(var(--panel))", color: C.ink } : s === "missing" ? { border: `2px dashed ${C.port}`, background: "rgb(var(--port) / 0.08)", color: C.port } : { border: `1.5px dashed ${C.muted}`, background: "rgb(var(--panel))", color: C.muted };
    const text = s === "present" ? "terminator" : s === "missing" ? "missing" : "not set";
    return (
      <div key={`end${end}`}>
        <button
          type="button"
          disabled={!editing}
          onClick={() => editing && setEditing({ ...editing, ends: { ...editing.ends, [end]: cycle(s) } })}
          aria-label={`${label}: ${text}${editing ? ". Click to change" : ""}`}
          className="absolute flex h-11 w-11 items-center justify-center rounded-[10px] font-bold"
          style={{ left: x - 22, top: y - 22, ...box, cursor: editing ? "pointer" : "default" }}
        >
          {s === "present" ? (
            <svg width="24" height="13" viewBox="0 0 26 14" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden>
              <path d="M0 7h4l2-5 3 10 3-10 3 10 3-10 2 5h6" />
            </svg>
          ) : s === "missing" ? (
            "!"
          ) : (
            "?"
          )}
        </button>
        <span className="absolute w-24 text-center text-[11px] leading-tight" style={{ left: x - 48, top: labelAbove ? y - 54 : y + 26, color: s === "missing" ? C.port : C.muted, fontWeight: s === "missing" ? 600 : 400 }}>
          {label}
          <br />
          {text}
        </span>
      </div>
    );
  };

  const tee = (x: number, y: number, gap: number) => {
    const here = L.tee === gap;
    if (!here && !editing) return null;
    return (
      <button
        key={`tee${gap}`}
        type="button"
        disabled={!editing}
        onClick={() => editing && setEditing({ ...editing, tee: here ? null : gap })}
        aria-label={here ? `Power tee${editing ? ", click to remove" : ""}` : "Put the power tee here"}
        className={`absolute flex items-center justify-center rounded-full text-[10px] font-bold tracking-wider ${here ? "bg-ink text-paper" : "border border-dashed border-sea bg-panel text-sea opacity-70 hover:opacity-100"}`}
        style={{ left: x - (here ? 30 : 11), top: y - 11, width: here ? 60 : 22, height: 22 }}
      >
        {here ? "POWER" : <Plus size={12} />}
      </button>
    );
  };

  // ---------- geometry ----------
  const n = ordered.length;
  let body: ReactNode;
  if (n === 0) {
    body = <p className="p-10 text-center text-muted">No devices to show.</p>;
  } else if (phone) {
    body = (
      <div className="flex flex-col py-2 pr-2">
        <div className="flex min-h-[48px] items-center gap-2 pl-14 text-[12.5px] text-muted">End A · {L.ends.a === "present" ? "terminator" : L.ends.a === "missing" ? <span className="font-semibold text-port">missing</span> : "not set"}</div>
        {ordered.map((d, i) => {
          const c = card(d);
          const ds = dropStyle(d, c.r, c.attn);
          return (
            <div key={d.key}>
              <div className="flex items-stretch">
                <div className="relative w-11 shrink-0">
                  <div className="absolute bottom-0 left-[20px] top-0 w-1 bg-ink/80" />
                  <div className="absolute left-[17px] top-1/2 -mt-[5px] h-2.5 w-2.5 rounded-full" style={{ background: ds.color }} />
                  <div className="absolute left-[27px] right-0 top-1/2" style={{ borderTop: `${ds.width}px ${ds.dashed ? "dashed" : "solid"} ${ds.color}` }} />
                </div>
                <button type="button" onClick={() => onSelect(d.key)} className={`my-1 flex min-h-[56px] flex-1 items-center gap-2 rounded-[10px] border bg-panel px-3 py-2 text-left ${d.key === selected ? "ring-2 ring-sea" : ""}`} style={{ borderColor: c.attn ? C.caution : undefined }}>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[14px] font-semibold">
                      {deviceLabel(d)} <span className="font-mono text-[12px] font-normal text-sea">@{d.address ?? "—"}</span>
                    </span>
                    <span className={`block truncate ${c.lines.find((l) => l.id === "reading")?.cls ?? "text-[12.5px] text-muted"}`}>{c.lines.find((l) => l.id === "reading")?.text ?? summary(live[d.key], 2)}</span>
                  </span>
                  <span className="h-2.5 w-2.5 shrink-0 rounded-full" style={{ background: !d.online ? C.muted : c.attn ? C.caution : C.stbd }} />
                </button>
              </div>
              {L.tee === i && (
                <div className="flex items-center">
                  <div className="relative h-8 w-11 shrink-0">
                    <div className="absolute bottom-0 left-[20px] top-0 w-1 bg-ink/80" />
                    <span className="absolute left-[6px] top-[5px] rounded-full bg-ink px-1.5 text-[9px] font-bold text-paper">PWR</span>
                  </div>
                  <span className="text-[12.5px] text-muted">Power tee</span>
                </div>
              )}
            </div>
          );
        })}
        <div className="flex min-h-[48px] items-center gap-2 pl-14 text-[12.5px] text-muted">End B · {L.ends.b === "present" ? "terminator" : L.ends.b === "missing" ? <span className="font-semibold text-port">missing</span> : "not set"}</div>
      </div>
    );
  } else if (L.mode === "alternating") {
    const pitch = 84,
      cw = 156,
      x0 = 140;
    const top = L.order ? 16 : 52;
    const busY = top + cardH + 56;
    const W = Math.max(x0 + (n - 1) * pitch + x0, 520);
    const H = busY + 56 + cardH + 24;
    const xs = ordered.map((_, i) => x0 + i * pitch);
    const groups: { label: string; a: number; b: number }[] = [];
    if (!L.order)
      ordered.forEach((d, i) => {
        const g = groups[groups.length - 1];
        const s = systemOf(d);
        if (!g || g.label !== s) groups.push({ label: s, a: xs[i], b: xs[i] });
        else g.b = xs[i];
      });
    body = (
      <div className="relative mx-auto" style={{ width: W, height: H }}>
        {groups.map((g) => (
          <div key={g.label} className="absolute flex justify-center rounded-b-md border-x-[1.5px] border-b-[1.5px] border-rule" style={{ left: g.a - 38, width: g.b - g.a + 76, top: 12, height: 26 }}>
            <span className="whitespace-nowrap text-[10.5px] font-semibold uppercase tracking-wider text-muted">{g.label}</span>
          </div>
        ))}
        <div className="absolute h-1 rounded bg-ink/80" style={{ left: 56, width: W - 112, top: busY - 2 }} />
        {terminator("a", 34, busY, "End A")}
        {terminator("b", W - 34, busY, "End B")}
        {ordered.map((d, i) => {
          const x = xs[i],
            above = i % 2 === 0;
          const c = card(d);
          const ds = dropStyle(d, c.r, c.attn);
          const lineTop = above ? top + cardH : busY + 2;
          return (
            <div key={d.key}>
              <div className="absolute" style={{ left: x - ds.width / 2, top: lineTop, height: busY - 2 - (top + cardH) + (above ? 0 : 0), width: 0, borderLeft: `${ds.width}px ${ds.dashed ? "dashed" : "solid"} ${ds.color}` }} />
              <div className="absolute h-2.5 w-2.5 rounded-full" style={{ left: x - 5, top: busY - 5, background: ds.color }} />
              {cardBox(d, { left: x - cw / 2, top: above ? top : busY + 56, width: cw, height: cardH })}
            </div>
          );
        })}
        {ordered.map((_, i) => tee(i < n - 1 ? (xs[i] + xs[i + 1]) / 2 : (xs[i] + W - 56) / 2, busY, i))}
        {tee((56 + xs[0]) / 2, busY, -1)}
      </div>
    );
  } else {
    const pitch = 204,
      cw = 184,
      side = 160;
    const perRow = Math.max(1, Math.floor((Math.max(width, 600) - side * 2) / pitch) + 1);
    const rows = Math.ceil(n / perRow);
    const rowH = cardH + 110;
    const W = side * 2 + (Math.min(perRow, n) - 1) * pitch;
    const H = rows * rowH + 44;
    const pos = ordered.map((_, i) => {
      const r = Math.floor(i / perRow),
        k = i % perRow;
      const x = r % 2 === 0 ? side + k * pitch : side + (perRow - 1 - k) * pitch;
      return { x, y: 64 + r * rowH };
    });
    const lastRow = rows - 1;
    const endB = { x: lastRow % 2 === 0 ? W - 34 : 34, y: 64 + lastRow * rowH };
    const firstOfGroup = new Set<number>();
    if (!L.order) ordered.forEach((d, i) => (i === 0 || systemOf(d) !== systemOf(ordered[i - 1])) && firstOfGroup.add(i));
    body = (
      <div className="relative mx-auto" style={{ width: W, height: H }}>
        {Array.from({ length: rows }).map((_, r) => {
          const y = 64 + r * rowH;
          return (
            <div key={r}>
              <div className="absolute h-1 bg-ink/80" style={{ left: 56, width: W - 112, top: y - 2 }} />
              {r < rows - 1 && <div className="absolute w-1 bg-ink/80" style={{ left: r % 2 === 0 ? W - 58 : 54, top: y - 2, height: rowH + 4 }} />}
              {r < rows - 1 && <div className="absolute h-1 bg-ink/80" style={{ left: r % 2 === 0 ? W - 58 : 54, width: 4, top: y + rowH - 2 }} />}
            </div>
          );
        })}
        {terminator("a", 34, 64, "End A", true)}
        {terminator("b", endB.x, endB.y, "End B", true)}
        {ordered.map((d, i) => {
          const { x, y } = pos[i];
          const c = card(d);
          const ds = dropStyle(d, c.r, c.attn);
          return (
            <div key={d.key}>
              {firstOfGroup.has(i) && <span className="absolute whitespace-nowrap text-[10.5px] font-semibold uppercase tracking-wider text-muted" style={{ left: x - cw / 2, top: y - 26 }}>{systemOf(d)}</span>}
              <div className="absolute" style={{ left: x - ds.width / 2, top: y + 2, height: 44, width: 0, borderLeft: `${ds.width}px ${ds.dashed ? "dashed" : "solid"} ${ds.color}` }} />
              <div className="absolute h-2.5 w-2.5 rounded-full" style={{ left: x - 5, top: y - 5, background: ds.color }} />
              {cardBox(d, { left: x - cw / 2, top: y + 46, width: cw, height: cardH })}
            </div>
          );
        })}
        {ordered.map((_, i) => (i < n - 1 && Math.floor(i / perRow) === Math.floor((i + 1) / perRow) ? tee((pos[i].x + pos[i + 1].x) / 2, pos[i].y, i) : null))}
      </div>
    );
  }

  const tile = (label: string, value: string, sub: string, tone?: "warn" | "bad" | "muted") => (
    <div className="min-w-[168px] shrink-0 rounded-xl border hairline bg-panel px-3.5 py-3 md:min-w-0">
      <p className="label">{label}</p>
      <p className={`tnum mt-1 text-[17px] font-semibold ${tone === "muted" ? "text-muted" : tone === "bad" ? "text-port" : ""}`}>{value}</p>
      <p className={`text-[12px] ${tone === "warn" ? "text-caution" : "text-muted"}`}>{sub}</p>
    </div>
  );

  return (
    <div className="scroll-thin min-h-0 flex-1 overflow-auto px-4 pb-6 sm:px-7">
      <section aria-label="Bus health" className="scroll-thin -mx-1 mb-3 flex gap-2.5 overflow-x-auto px-1 pb-1 md:mx-0 md:grid md:grid-cols-3 md:overflow-visible md:px-0 md:pb-0 xl:grid-cols-5">
        {tile("Termination", L.ends.a === "missing" || L.ends.b === "missing" ? "Missing" : "Not measured", L.ends.a === "missing" || L.ends.b === "missing" ? "Marked missing by the technician" : "This gateway can’t measure it", L.ends.a === "missing" || L.ends.b === "missing" ? "bad" : "muted")}
        {errorsReported ? tile("Bus errors", `${metrics?.errors_per_sec ?? 0}/s`, `${(metrics?.total_errors ?? 0).toLocaleString()} since start`, (metrics?.total_errors ?? 0) > 0 ? "warn" : undefined) : tile("Bus errors", "Not reported", "This gateway doesn’t pass CAN errors on", "muted")}
        {tile("Bus load", `${(metrics?.load_pct ?? 0).toFixed(1)} %`, `Peak ${(metrics?.peak_load_pct ?? 0).toFixed(1)} % in the last minute`)}
        {tile("Declared load (LEN)", `${(len * 0.05).toFixed(2)} A max`, noLen ? `${len} LEN · ${noLen} didn’t report LEN` : `${len} LEN · stated maximum, not measured`, noLen ? "warn" : undefined)}
        {tile("Voltage at gateway", "Not measured", "Needs a Keelwire Tech Gateway", "muted")}
      </section>

      <section aria-label="Network map" className="card overflow-hidden">
        {editing ? (
          <div className="flex flex-wrap items-center justify-between gap-3 border-b hairline bg-sea/5 px-4 py-3">
            <p className="max-w-3xl text-[13px]">
              <strong className="font-semibold">Arranging the backbone.</strong> <span className="text-muted">Drag devices into the order they sit on the cable. Click an end to set its terminator. Click + between two drops to place the power tee.</span>
            </p>
            <div className="flex gap-2">
              <button className="btn" onClick={() => setEditing({ ...editing, order: null, tee: null })}>
                Group by type
              </button>
              <button className="btn" onClick={() => setEditing(null)}>
                Cancel
              </button>
              <button
                className="btn btn-primary"
                onClick={() => {
                  setLayout(editing);
                  setEditing(null);
                }}
              >
                <Check size={14} /> Save layout
              </button>
            </div>
          </div>
        ) : (
          <div className="flex flex-wrap items-center justify-between gap-3 border-b hairline px-4 py-3">
            {layout.order ? (
              <p className="flex items-center gap-2 text-[13px]">
                <Check size={15} className="text-stbd" aria-hidden />
                <span>
                  <strong className="font-semibold">Physical order</strong> <span className="text-muted">· set by technician, saved on this computer</span>
                </span>
              </p>
            ) : (
              <p className="flex max-w-3xl items-start gap-2 text-[13px]">
                <Info size={16} className="mt-0.5 shrink-0 text-sea" aria-hidden />
                <span>
                  <strong className="font-semibold">Order unknown.</strong> <span className="text-muted">Devices are grouped by type, not by where they sit on the backbone. Arrange them to match the boat.</span>
                </span>
              </p>
            )}
            <div className="flex flex-wrap items-center gap-2">
              {!phone && (
                <div className="seg" role="group" aria-label="Map layout">
                  <button aria-pressed={layout.mode === "alternating"} onClick={() => setLayout({ ...layout, mode: "alternating" })}>
                    Alternating
                  </button>
                  <button aria-pressed={layout.mode === "oneside"} onClick={() => setLayout({ ...layout, mode: "oneside" })}>
                    One side
                  </button>
                </div>
              )}
              <div className="relative" onClick={(e) => e.stopPropagation()}>
                <button className="btn" aria-expanded={fieldsOpen} onClick={() => setFieldsOpen(!fieldsOpen)}>
                  Card fields
                </button>
                {fieldsOpen && (
                  <div className="absolute right-0 top-full z-20 mt-1 w-72 rounded-xl border hairline bg-panel p-3 shadow-xl">
                    <p className="text-sm font-semibold">Card fields</p>
                    <p className="mb-2 text-[12px] text-muted">Name, type and status always show. Pick up to {MAX_FIELDS}; everything else is in the table.</p>
                    {FIELDS.map((f) => {
                      const on = layout.fields.includes(f.id);
                      const full = !on && layout.fields.length >= MAX_FIELDS;
                      return (
                        <label key={f.id} className={`flex min-h-[32px] cursor-pointer items-center gap-2.5 text-sm ${full ? "text-muted" : ""}`}>
                          <input type="checkbox" checked={on} disabled={full} onChange={() => setLayout({ ...layout, fields: on ? layout.fields.filter((x) => x !== f.id) : FIELDS.map((x) => x.id).filter((x) => x === f.id || layout.fields.includes(x)) })} />
                          {f.label}
                        </label>
                      );
                    })}
                    <p className="mt-1 text-[12px] text-muted">
                      {layout.fields.length} of {MAX_FIELDS} chosen
                    </p>
                  </div>
                )}
              </div>
              <button className={layout.order ? "btn" : "btn btn-primary"} onClick={startEdit}>
                {layout.order ? "Edit layout" : "Arrange backbone"}
              </button>
            </div>
          </div>
        )}
        <div ref={ref} className="scroll-thin overflow-x-auto" style={{ backgroundImage: "radial-gradient(rgb(var(--rule)) 1px, transparent 1px)", backgroundSize: "20px 20px" }}>
          {body}
        </div>
        <div className="flex flex-wrap items-center gap-x-5 gap-y-1.5 border-t hairline px-4 py-2.5 text-[12px] text-muted">
          <span className="flex items-center gap-2">
            <span className="h-1 w-6 rounded bg-sea" />
            Drop thickness = traffic
          </span>
          <span className="flex items-center gap-2">
            <span className="h-4 w-0 border-l-2 border-dashed border-muted" />
            Not responding
          </span>
          <span className="flex items-center gap-2">
            <span className="h-1 w-6 rounded bg-caution" />
            Needs attention
          </span>
          <span className="ml-auto flex items-center gap-1.5">
            <Maximize2 size={12} aria-hidden /> Click a device for details
          </span>
        </div>
      </section>
    </div>
  );
}
