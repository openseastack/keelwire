import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { isMobileApp, saveBlob } from "../lib/files";
import { RefreshCw, Network, Table2, X, Columns3, FileDown, Printer, ChevronUp, ChevronDown, ChevronRight, Plus, Minus, RotateCcw } from "lucide-react";
import { useApp, deviceLabel, load, save, type PanelTab } from "../lib/store";
import { useLive, summary, readings, systemOf, SYSTEMS, type LiveMap, type LiveMsg } from "../lib/live";
import { Sparkline, TimeArea, type TimePoint } from "../lib/charts";
import { DeviceSettings } from "../components/DeviceSettings";
import { NetworkMap } from "../components/NetworkMap";
import { PgnTable } from "../components/PgnTable";
import { api, patch, post } from "../lib/api";
import { friendly, fmtAgo, fmtBytes } from "../lib/format";
import { useStats, useFindings, usePgnNames, fmtCount, fmtDuration, type SourceTotals, type Stats } from "../lib/stats";
import { certText, devInst, dataInfo, ratesBySource } from "../lib/device";
import type { Device, Metrics } from "../lib/types";

type Group = "Identity" | "Traffic" | "Health and power" | "Setup";
const GROUPS: Group[] = ["Identity", "Traffic", "Health and power", "Setup"];

interface ColDef {
  id: string;
  label: string;
  group: Group;
  def: boolean;
  get: (d: Device, ctx: Ctx) => string | number | null | undefined;
  cell?: (d: Device, ctx: Ctx) => ReactNode;
  cls?: string;
}
interface Ctx {
  load: Map<number, number>;
  fps: Map<number, number>;
  rate: Map<number, number>;
  src: Map<number, SourceTotals>;
  pgns: Map<number, number>;
  nowUs: number;
  editable: boolean;
  attention: Set<string>;
  own: number | null;
}

const num = "tnum text-right";
const lastUs = (d: Device, c: Ctx) => Math.max(d.last_seen_us, d.address != null ? c.src.get(d.address)?.last_us ?? 0 : 0);
const COLS: ColDef[] = [
  {
    id: "model",
    label: "Device",
    group: "Identity",
    def: true,
    get: (d) => deviceLabel(d),
    cell: (d, c) => {
      const attn = c.attention.has(d.key);
      return (
        <span className="flex min-w-[180px] items-center gap-2.5">
          <span role="img" aria-label={!d.online ? "Not responding" : attn ? "Needs attention" : "Online"} className={`h-2.5 w-2.5 shrink-0 rounded-full ${!d.online ? "bg-muted/50" : attn ? "bg-caution" : "bg-stbd"}`} />
          <span className="min-w-0">
            <span className={`block truncate font-semibold ${c.own != null && d.address === c.own ? "text-sea" : ""}`}>{deviceLabel(d)}</span>
            <span className={`block truncate text-[12px] ${attn ? "text-caution" : "text-muted"}`}>{[d.manufacturer, d.device_function_text].filter(Boolean).join(" · ")}</span>
          </span>
        </span>
      );
    },
  },
  { id: "addr", label: "Address", group: "Identity", def: true, get: (d) => d.address, cell: (d) => <span className="font-mono text-sea">{d.address != null ? `@${d.address}` : "—"}</span> },
  { id: "rate", label: "Msgs/s", group: "Traffic", def: true, get: (d, c) => (d.online && d.address != null ? Number((c.rate.get(d.address) ?? 0).toFixed(1)) : null), cls: `${num} font-semibold` },
  {
    id: "share",
    label: "Share of bus",
    group: "Traffic",
    def: true,
    get: (d, c) => (d.address != null && c.load.get(d.address) != null ? Number(c.load.get(d.address)!.toFixed(1)) : null),
    cell: (d, c) => {
      const v = d.address != null ? c.load.get(d.address) : undefined;
      return v == null ? (
        <span className="text-muted">—</span>
      ) : (
        <span className="flex min-w-[120px] items-center gap-2">
          <span className="h-1.5 flex-1 overflow-hidden rounded bg-shoal">
            <span className="block h-1.5 rounded bg-sea" style={{ width: `${Math.min(100, v * 3)}%` }} />
          </span>
          <span className="tnum w-11 text-right text-[12px] text-muted">{v.toFixed(1)} %</span>
        </span>
      );
    },
  },
  { id: "messages", label: "Messages", group: "Traffic", def: true, get: (d, c) => (d.address != null ? c.src.get(d.address)?.messages : null), cell: (d, c) => fmtCount((d.address != null ? c.src.get(d.address)?.messages : 0) ?? 0), cls: num },
  { id: "bytes", label: "Data", group: "Traffic", def: true, get: (d, c) => (d.address != null ? c.src.get(d.address)?.bytes : null), cell: (d, c) => <span className="text-muted">{fmtBytes((d.address != null ? c.src.get(d.address)?.bytes : 0) ?? 0)}</span>, cls: num },
  { id: "pgns", label: "PGNs sent", group: "Traffic", def: true, get: (d, c) => (d.address != null ? c.pgns.get(d.address) ?? 0 : null), cls: num },
  { id: "fps", label: "Frames/s", group: "Traffic", def: false, get: (d, c) => (d.address != null ? c.fps.get(d.address) : null), cls: num },
  { id: "incomplete", label: "Incomplete", group: "Health and power", def: true, get: (d, c) => (d.address != null ? c.src.get(d.address)?.incomplete ?? 0 : null), cell: (d, c) => { const n = d.address != null ? c.src.get(d.address)?.incomplete ?? 0 : 0; return <span className={n ? "font-medium text-caution" : "text-muted"} title="Multi-frame messages that never completed">{n}</span>; }, cls: num },
  { id: "last", label: "Last heard", group: "Traffic", def: true, get: (d, c) => lastUs(d, c), cell: (d, c) => <span className={d.online ? "text-muted" : "text-caution"}>{fmtAgo(lastUs(d, c), c.nowUs)}</span>, cls: num },
  { id: "first", label: "First seen", group: "Traffic", def: false, get: (d) => d.first_seen_us, cell: (d, c) => <span className="text-muted">{fmtAgo(d.first_seen_us, c.nowUs)}</span>, cls: num },
  { id: "len", label: "LEN", group: "Health and power", def: true, get: (d) => d.product?.load_equivalency, cls: num },
  { id: "addrchg", label: "Address changes", group: "Health and power", def: false, get: (d) => Math.max(0, d.address_history.length - 1), cls: num },
  { id: "sw", label: "Software", group: "Identity", def: true, get: (d) => d.product?.software_version, cls: "text-[12.5px] text-muted" },
  { id: "mfr", label: "Manufacturer", group: "Identity", def: false, get: (d) => d.manufacturer ?? (d.name ? `#${d.name.manufacturer_code}` : "") },
  { id: "serial", label: "Serial", group: "Identity", def: false, get: (d) => d.product?.serial_code, cls: "font-mono text-[12.5px]" },
  { id: "modelver", label: "Hardware version", group: "Identity", def: false, get: (d) => d.product?.model_version },
  { id: "pcode", label: "Product code", group: "Identity", def: false, get: (d) => d.product?.product_code },
  { id: "n2kver", label: "NMEA 2000 version", group: "Identity", def: false, get: (d) => (d.product?.nmea2000_version != null ? (d.product.nmea2000_version / 1000).toFixed(3) : "") },
  { id: "cert", label: "Cert. level", group: "Identity", def: false, get: (d) => certText(d.product?.certification_level) },
  { id: "class", label: "Device class", group: "Identity", def: false, get: (d) => d.device_class_text ?? d.name?.device_class },
  { id: "function", label: "Device function", group: "Identity", def: false, get: (d) => d.device_function_text ?? "", cls: "max-w-[180px] truncate" },
  { id: "unique", label: "Unique number", group: "Identity", def: false, get: (d) => d.name?.unique_number },
  { id: "devinst", label: "Device instance", group: "Setup", def: false, get: (d) => devInst(d), cell: (d, c) => <EditCell device={d} value={devInst(d)} editable={c.editable && !!d.name} field="device_instance" kind="number" max={255} /> },
  { id: "sysinst", label: "System instance", group: "Setup", def: false, get: (d) => d.name?.system_instance, cell: (d, c) => <EditCell device={d} value={d.name?.system_instance ?? null} editable={c.editable && !!d.name} field="system_instance" kind="number" max={15} /> },
  { id: "datainst", label: "Data instances", group: "Setup", def: false, get: (d) => dataInfo(d).instances },
  { id: "source", label: "Source type", group: "Setup", def: false, get: (d) => dataInfo(d).source },
  { id: "label", label: "Label", group: "Setup", def: false, get: (d) => (d.labels.length === 1 ? d.labels[0].label : d.labels.length > 1 ? `${d.labels.length} channels` : "") },
  { id: "inst1", label: "Installation description 1", group: "Setup", def: false, get: (d) => d.config?.installation_description1, cell: (d, c) => <EditCell device={d} value={d.config?.installation_description1 ?? null} editable={c.editable && !!d.name} field="installation_description1" kind="text" /> },
  { id: "inst2", label: "Installation description 2", group: "Setup", def: false, get: (d) => d.config?.installation_description2, cell: (d, c) => <EditCell device={d} value={d.config?.installation_description2 ?? null} editable={c.editable && !!d.name} field="installation_description2" kind="text" /> },
  { id: "mfginfo", label: "Mfg information", group: "Setup", def: false, get: (d) => d.config?.manufacturer_information, cls: "max-w-[220px] truncate" },
];
const DEFAULT_COLS = COLS.filter((c) => c.def).map((c) => c.id);

type Chip = "all" | "online" | "offline" | "attention";
type View = "systems" | "table" | "map";

export function Devices() {
  const { devices, selectedDevice, selectDevice, metrics, toast, status, highlight, setHighlight, setFilter, setPage } = useApp();
  const [view, setViewRaw] = useState<View>(() => load("keelwire.devview", "systems"));
  const setView = (v: View) => {
    save("keelwire.devview", v);
    setViewRaw(v);
  };
  const [rowsBy, setRowsByRaw] = useState<"device" | "pgn">(() => load("keelwire.tablerows", "device"));
  const setRowsBy = (v: "device" | "pgn") => {
    save("keelwire.tablerows", v);
    setRowsByRaw(v);
  };
  const live = useLive(null, 3000);
  const srcHist = useApp((s) => s.srcHist);
  const [stats, refreshStats] = useStats(2000);
  const findings = useFindings();
  const names = usePgnNames();
  const [q, setQ] = useState("");
  const [chip, setChip] = useState<Chip>("all");
  const [kind, setKind] = useState("");
  const [maker, setMaker] = useState("");
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [visible, setVisibleRaw] = useState<string[]>(() => load("keelwire.devcols2", DEFAULT_COLS));
  const setVisible = (v: string[]) => {
    save("keelwire.devcols2", v);
    setVisibleRaw(v);
  };
  const [sort, setSort] = useState<{ id: string; dir: 1 | -1 }>(() => load("keelwire.devsort2", { id: "rate", dir: -1 as 1 | -1 }));
  const [chooser, setChooser] = useState(false);

  const attention = useMemo(() => new Set([...findings.flatMap((f) => f.devices), ...highlight]), [findings, highlight]);
  const ctx: Ctx = useMemo(() => {
    const pgns = new Map<number, number>();
    for (const p of stats.pgns) pgns.set(p.source, (pgns.get(p.source) ?? 0) + 1);
    return {
      load: new Map((metrics?.per_source ?? []).map(([a, , pct]) => [a, pct])),
      fps: new Map((metrics?.per_source ?? []).map(([a, f]) => [a, f])),
      rate: ratesBySource(metrics?.per_pgn),
      src: new Map(stats.sources.map((s) => [s.source, s])),
      pgns,
      nowUs: stats.now_us,
      editable: !!status?.armed,
      attention,
      own: status?.own_address ?? null,
    };
  }, [metrics, stats, devices, status, attention]);

  const cols = COLS.filter((c) => visible.includes(c.id));
  const sortCol = COLS.find((c) => c.id === sort.id) ?? COLS[0];
  const makers = [...new Set(devices.map((d) => d.manufacturer).filter(Boolean) as string[])].sort();
  const ql = q.trim().toLowerCase();
  const matches = (d: Device) => {
    if (!ql) return true;
    if (ql.startsWith("@")) return String(d.address ?? "") === ql.slice(1);
    const sent = Object.keys(d.observed);
    if (/^\d{4,6}$/.test(ql)) return sent.includes(ql) || String(d.product?.serial_code ?? "").includes(ql);
    const t = `${deviceLabel(d)} ${d.manufacturer ?? ""} ${d.device_function_text ?? ""} ${d.product?.serial_code ?? ""} ${d.config?.installation_description1 ?? ""} ${d.config?.installation_description2 ?? ""} ${sent.map((p) => names.get(Number(p)) ?? "").join(" ")}`.toLowerCase();
    return t.includes(ql);
  };
  const chipOk = (d: Device, c: Chip) => (c === "online" ? d.online : c === "offline" ? !d.online : c === "attention" ? attention.has(d.key) : true);
  const base = devices.filter((d) => matches(d) && (!kind || systemOf(d) === kind) && (!maker || d.manufacturer === maker));
  const shown = base
    .filter((d) => chipOk(d, chip))
    .sort((a, b) => {
      const x = sortCol.get(a, ctx),
        y = sortCol.get(b, ctx);
      if (x == null || x === "") return 1;
      if (y == null || y === "") return -1;
      const r = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y), undefined, { numeric: true });
      return r * sort.dir;
    });

  const exportCsv = () => {
    const esc = (v: unknown) => `"${String(v ?? "").replace(/"/g, '""')}"`;
    const lines = [cols.map((c) => esc(c.label)).join(","), ...shown.map((d) => cols.map((c) => esc(c.get(d, ctx))).join(","))];
    saveBlob(`devices-${new Date().toISOString().slice(0, 10)}.csv`, new Blob([lines.join("\r\n")], { type: "text/csv" }));
  };
  const toTraffic = (f: string) => {
    setFilter(f);
    setPage("sniffer");
  };
  const toggleOpen = (k: string) => {
    const n = new Set(open);
    if (n.has(k)) n.delete(k);
    else n.add(k);
    setOpen(n);
  };
  const chips: [Chip, string][] = [
    ["all", "All"],
    ["online", "Online"],
    ["offline", "Not responding"],
    ["attention", "Needs attention"],
  ];
  const tableByPgn = view === "table" && rowsBy === "pgn";

  return (
    <div className="flex h-full min-h-0">
      <div className={`min-w-0 flex-1 flex-col ${selectedDevice ? "hidden lg:flex" : "flex"}`}>
        <div className="no-print flex flex-wrap items-center gap-2 px-4 pb-3 pt-5 sm:px-7">
          <h1 className="mr-2 text-2xl font-semibold tracking-tight">
            Devices <span className="font-medium text-muted">{devices.length}</span>
          </h1>
          <label className="sr-only" htmlFor="dev-q">
            Search devices, @address, serial or PGN
          </label>
          <input id="dev-q" type="search" className="input w-80" placeholder={tableByPgn ? "PGN number or name, or @address" : "Name, @address, serial or PGN"} value={q} onChange={(e) => setQ(e.target.value)} />
          <span className="flex-1" />
          <div className="seg" role="group" aria-label="View">
            <button aria-pressed={view === "systems"} onClick={() => setView("systems")}>
              By system
            </button>
            <button aria-pressed={view === "table"} onClick={() => setView("table")}>
              <Table2 size={13} className="mr-1 inline" />
              Table
            </button>
            <button aria-pressed={view === "map"} onClick={() => setView("map")}>
              <Network size={13} className="mr-1 inline" />
              Map
            </button>
          </div>
          <button className="btn" onClick={() => post("/devices/refresh").then(() => toast("info", "Asked every device to identify itself")).catch((e) => toast("error", e.message))}>
            <RefreshCw size={14} /> Refresh
          </button>
          {highlight.length > 0 && (
            <span className="flex w-full items-center gap-2 rounded-lg border border-port/40 bg-port/10 px-3 py-1.5 text-sm text-port">
              {highlight.length} devices in an instancing conflict are highlighted.
              <button className="font-medium underline" onClick={() => setHighlight([])}>
                Clear
              </button>
            </span>
          )}
        </div>

        <div className="no-print flex flex-wrap items-center gap-2 px-4 pb-3 sm:px-7">
          {view === "table" && (
            <div className="seg mr-1" role="group" aria-label="Rows">
              <button aria-pressed={rowsBy === "device"} onClick={() => setRowsBy("device")}>
                By device
              </button>
              <button aria-pressed={rowsBy === "pgn"} onClick={() => setRowsBy("pgn")}>
                By PGN
              </button>
            </div>
          )}
          {!tableByPgn && (
            <>
              {chips.map(([c, l]) => (
                <button key={c} aria-pressed={chip === c} className={`chip-btn ${chip === c ? "chip-btn-on" : ""} ${c === "attention" && chip !== c && base.some((d) => chipOk(d, c)) ? "text-caution" : ""}`} onClick={() => setChip(c)}>
                  {l} {base.filter((d) => chipOk(d, c)).length}
                </button>
              ))}
              <label className="sr-only" htmlFor="dev-kind">
                Type
              </label>
              <select id="dev-kind" className="input !h-8 w-auto !rounded-full !text-[12.5px]" value={kind} onChange={(e) => setKind(e.target.value)}>
                <option value="">Type: any</option>
                {SYSTEMS.map((s) => (
                  <option key={s} value={s}>
                    {s}
                  </option>
                ))}
              </select>
              <label className="sr-only" htmlFor="dev-maker">
                Maker
              </label>
              <select id="dev-maker" className="input !h-8 w-auto !rounded-full !text-[12.5px]" value={maker} onChange={(e) => setMaker(e.target.value)}>
                <option value="">Maker: any</option>
                {makers.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            </>
          )}
          <span className="flex-1" />
          {view === "table" && (
            <>
              <span className="hidden text-[12.5px] text-muted 2xl:inline">Counting for {fmtDuration(stats.now_us - stats.since_us)}</span>
              <button className="btn btn-sm" onClick={() => post("/stats/reset").then(refreshStats).catch((e) => toast("error", e.message))} title="Start counting messages and data from zero">
                <RotateCcw size={13} /> Reset
              </button>
            </>
          )}
          {view === "table" && rowsBy === "device" && (
            <>
              <div className="relative">
                <button className="btn btn-sm" onClick={() => setChooser(!chooser)} aria-expanded={chooser}>
                  <Columns3 size={14} /> Columns
                </button>
                {chooser && (
                  <div className="absolute right-0 top-full z-20 mt-1 w-[520px] max-w-[90vw] rounded-xl border hairline bg-panel p-4 shadow-xl" onMouseLeave={() => setChooser(false)}>
                    <div className="mb-2 flex items-baseline justify-between">
                      <span className="text-sm font-semibold">Columns</span>
                      <button className="text-[12.5px] text-sea hover:underline" onClick={() => setVisible(DEFAULT_COLS)}>
                        Reset to default
                      </button>
                    </div>
                    <div className="grid grid-cols-2 gap-x-6 gap-y-3">
                      {GROUPS.map((g) => (
                        <div key={g}>
                          <p className="label mb-1">{g}</p>
                          {COLS.filter((c) => c.group === g && c.id !== "model").map((c) => (
                            <label key={c.id} className="flex min-h-[30px] cursor-pointer items-center gap-2 rounded px-1 text-sm hover:bg-shoal">
                              <input type="checkbox" checked={visible.includes(c.id)} onChange={(e) => setVisible(e.target.checked ? COLS.filter((x) => x.id === c.id || visible.includes(x.id)).map((x) => x.id) : visible.filter((v) => v !== c.id))} />
                              {c.label}
                            </label>
                          ))}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
              <button className="btn btn-sm" onClick={exportCsv} title="Export the device list as CSV" aria-label="Export CSV">
                <FileDown size={14} />
              </button>
              {!isMobileApp() && (
                <button className="btn btn-sm" onClick={() => window.print()} title="Print the device list" aria-label="Print">
                  <Printer size={14} />
                </button>
              )}
            </>
          )}
        </div>

        {view === "systems" ? (
          <SystemList devices={shown} live={live} srcHist={srcHist} selected={selectedDevice} highlight={[...attention]} ownAddress={status?.own_address ?? null} onSelect={(k) => selectDevice(k)} />
        ) : view === "map" ? (
          <NetworkMap devices={shown} all={devices} live={live} metrics={metrics} status={status} attention={attention} selected={selectedDevice} onSelect={(k) => selectDevice(k)} />
        ) : rowsBy === "pgn" ? (
          <PgnTable stats={stats} metrics={metrics} names={names} live={live} devices={devices} findings={findings} q={q} onDevice={(k) => selectDevice(k)} onTraffic={toTraffic} />
        ) : (
          <div className="print-area scroll-thin mx-4 mb-4 min-h-0 flex-1 overflow-auto rounded-xl border hairline bg-panel sm:mx-7">
            {devices.length === 0 ? (
              <p className="p-10 text-center text-muted">No devices yet. Devices appear as soon as they transmit on the bus.</p>
            ) : (
              <table className="tbl w-full text-[13px]">
                <thead className="sticky top-0 z-10 bg-panel">
                  <tr>
                    <th className="w-8" />
                    {["model", ...cols.filter((c) => c.id !== "model").map((c) => c.id)].map((id) => {
                      const c = COLS.find((x) => x.id === id)!;
                      return (
                        <th key={c.id} aria-sort={sort.id === c.id ? (sort.dir === 1 ? "ascending" : "descending") : undefined} className={c.cls?.includes("text-right") ? "text-right" : ""}>
                          <button
                            className="inline-flex items-center gap-1 hover:text-ink"
                            onClick={() => {
                              const n = { id: c.id, dir: (sort.id === c.id ? -sort.dir : typeof c.get(devices[0], ctx) === "number" ? -1 : 1) as 1 | -1 };
                              save("keelwire.devsort2", n);
                              setSort(n);
                            }}
                          >
                            {c.label}
                            {sort.id === c.id && (sort.dir === 1 ? <ChevronUp size={12} /> : <ChevronDown size={12} />)}
                          </button>
                        </th>
                      );
                    })}
                  </tr>
                </thead>
                <tbody>
                  {shown.map((d) => {
                    const isOpen = open.has(d.key);
                    const rowCols = [COLS[0], ...cols.filter((c) => c.id !== "model")];
                    return (
                      <Fragment key={d.key}>
                        <tr className={`cursor-pointer ${d.key === selectedDevice ? "bg-sea/10" : isOpen ? "bg-sea/5" : attention.has(d.key) ? "bg-caution/5" : "hover:bg-shoal/50"} ${d.online ? "" : "opacity-60"}`} onClick={() => selectDevice(d.key)}>
                          <td>
                            <button
                              className="flex h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-shoal"
                              aria-expanded={isOpen}
                              aria-label={`${isOpen ? "Hide" : "Show"} what ${deviceLabel(d)} sends`}
                              onClick={(e) => {
                                e.stopPropagation();
                                toggleOpen(d.key);
                              }}
                            >
                              {isOpen ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
                            </button>
                          </td>
                          {rowCols.map((c) => (
                            <td key={c.id} className={c.cls}>
                              {c.cell ? c.cell(d, ctx) : (c.get(d, ctx) ?? "")}
                            </td>
                          ))}
                        </tr>
                        {isOpen && (
                          <tr>
                            <td colSpan={rowCols.length + 1} className="bg-sea/5 !px-4 !pb-4 !pt-1">
                              <DevicePgns d={d} stats={stats} metrics={metrics} names={names} live={live[d.key]} onOpen={() => selectDevice(d.key)} onTraffic={toTraffic} />
                            </td>
                          </tr>
                        )}
                      </Fragment>
                    );
                  })}
                </tbody>
              </table>
            )}
          </div>
        )}
      </div>
      {sel(devices, selectedDevice) && <DevicePanel d={sel(devices, selectedDevice)!} live={live[selectedDevice!]} onClose={() => selectDevice(null)} />}
    </div>
  );
}

/** Expanded table row: every PGN the device sends, with rate, count and latest value. */
function DevicePgns({ d, stats, metrics, names, live, onOpen, onTraffic }: { d: Device; stats: Stats; metrics: Metrics | null; names: Map<number, string>; live: LiveMsg[] | undefined; onOpen: () => void; onTraffic: (f: string) => void }) {
  const rate = new Map((metrics?.per_pgn ?? []).filter(([, s]) => s === d.address).map(([p, , r]) => [p, r]));
  const rows = stats.pgns.filter((p) => p.source === d.address).sort((a, b) => (rate.get(b.pgn) ?? 0) - (rate.get(a.pgn) ?? 0) || a.pgn - b.pgn);
  return (
    <div className="overflow-hidden rounded-lg border hairline bg-panel" onClick={(e) => e.stopPropagation()}>
      <div className="flex flex-wrap items-center justify-between gap-2 border-b hairline px-3.5 py-2">
        <span className="text-[12.5px]">
          <strong className="font-semibold">
            What {deviceLabel(d)} @{d.address} sends
          </strong>{" "}
          <span className="text-muted">
            · {rows.length} PGNs{d.product?.serial_code ? ` · serial ${d.product.serial_code}` : ""}
            {d.product?.certification_level ? ` · certification level ${certText(d.product.certification_level)}` : ""}
          </span>
        </span>
        <span className="flex gap-4 text-[12.5px]">
          <button className="text-sea hover:underline" onClick={onOpen}>
            Open device panel
          </button>
          <button className="text-sea hover:underline" onClick={() => onTraffic(`src == ${d.address}`)}>
            Show in Traffic
          </button>
        </span>
      </div>
      <table className="w-full text-[12.5px]">
        <thead>
          <tr className="text-left text-[11.5px] text-muted">
            <th className="px-3.5 py-1.5 font-semibold">PGN</th>
            <th className="px-2 py-1.5 font-semibold">Name</th>
            <th className="px-2 py-1.5 text-right font-semibold">Rate</th>
            <th className="px-2 py-1.5 text-right font-semibold">Count</th>
            <th className="px-2 py-1.5 text-right font-semibold">Last</th>
            <th className="px-2 py-1.5 font-semibold">Latest value</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {rows.map((p) => {
            const m = live?.find((x) => x.pgn === p.pgn);
            const r = rate.get(p.pgn) ?? 0;
            const value = m
              ? m.fields
                  .filter((f) => !/^(reserved|spare)/i.test(f.id) && f.value !== null && !Array.isArray(f.value))
                  .slice(0, 3)
                  .map((f) => `${f.name} ${friendly(f)}`)
                  .join(" · ")
              : "";
            return (
              <tr key={p.pgn} className="border-t border-rule/50">
                <td className="px-3.5 py-1.5 font-mono text-sea">{p.pgn}</td>
                <td className="px-2 py-1.5">{names.get(p.pgn) ?? m?.name ?? `PGN ${p.pgn}`}</td>
                <td className="tnum px-2 py-1.5 text-right">{r > 0 ? (r < 0.1 ? `every ${Math.round(1 / r)} s` : `${r.toFixed(1)}/s`) : "—"}</td>
                <td className="tnum px-2 py-1.5 text-right text-muted">{fmtCount(p.messages)}</td>
                <td className="tnum px-2 py-1.5 text-right text-muted">{fmtAgo(p.last_us, stats.now_us)}</td>
                <td className="max-w-[420px] truncate px-2 py-1.5">{value}</td>
                <td className="px-3 py-1.5 text-right">
                  <button className="text-sea hover:underline" onClick={() => onTraffic(`src == ${d.address} && pgn == ${p.pgn}`)}>
                    Traffic →
                  </button>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

const sel = (devices: Device[], k: string | null) => devices.find((d) => d.key === k) ?? null;

function EditCell({ device, value, editable, field, kind, max }: { device: Device; value: string | number | null; editable: boolean; field: string; kind: "text" | "number"; max?: number }) {
  const toast = useApp((s) => s.toast);
  const [editing, setEditing] = useState(false);
  const [v, setV] = useState(String(value ?? ""));
  const [busy, setBusy] = useState(false);
  useEffect(() => setV(String(value ?? "")), [value]);
  if (!editable || !editing) {
    return (
      <span
        className={editable ? "-mx-1 block min-h-[1.4em] cursor-text rounded px-1 hover:bg-panel hover:ring-1 hover:ring-sea" : ""}
        onClick={(e) => {
          if (editable) {
            e.stopPropagation();
            setEditing(true);
          }
        }}
        title={editable ? "Click to edit" : undefined}
      >
        {value ?? ""}
      </span>
    );
  }
  const commit = async () => {
    if (String(value ?? "") === v) return setEditing(false);
    const body: Record<string, unknown> = {};
    if (kind === "number") {
      const n = Number(v);
      if (!Number.isInteger(n) || n < 0 || (max != null && n > max)) return toast("error", `Enter a whole number from 0 to ${max}`);
      body[field] = n;
    } else body[field] = v;
    setBusy(true);
    try {
      const r = await patch<{ key: string }>(`/devices/${device.key}/standard`, body);
      // Changing an instance changes the device's ISO NAME, which is its key.
      if (r.key && r.key !== device.key && useApp.getState().selectedDevice === device.key) useApp.getState().selectDevice(r.key);
      toast("ok", `${deviceLabel(device)} updated`);
      setEditing(false);
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <input
      autoFocus
      disabled={busy}
      className="input w-full !py-0.5"
      value={v}
      maxLength={kind === "text" ? 70 : 3}
      onClick={(e) => e.stopPropagation()}
      onChange={(e) => setV(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") {
          setV(String(value ?? ""));
          setEditing(false);
        }
      }}
    />
  );
}

function SystemList({ devices, live, srcHist, selected, highlight, ownAddress, onSelect }: { devices: Device[]; live: LiveMap; srcHist: Record<number, number[]>; selected: string | null; highlight: string[]; ownAddress: number | null; onSelect: (k: string) => void }) {
  if (devices.length === 0) return <p className="p-10 text-center text-muted">No devices yet. Devices appear as soon as they transmit on the bus.</p>;
  const groups = SYSTEMS.map((s) => ({ s, ds: devices.filter((d) => systemOf(d) === s).sort((a, b) => (a.address ?? 999) - (b.address ?? 999)) })).filter((g) => g.ds.length);
  return (
    <div className="scroll-thin min-h-0 flex-1 overflow-auto px-4 pb-6 sm:px-7">
      <div className="card overflow-hidden">
        <div className="hidden grid-cols-[12px_minmax(0,1.2fr)_40px_minmax(0,1.8fr)_80px_16px] gap-3 bg-shoal px-4 py-2 md:grid">
          <span />
          <span className="label">Device</span>
          <span className="label">Addr</span>
          <span className="label">Reporting now</span>
          <span className="label">Msgs/s</span>
          <span />
        </div>
        {groups.map((g) => (
          <section key={g.s} aria-label={g.s}>
            <h2 className="px-4 pb-1.5 pt-3.5 text-[11.5px] font-semibold uppercase tracking-wider text-muted">{g.s}</h2>
            {g.ds.map((d) => {
              const hl = highlight.includes(d.key);
              const own = ownAddress != null && d.address === ownAddress;
              const sum = own ? "This analyzer" : summary(live[d.key]) || (d.config?.installation_description1 ?? "");
              const hist = d.address != null ? (srcHist[d.address] ?? []) : [];
              return (
                <button
                  key={d.key}
                  onClick={() => onSelect(d.key)}
                  aria-current={selected === d.key ? "true" : undefined}
                  className={`grid w-full grid-cols-[12px_minmax(0,1fr)_40px_16px] items-center gap-3 border-t hairline px-4 py-2.5 text-left md:grid-cols-[12px_minmax(0,1.2fr)_40px_minmax(0,1.8fr)_80px_16px] ${selected === d.key ? "bg-sea/10 shadow-[inset_3px_0_0_rgb(var(--sea))]" : hl ? "bg-port/10" : "hover:bg-shoal/60"}`}
                >
                  <span className={`h-2.5 w-2.5 rounded-full ${hl ? "bg-port" : d.online ? "bg-stbd" : "bg-muted/50"}`} aria-label={hl ? "Instance conflict" : d.online ? "Online" : "Not responding"} role="img" />
                  <span className="min-w-0 truncate">
                    <span className={`font-semibold ${own ? "text-sea" : ""}`}>{deviceLabel(d)}</span> <span className="text-muted">{d.device_function_text ?? ""}</span>
                  </span>
                  <span className="font-mono text-muted">{d.address ?? "—"}</span>
                  <span className={`hidden truncate font-mono text-[13px] md:block ${hl ? "text-port" : ""}`}>{d.online ? sum : "Not responding"}</span>
                  <span className="hidden md:block">
                    <Sparkline values={hist} width={72} height={20} />
                  </span>
                  <ChevronRight size={16} className="text-muted" aria-hidden />
                </button>
              );
            })}
          </section>
        ))}
      </div>
    </div>
  );
}

function DevicePanel({ d, live, onClose }: { d: Device; live: LiveMsg[] | undefined; onClose: () => void }) {
  const { panelTab: tab, setPanelTab: setTab, setFilter, setPage } = useApp();
  const ok = d.online;
  const tabs: [PanelTab, string][] = [
    ["overview", "Overview"],
    ["settings", "Settings"],
    ["messages", "Messages"],
    ["identity", "Identity"],
  ];
  return (
    <aside aria-label={`${deviceLabel(d)} details`} className="flex w-full min-w-0 flex-col border-l hairline bg-panel lg:w-[46%] lg:min-w-[540px] lg:max-w-[760px]">
      <div className="px-5 pt-5 sm:px-7">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2.5">
              <h2 className="text-2xl font-semibold tracking-tight">{deviceLabel(d)}</h2>
              <span className={`chip ${ok ? "bg-stbd/10 text-stbd" : "bg-shoal text-muted"}`}>{ok ? "Online" : "Not responding"}</span>
            </div>
            <p className="mt-1 truncate text-muted">
              {[d.manufacturer, d.device_function_text?.toLowerCase(), d.address != null ? `address ${d.address}` : null, d.config?.installation_description1].filter(Boolean).join(" · ")}
            </p>
          </div>
          <button className="btn !px-2.5" onClick={onClose} aria-label="Close device">
            <X size={16} />
          </button>
        </div>
        <div role="tablist" className="mt-4 flex overflow-x-auto border-b hairline">
          {tabs.map(([t, l]) => (
            <button key={t} role="tab" className="tab" aria-selected={tab === t} onClick={() => setTab(t)}>
              {l}
            </button>
          ))}
        </div>
      </div>
      <div className="scroll-thin min-h-0 flex-1 overflow-auto">
        {tab === "overview" && (
          <PanelOverview
            d={d}
            live={live}
            onSettings={() => setTab("settings")}
            onTraffic={() => {
              setFilter(`src == ${d.address}`);
              setPage("sniffer");
            }}
          />
        )}
        {tab === "settings" && (d.name ? <DeviceSettings d={d} /> : <p className="p-7 text-muted">This sender hasn't identified itself, so it can't be configured. Use Refresh to ask it again.</p>)}
        {tab === "messages" && <LiveValues d={d} />}
        {tab === "identity" && <Identity d={d} />}
      </div>
    </aside>
  );
}

/** Big live values, a history of the main reading, setup and identity. */
function PanelOverview({ d, live, onSettings, onTraffic }: { d: Device; live: LiveMsg[] | undefined; onSettings: () => void; onTraffic: () => void }) {
  const rs = readings(live);
  const main = rs.find((r) => r.raw != null && r.field);
  const hist = useRef<{ key: string; label: string; pts: TimePoint[] }>({ key: "", label: "", pts: [] });
  const id = `${d.key}|${main?.label ?? ""}`;
  if (hist.current.key !== id) hist.current = { key: id, label: main?.label ?? "", pts: [] };
  if (main?.field && main.raw != null) {
    const t = Math.floor(Date.now() / 1000);
    const disp = parseFloat(friendly({ ...main.field, value: main.raw }).replace(/[^\d.+-].*$/, ""));
    const pts = hist.current.pts;
    if (!pts.length || pts[pts.length - 1].t !== t) pts.push({ t, v: Number.isFinite(disp) ? disp : main.raw });
    if (pts.length > 600) pts.shift();
  }
  const unit = main?.field ? friendly(main.field).replace(/^[-+\d.,\s°]*/, "").trim() : "";
  const metrics = useApp((s) => s.metrics);
  const rate = new Map((metrics?.per_pgn ?? []).filter(([, s]) => s === d.address).map(([p, , r]) => [p, r]));
  const sent = Object.keys(d.observed)
    .map(Number)
    .filter((p) => p >= 65280 && ![126208, 126464, 126993, 126996, 126998].includes(p))
    .map((p) => {
      const r = rate.get(p) ?? 0;
      const nm = live?.find((m) => m.pgn === p)?.name ?? `PGN ${p}`;
      return `${nm} ${r >= 0.95 ? `${r.toFixed(0)}/s` : r > 0 ? `every ${Math.round(1 / r)} s` : ""}`.trim();
    });
  return (
    <div className="flex flex-col gap-6 px-5 py-6 sm:px-7">
      {rs.length > 0 ? (
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {rs.slice(0, 6).map((r, i) => (
            <div key={i} className="rounded-xl bg-shoal px-4 py-3">
              <p className="label truncate">{r.label}</p>
              <p className={`tnum mt-1 truncate font-semibold tracking-tight ${r.value.length > 12 ? "text-[17px]" : "text-[26px]"}`}>{r.value}</p>
            </div>
          ))}
        </div>
      ) : (
        <p className="text-muted">{d.online ? "This device doesn't send readings the analyzer can summarise. See Messages for everything it sends." : "Not responding."}</p>
      )}
      {main && hist.current.pts.length > 1 && (
        <div>
          <div className="flex items-baseline justify-between">
            <h3 className="text-sm font-semibold">{main.label}, since you opened this</h3>
            <span className="label font-mono">
              {Math.min(...hist.current.pts.map((p) => p.v ?? Infinity)).toFixed(1)} – {Math.max(...hist.current.pts.map((p) => p.v ?? -Infinity)).toFixed(1)} {unit}
            </span>
          </div>
          <div className="mt-2">
            <TimeArea points={hist.current.pts} height={150} unit={unit} digits={2} label={`${main.label} history`} />
          </div>
        </div>
      )}
      <div>
        <div className="mb-2 flex items-center justify-between">
          <h3 className="text-sm font-semibold">Setup</h3>
          {d.name && (
            <button className="text-[13px] text-sea hover:underline" onClick={onSettings}>
              Change settings
            </button>
          )}
        </div>
        <dl className="grid grid-cols-[150px_1fr] gap-x-4 gap-y-2 text-[13px]">
          <dt className="text-muted">Installed</dt>
          <dd>{[d.config?.installation_description1, d.config?.installation_description2].filter(Boolean).join(" · ") || <span className="text-muted">No description yet</span>}</dd>
          <dt className="text-muted">Instances</dt>
          <dd className="font-mono">
            device {devInst(d) ?? "—"}
            {dataInfo(d).instances ? ` · data ${dataInfo(d).instances}` : ""}
          </dd>
          <dt className="text-muted">Sends</dt>
          <dd>{sent.length ? sent.join(" · ") : <span className="text-muted">Nothing yet</span>}</dd>
        </dl>
      </div>
      <div>
        <h3 className="mb-2 text-sm font-semibold">Identity</h3>
        <dl className="grid grid-cols-[150px_1fr] gap-x-4 gap-y-2 text-[13px]">
          <dt className="text-muted">Serial · software</dt>
          <dd className="font-mono">
            {d.product?.serial_code || "—"} · {d.product?.software_version || "—"}
          </dd>
          <dt className="text-muted">Product code · LEN</dt>
          <dd className="font-mono">
            {d.product?.product_code ?? "—"} · {d.product?.load_equivalency != null ? `${d.product.load_equivalency} (${d.product.load_equivalency * 50} mA)` : "—"}
          </dd>
          <dt className="text-muted">NMEA 2000</dt>
          <dd>
            {d.product?.nmea2000_version != null ? `Version ${(d.product.nmea2000_version / 1000).toFixed(3)}` : "—"}
            {d.product?.certification_level ? `, certification level ${certText(d.product.certification_level)}` : ""}
          </dd>
        </dl>
      </div>
      <div className="flex flex-wrap gap-2 border-t hairline pt-4">
        <button className="btn" onClick={onTraffic}>
          Show its traffic
        </button>
        {d.name && (
          <button className="btn" onClick={onSettings}>
            Change settings
          </button>
        )}
      </div>
    </div>
  );
}

function Identity({ d }: { d: Device }) {
  const [sub, setSub] = useState<"props" | "tx" | "rx" | "channels">("props");
  const lastUs = useApp((s) => s.packets[s.packets.length - 1]?.ts ?? d.last_seen_us);
  const rows: [string, ReactNode][] = [
    ["Node address", d.address ?? "none"],
    ["Manufacturer", d.manufacturer ?? (d.name ? `code ${d.name.manufacturer_code}` : "unknown")],
    ["Model", d.product?.model_id],
    ["Product code", d.product?.product_code],
    ["Serial number", d.product?.serial_code],
    ["Model version", d.product?.model_version],
    ["Software", d.product?.software_version],
    ["NMEA 2000 version", d.product?.nmea2000_version != null ? (d.product.nmea2000_version / 1000).toFixed(3) : undefined],
    ["Certification level", certText(d.product?.certification_level)],
    ["Load equivalency", d.product?.load_equivalency != null ? `${d.product.load_equivalency} (${d.product.load_equivalency * 50} mA)` : undefined],
    ["Industry group", d.name ? (["Global", "Highway", "Agriculture", "Construction", "Marine", "Industrial"][d.name.industry_group] ?? d.name.industry_group) : undefined],
    ["Device class", d.device_class_text ?? d.name?.device_class],
    ["Device function", d.device_function_text ?? d.name?.device_function],
    ["Device instance", devInst(d)],
    ["System instance", d.name?.system_instance],
    ["Unique number", d.name?.unique_number],
    ["ISO NAME", d.name ? <span className="font-mono text-xs">{d.key}</span> : undefined],
    ["Manufacturer information", d.config?.manufacturer_information],
    ["Heartbeat", d.heartbeat_interval_ms ? `every ${d.heartbeat_interval_ms / 1000} s` : undefined],
    ["Last heard", d.online ? "Transmitting now" : fmtAgo(d.last_seen_us, lastUs)],
    ["Frames", d.frames.toLocaleString()],
  ];
  return (
    <div className="px-5 py-5 sm:px-7">
      <div className="seg mb-4 w-fit" role="group" aria-label="Section">
        <button aria-pressed={sub === "props"} onClick={() => setSub("props")}>
          Properties
        </button>
        <button aria-pressed={sub === "tx"} onClick={() => setSub("tx")}>
          Transmit list
        </button>
        <button aria-pressed={sub === "rx"} onClick={() => setSub("rx")}>
          Receive list
        </button>
        <button aria-pressed={sub === "channels"} onClick={() => setSub("channels")}>
          Channels
        </button>
      </div>
      {sub === "props" && (
        <dl className="grid grid-cols-[180px_1fr] gap-x-4 gap-y-2 text-[13px]">
          {rows.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd>{v === undefined || v === null || v === "" ? <span className="text-muted">—</span> : v}</dd>
            </div>
          ))}
        </dl>
      )}
      {(sub === "tx" || sub === "rx") && <PgnList d={d} which={sub} />}
      {sub === "channels" &&
        (d.labels.length === 0 ? (
          <p className="text-sm text-muted">No channel labels received from this device.</p>
        ) : (
          <table className="tbl w-full text-sm">
            <thead>
              <tr>
                <th>HW channel</th>
                <th>Indicator</th>
                <th>Source</th>
                <th>Instance</th>
                <th>Label</th>
              </tr>
            </thead>
            <tbody>
              {d.labels.map((l, i) => (
                <tr key={i}>
                  <td>{l.hardware_channel ?? ""}</td>
                  <td>{l.indicator}</td>
                  <td>{l.data_source}</td>
                  <td>{l.instance}</td>
                  <td className="font-medium">{l.label}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ))}
    </div>
  );
}

interface FieldDefDto {
  order: number;
  id: string;
  name: string;
  type: string;
  bits: number | null;
  resolution: number;
  unit: string | null;
  lookup: string | null;
  match: number | null;
  description: string | null;
}

/** Field descriptions of one PGN. */
function PgnFields({ pgn }: { pgn: number }) {
  const [defs, setDefs] = useState<{ name: string; fields: FieldDefDto[] }[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  useEffect(() => {
    api<{ name: string; fields: FieldDefDto[]; fallback: boolean }[]>(`/pgn-db/${pgn}`)
      .then((r) => setDefs(r.filter((d) => !d.fallback)))
      .catch((e) => setErr((e as Error).message));
  }, [pgn]);
  if (err) return <p className="px-2 py-1 text-xs text-muted">Not in the PGN database.</p>;
  if (!defs) return <p className="px-2 py-1 text-xs text-muted">Loading…</p>;
  return (
    <div className="space-y-2 px-2 pb-2">
      {defs.map((d, i) => (
        <table key={i} className="w-full text-xs">
          {defs.length > 1 && <caption className="text-left font-semibold">{d.name}</caption>}
          <tbody>
            {d.fields
              .filter((f) => !/^reserved|^spare/i.test(f.id))
              .map((f) => (
                <tr key={f.order} className="align-top">
                  <td className="w-40 py-0.5 pr-2 font-medium">{f.name}</td>
                  <td className="py-0.5 text-muted">
                    {f.match != null ? `= ${f.match}` : [f.bits ? `${f.bits} bits` : f.type, f.unit, f.lookup ? `list: ${f.lookup.toLowerCase().replace(/_/g, " ")}` : null].filter(Boolean).join(", ")}
                    {f.description && <span className="block">{f.description}</span>}
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      ))}
    </div>
  );
}

function PgnList({ d, which }: { d: Device; which: "tx" | "rx" }) {
  const [names, setNames] = useState<Map<number, string>>(new Map());
  const [open, setOpen] = useState<number | null>(null);
  const metrics = useApp((s) => s.metrics);
  useEffect(() => {
    api<{ pgns: { pgn: number; name: string }[] }>("/pgn-db").then((r) => setNames(new Map(r.pgns.map((p) => [p.pgn, p.name])))).catch(() => {});
  }, []);
  const declared = which === "tx" ? d.tx_pgns : d.rx_pgns;
  const observed = Object.keys(d.observed).map(Number);
  const rates = new Map((metrics?.per_pgn ?? []).filter(([, s]) => s === d.address).map(([p, , r]) => [p, r]));
  const all = which === "tx" ? Array.from(new Set([...(declared ?? []), ...observed])).sort((a, b) => a - b) : (declared ?? []);
  return (
    <div>
      <p className="px-3 py-2 text-sm text-muted">
        {declared ? `${declared.length} PGNs reported by the device (126464).` : "The device has not reported this list (required only for certification level A)."}
        {which === "tx" && " Observed traffic is included."} Click a PGN for its fields.
      </p>
      <table className="tbl w-full text-sm">
        <thead>
          <tr><th className="w-5" /><th>PGN</th><th>Name</th>{which === "tx" && <><th>Declared</th><th>Seen</th><th>Rate /s</th></>}</tr>
        </thead>
        <tbody>
          {all.map((p) => (
            <Fragment key={p}>
              <tr className="cursor-pointer hover:bg-shoal/50" onClick={() => setOpen(open === p ? null : p)} aria-expanded={open === p}>
                <td className="text-muted">{open === p ? <Minus size={12} /> : <Plus size={12} />}</td>
                <td className="font-mono">{p}</td>
                <td>{names.get(p) ?? ""}</td>
                {which === "tx" && (
                  <>
                    <td>{declared?.includes(p) ? "yes" : ""}</td>
                    <td>{d.observed[String(p)] ? d.observed[String(p)].count.toLocaleString() : ""}</td>
                    <td>{rates.get(p)?.toFixed(1) ?? ""}</td>
                  </>
                )}
              </tr>
              {open === p && (
                <tr>
                  <td colSpan={which === "tx" ? 6 : 3} className="bg-shoal/30">
                    <PgnFields pgn={p} />
                  </td>
                </tr>
              )}
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}


/** Transmitted messages: one line per message (and per
 *  instance), expandable to all fields, live, with Clear. */
function LiveValues({ d }: { d: Device }) {
  const [items, setItems] = useState<LiveMsg[]>([]);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [clearedAt, setClearedAt] = useState(0);
  const [now, setNow] = useState(0);
  useEffect(() => {
    let alive = true;
    const tick = () =>
      api<LiveMsg[]>(`/devices/${d.key}/messages`)
        .then((r) => {
          if (!alive) return;
          setItems(r);
          setNow(Math.max(...r.map((m) => m.last_us), 0));
        })
        .catch(() => {});
    tick();
    const t = setInterval(tick, 1000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [d.key]);
  const id = (m: LiveMsg) => `${m.pgn}|${m.key ?? ""}`;
  const shown = items.filter((m) => m.last_us > clearedAt);
  const toggle = (k: string) => {
    const n = new Set(open);
    if (n.has(k)) n.delete(k);
    else n.add(k);
    setOpen(n);
  };
  const visibleFields = (m: LiveMsg) => m.fields.filter((f) => !/^(reserved|spare)/i.test(f.id) && f.value !== null);
  return (
    <div>
      <div className="flex items-center gap-2 border-b hairline px-3 py-2 text-sm">
        <span className="text-muted">{shown.length} messages, updated live</span>
        <button className="btn ml-auto" onClick={() => setOpen(open.size ? new Set() : new Set(shown.map(id)))}>{open.size ? "Collapse all" : "Expand all"}</button>
        <button className="btn" onClick={() => setClearedAt(now)}>Clear</button>
      </div>
      <ul className="divide-y divide-rule/60">
        {shown.map((m) => {
          const k = id(m);
          const isOpen = open.has(k);
          const fs = visibleFields(m);
          return (
            <li key={k} className="px-3 py-1.5">
              <button className="flex w-full items-baseline gap-2 text-left" onClick={() => toggle(k)} aria-expanded={isOpen}>
                <span className="text-muted">{isOpen ? <Minus size={12} /> : <Plus size={12} />}</span>
                <span className="font-mono text-xs text-muted">{m.pgn}</span>
                <span className="font-semibold">{m.name}</span>
                {m.key && <span className="truncate text-xs text-muted">{m.key}</span>}
                <span className="ml-auto shrink-0 font-mono text-xs text-muted">{fmtAgo(m.last_us, now)}</span>
              </button>
              {!isOpen && (
                <p className="truncate pl-5 text-xs text-muted">
                  {fs.slice(0, 4).map((f) => `${f.name}: ${Array.isArray(f.value) ? `${(f.value as unknown[]).length} items` : friendly(f)}`).join(" · ")}
                </p>
              )}
              {isOpen && (
                <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-3 pl-5 text-sm">
                  {fs.map((f, i) => (
                    <div key={i} className="contents">
                      <dt className="font-cond text-muted">{f.name}</dt>
                      <dd>{Array.isArray(f.value) ? `${(f.value as unknown[]).length} items` : friendly(f)}</dd>
                    </div>
                  ))}
                  <dt className="font-cond text-muted">Raw</dt>
                  <dd className="break-all font-mono text-xs">{m.data}</dd>
                </dl>
              )}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
