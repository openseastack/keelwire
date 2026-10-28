/** Devices → Table → By PGN: one row per message type on the bus. */
import { useMemo, useState } from "react";
import { saveBlob } from "../lib/files";
import { ChevronDown, ChevronUp, FileDown } from "lucide-react";
import { deviceLabel, load, save } from "../lib/store";
import { friendly, fmtAgo } from "../lib/format";
import { fmtCount, type Stats } from "../lib/stats";
import type { LiveMap } from "../lib/live";
import type { Device, Finding, Metrics } from "../lib/types";

type Chip = "all" | "multi" | "fast" | "stopped" | "attention";

interface Row {
  pgn: number;
  name: string;
  senders: { source: number; device: Device | undefined }[];
  rate: number;
  share: number;
  messages: number;
  frames: number;
  lastUs: number;
  value: string;
  multi: boolean;
  stopped: boolean;
  attention: boolean;
}

export function PgnTable({ stats, metrics, names, live, devices, findings, q, onDevice, onTraffic }: { stats: Stats; metrics: Metrics | null; names: Map<number, string>; live: LiveMap; devices: Device[]; findings: Finding[]; q: string; onDevice: (key: string) => void; onTraffic: (filter: string) => void }) {
  const [chip, setChip] = useState<Chip>("all");
  const [sort, setSortRaw] = useState<{ id: "share" | "rate" | "messages" | "pgn"; dir: 1 | -1 }>(() => load("keelwire.pgnsort", { id: "share", dir: -1 as 1 | -1 }));
  const setSort = (id: "share" | "rate" | "messages" | "pgn") => {
    const n = { id, dir: (sort.id === id ? -sort.dir : id === "pgn" ? 1 : -1) as 1 | -1 };
    save("keelwire.pgnsort", n);
    setSortRaw(n);
  };

  const rows = useMemo(() => {
    const byAddr = new Map(devices.filter((d) => d.address != null).map((d) => [d.address!, d]));
    const rate = new Map((metrics?.per_pgn ?? []).map(([p, s, r]) => [`${p}|${s}`, r]));
    const fps = Math.max(metrics?.frames_per_sec ?? 0, 1e-9);
    const attnPgns = new Set(findings.filter((f) => f.pgn != null).map((f) => f.pgn!));
    const groups = new Map<number, Row>();
    for (const p of stats.pgns) {
      let g = groups.get(p.pgn);
      if (!g) {
        g = { pgn: p.pgn, name: names.get(p.pgn) ?? `PGN ${p.pgn}`, senders: [], rate: 0, share: 0, messages: 0, frames: 0, lastUs: 0, value: "", multi: false, stopped: false, attention: attnPgns.has(p.pgn) };
        groups.set(p.pgn, g);
      }
      const r = rate.get(`${p.pgn}|${p.source}`) ?? 0;
      g.senders.push({ source: p.source, device: byAddr.get(p.source) });
      g.rate += r;
      g.share += ((r * Math.max(p.frames_per_msg, 1)) / fps) * 100;
      g.messages += p.messages;
      g.frames = Math.max(g.frames, p.frames_per_msg);
      if (p.last_us >= g.lastUs) {
        g.lastUs = p.last_us;
        const d = byAddr.get(p.source);
        const m = d ? live[d.key]?.find((x) => x.pgn === p.pgn) : undefined;
        g.value = m
          ? m.fields
              .filter((f) => !/^(reserved|spare)/i.test(f.id) && f.value !== null && !Array.isArray(f.value))
              .slice(0, 3)
              .map((f) => `${f.name} ${friendly(f)}`)
              .join(" · ")
          : "";
      }
    }
    const out = [...groups.values()];
    // Network-management messages come from every device and on request; they are never "several senders" or "stopped".
    const NM = new Set([59392, 59904, 60160, 60416, 60928, 65240, 126208, 126464, 126993, 126996, 126998]);
    for (const g of out) {
      g.multi = g.senders.length > 1 && !NM.has(g.pgn);
      g.stopped = !NM.has(g.pgn) && g.rate === 0 && g.messages >= 5 && stats.now_us - g.lastUs > 10e6;
      g.senders.sort((a, b) => a.source - b.source);
    }
    return out;
  }, [stats, metrics, names, live, devices, findings]);

  const ql = q.trim().toLowerCase();
  const shown = rows
    .filter((r) => (chip === "multi" ? r.multi : chip === "fast" ? r.frames > 1 : chip === "stopped" ? r.stopped : chip === "attention" ? r.attention : true))
    .filter((r) => {
      if (!ql) return true;
      if (ql.startsWith("@")) return r.senders.some((s) => String(s.source) === ql.slice(1));
      return String(r.pgn).includes(ql) || r.name.toLowerCase().includes(ql) || r.senders.some((s) => deviceLabel(s.device).toLowerCase().includes(ql));
    })
    .sort((a, b) => (sort.id === "pgn" ? a.pgn - b.pgn : sort.id === "rate" ? a.rate - b.rate : sort.id === "messages" ? a.messages - b.messages : a.share - b.share) * sort.dir);

  const count = (c: Chip) => rows.filter((r) => (c === "multi" ? r.multi : c === "fast" ? r.frames > 1 : c === "stopped" ? r.stopped : c === "attention" ? r.attention : true)).length;
  const chips: [Chip, string][] = [
    ["all", "All"],
    ["multi", "Several senders"],
    ["fast", "Multi-frame"],
    ["stopped", "Stopped"],
    ["attention", "Needs attention"],
  ];

  const exportCsv = () => {
    const esc = (v: unknown) => `"${String(v ?? "").replace(/"/g, '""')}"`;
    const head = ["PGN", "Name", "Senders", "Rate /s", "Share of bus %", "Messages", "Frames per message", "Latest value"];
    const lines = [head.map(esc).join(","), ...shown.map((r) => [r.pgn, r.name, r.senders.map((s) => `@${s.source}`).join(" "), r.rate.toFixed(2), r.share.toFixed(2), r.messages, r.frames, r.value].map(esc).join(","))];
    saveBlob(`pgns-${new Date().toISOString().slice(0, 10)}.csv`, new Blob([lines.join("\r\n")], { type: "text/csv" }));
  };

  const H = ({ id, children, right }: { id: "share" | "rate" | "messages" | "pgn"; children: string; right?: boolean }) => (
    <th aria-sort={sort.id === id ? (sort.dir === 1 ? "ascending" : "descending") : undefined} className={right ? "text-right" : ""}>
      <button className="inline-flex items-center gap-1 hover:text-ink" onClick={() => setSort(id)}>
        {children}
        {sort.id === id && (sort.dir === 1 ? <ChevronUp size={12} /> : <ChevronDown size={12} />)}
      </button>
    </th>
  );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-wrap items-center gap-2 px-4 pb-3 sm:px-7">
        {chips.map(([c, l]) => (
          <button key={c} className={`chip-btn ${chip === c ? "chip-btn-on" : ""} ${c === "attention" && count(c) ? "text-caution" : ""}`} aria-pressed={chip === c} onClick={() => setChip(c)}>
            {l} {count(c)}
          </button>
        ))}
        <span className="flex-1" />
        <button className="btn" onClick={exportCsv} aria-label="Export CSV" title="Export these rows as CSV">
          <FileDown size={14} /> CSV
        </button>
      </div>
      <div className="print-area scroll-thin mx-4 mb-4 min-h-0 flex-1 overflow-auto rounded-xl border hairline bg-panel sm:mx-7">
        {rows.length === 0 ? (
          <p className="p-10 text-center text-muted">No traffic counted yet.</p>
        ) : (
          <table className="tbl w-full min-w-[1080px] text-[13px]">
            <thead className="sticky top-0 bg-panel">
              <tr>
                <H id="pgn">PGN</H>
                <th>Name</th>
                <th>Sent by</th>
                <H id="rate" right>
                  Rate
                </H>
                <H id="share">Share of bus</H>
                <H id="messages" right>
                  Messages
                </H>
                <th className="text-right" title="CAN frames per message; more than 1 is a multi-frame message">
                  Frames
                </th>
                <th className="text-right">Last</th>
                <th>Latest value</th>
              </tr>
            </thead>
            <tbody>
              {shown.map((r) => (
                <tr key={r.pgn} className={r.attention ? "bg-caution/5" : undefined}>
                  <td className="font-mono text-sea">
                    <button className="hover:underline" onClick={() => onTraffic(`pgn == ${r.pgn}`)} title="Show in Traffic">
                      {r.pgn}
                    </button>
                  </td>
                  <td className="max-w-[260px]">
                    <span className="block truncate font-medium">{r.name}</span>
                    {(r.attention || r.stopped || r.multi) && <span className={`block text-[12px] ${r.attention || r.stopped ? "text-caution" : "text-muted"}`}>{r.attention ? "Instance conflict between senders" : r.stopped ? "Stopped: sender not heard" : "Several senders"}</span>}
                  </td>
                  <td>
                    <span className="flex flex-wrap gap-1">
                      {r.senders.map((s) => (
                        <button key={s.source} onClick={() => s.device && onDevice(s.device.key)} title={s.device ? `${deviceLabel(s.device)} @${s.source}` : `@${s.source}`} className={`rounded-md px-1.5 py-0.5 font-mono text-[11.5px] ${r.attention ? "bg-caution/10 text-caution" : r.stopped ? "bg-shoal text-muted" : "bg-sea/10 text-sea"}`}>
                          @{s.source}
                        </button>
                      ))}
                    </span>
                  </td>
                  <td className="tnum text-right font-semibold">{r.rate > 0 ? `${r.rate < 1 ? r.rate.toFixed(2) : r.rate.toFixed(1)}/s` : "—"}</td>
                  <td>
                    <span className="flex items-center gap-2">
                      <span className="h-1.5 w-24 overflow-hidden rounded bg-shoal">
                        <span className="block h-1.5 rounded bg-sea" style={{ width: `${Math.min(100, r.share * 4)}%` }} />
                      </span>
                      <span className="tnum w-12 text-right text-[12px] text-muted">{r.rate > 0 ? `${r.share.toFixed(1)} %` : "—"}</span>
                    </span>
                  </td>
                  <td className="tnum text-right">{fmtCount(r.messages)}</td>
                  <td className={`tnum text-right ${r.frames > 1 ? "" : "text-muted"}`}>{r.frames}</td>
                  <td className={`tnum text-right ${r.stopped ? "text-caution" : "text-muted"}`}>{fmtAgo(r.lastUs, stats.now_us)}</td>
                  <td className="max-w-[340px] truncate text-[12.5px] text-muted">{r.value}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
