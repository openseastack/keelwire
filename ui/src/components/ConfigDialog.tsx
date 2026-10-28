import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { saveBlob } from "../lib/files";
import { X, Download, Upload, RotateCcw, Send } from "lucide-react";
import { api, patch, post, put } from "../lib/api";
import { friendly } from "../lib/format";
import { useApp, deviceLabel } from "../lib/store";
import type { Device } from "../lib/types";
import { DataFields } from "./DataFields";

export interface Standard {
  device_instance: number;
  system_instance: number;
  installation_description1: string | null;
  installation_description2: string | null;
  manufacturer_information: string | null;
}
interface ProprietarySection {
  schema: string;
  verified: boolean;
  values?: Record<string, unknown>;
  raw?: string;
  error?: string;
}
export interface ConfigResp {
  standard: Standard;
  proprietary: ProprietarySection[] | null;
  note?: string;
}

type Tab = "general" | "pgns" | "instances" | "address" | "proprietary";

/** Maretron commands observed on the wire (mbj4668/n2k; SSC300 manual). */
const MARETRON_COMMANDS: [number, string][] = [
  [93, "Compass: variation source (data FF = read)"],
  [94, "Compass: rate of turn / installation (data = 16-bit sub-command)"],
  [80, "Compass: resend calibration status"],
  [36, "Compass: installation offset"],
  [240, "Compass: reset and start calibration"],
  [49, "Tank: get level configuration"],
  [33, "Tank: set level configuration"],
];

/** Per-model tab layout, shown until schemas exist. */
const MODEL_TABS: Record<string, string> = {
  CLMD16: "General (16 breakers: type, rating, pickups, long-time delay, input signal), Control, Alarm, Counter, Flash, Grouping, Inputs, Tanks, Latch, Load Shedding, Logic (48 elements), Timer, Toggle, Discrete I/O, Advanced",
  CLMD12: "General (12 breakers: trip/inrush delay, dimming, rating, flash map, combine), Flash Map, Discrete I/O (36), Inputs, Advanced, Control",
  CKM12: "Keys 1–12 × LEDs 1–3: data instance and indicator",
  TLM100: "Tank type, number, damping, manual table or step-fill calibration (capacity is on the General tab)",
  TLA100: "Tank type, number, damping, manual table or step-fill calibration (capacity is on the General tab)",
  DCM100: "DC type, battery type, capacity, Peukert, charge efficiency, temperature",
  ACM100: "AC circuit type, device type, CT range and direction, damping",
};

export function ConfigDialog() {
  const { configDevice, openConfig, devices, status, toast } = useApp();
  const d = devices.find((x) => x.key === configDevice);
  const [tab, setTab] = useState<Tab>("general");
  const [cfg, setCfg] = useState<ConfigResp | null>(null);
  const [edit, setEdit] = useState<Standard | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadErr, setLoadErr] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  const get = async () => {
    if (!d) return;
    setBusy(true);
    setLoadErr(null);
    try {
      const r = await api<ConfigResp>(`/devices/${d.key}/config`);
      setCfg(r);
      setEdit({ ...r.standard });
    } catch (e) {
      setLoadErr((e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    setTab("general");
    setCfg(null);
    setEdit(null);
    if (d) get();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [configDevice]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", onKey);
    dialogRef.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  });

  const dirty = useMemo(() => {
    if (!cfg || !edit) return [] as (keyof Standard)[];
    return (Object.keys(edit) as (keyof Standard)[]).filter((k) => (edit[k] ?? "") !== (cfg.standard[k] ?? ""));
  }, [cfg, edit]);

  if (!d) return null;
  const armed = !!status?.armed;
  const model = d.model ?? d.product?.model_id ?? "";

  function close() {
    if (dirty.length && !confirm("You have changes that were not put to the device. Close anyway?")) return;
    openConfig(null);
  }

  const putStandard = async () => {
    if (!edit || !d) return;
    const body: Record<string, unknown> = {};
    for (const k of dirty) if (k !== "manufacturer_information") body[k] = edit[k];
    setBusy(true);
    try {
      const r = await patch<{ key: string }>(`/devices/${d.key}/standard`, body);
      toast("ok", `Settings put to ${deviceLabel(d)}`);
      if (r.key && r.key !== d.key) {
        // New instance means a new ISO NAME (the device key); reopen under it.
        if (useApp.getState().selectedDevice === d.key) useApp.getState().selectDevice(r.key);
        openConfig(r.key);
      } else await get();
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const saveFile = () => {
    const blob = new Blob([JSON.stringify({ format: "keelwire-device-config", version: 1, model, manufacturer: d.name?.manufacturer_code, software: d.product?.software_version, standard: edit, proprietary: cfg?.proprietary ?? null }, null, 2)], { type: "application/json" });
    saveBlob(`${model || "device"}-${d.product?.serial_code || d.address}.kwcfg`, blob);
  };

  const loadFile = async (f: File) => {
    try {
      const j = JSON.parse(await f.text());
      if (j.format !== "keelwire-device-config") throw new Error("Not an Keelwire device configuration file");
      if (j.model && model && j.model !== model) throw new Error(`This file is for a ${j.model}, not a ${model}`);
      if (j.software && d.product?.software_version && j.software !== d.product.software_version) toast("info", `File was saved from software ${j.software}; this device runs ${d.product.software_version}`);
      setEdit((e) => ({ ...(e as Standard), ...j.standard, device_instance: e?.device_instance ?? j.standard.device_instance }));
      toast("info", "Configuration loaded into the dialog. Review the red fields, then put it to the device.");
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };

  const tabs: [Tab, string][] = [
    ["general", "General"],
    ["pgns", "PGN enable / rate"],
    ["instances", "Data fields"],
    ["address", "Address"],
    ["proprietary", d.name?.manufacturer_code === 137 ? "Maretron settings" : "Proprietary"],
  ];

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-ink/40 p-4" onMouseDown={(e) => e.target === e.currentTarget && close()}>
      <div ref={dialogRef} tabIndex={-1} role="dialog" aria-modal="true" aria-labelledby="cfg-title" className="flex max-h-[92vh] w-full max-w-3xl flex-col rounded-lg border hairline bg-panel shadow-2xl outline-none">
        <header className="flex items-start justify-between gap-3 border-b hairline p-4">
          <div>
            <h2 id="cfg-title" className="text-xl font-semibold">Configure {deviceLabel(d)}</h2>
            <p className="text-sm text-muted">
              Address {d.address}, serial {d.product?.serial_code || "unknown"}, software {d.product?.software_version || "unknown"}
            </p>
          </div>
          <button className="btn !px-2" onClick={close} aria-label="Close">
            <X size={16} />
          </button>
        </header>
        {!armed && (
          <p className="border-b hairline bg-caution/15 px-4 py-2 text-sm">
            Transmit is not armed, so changes can be prepared but not put to the device. Arm TX in Settings first{status?.allow_tx ? "" : " (the server must be started with --allow-tx)"}.
          </p>
        )}
        <nav className="flex shrink-0 gap-1 overflow-x-auto border-b hairline px-3" role="tablist">
          {tabs.map(([t, l]) => (
            <button key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)} className={`whitespace-nowrap border-b-2 px-3 py-2 text-sm ${tab === t ? "border-sea font-semibold" : "border-transparent text-muted"}`}>
              {l}
            </button>
          ))}
        </nav>
        <div className="scroll-thin min-h-0 flex-1 overflow-auto p-4">
          {loadErr && <p className="mb-3 rounded border border-port/40 bg-port/10 p-2 text-sm">Could not read the configuration: {loadErr}</p>}
          {tab === "general" && edit && cfg && (
            <div className="grid max-w-xl grid-cols-[180px_1fr] items-center gap-x-4 gap-y-3">
              <Lbl changed={dirty.includes("device_instance")}>Device instance</Lbl>
              <input className={`input w-28 ${dirty.includes("device_instance") ? "!text-port" : ""}`} type="number" min={0} max={255} value={edit.device_instance} onChange={(e) => setEdit({ ...edit, device_instance: Number(e.target.value) })} />
              <Lbl changed={dirty.includes("system_instance")}>System instance</Lbl>
              <input className={`input w-28 ${dirty.includes("system_instance") ? "!text-port" : ""}`} type="number" min={0} max={15} value={edit.system_instance} onChange={(e) => setEdit({ ...edit, system_instance: Number(e.target.value) })} />
              <Lbl changed={dirty.includes("installation_description1")}>Installation description 1</Lbl>
              <input className={`input ${dirty.includes("installation_description1") ? "!text-port" : ""}`} maxLength={70} value={edit.installation_description1 ?? ""} onChange={(e) => setEdit({ ...edit, installation_description1: e.target.value })} placeholder="e.g. Engine room, port side" />
              <Lbl changed={dirty.includes("installation_description2")}>Installation description 2</Lbl>
              <input className={`input ${dirty.includes("installation_description2") ? "!text-port" : ""}`} maxLength={70} value={edit.installation_description2 ?? ""} onChange={(e) => setEdit({ ...edit, installation_description2: e.target.value })} placeholder="e.g. Installed 2026-10 by TM" />
              <Lbl>Manufacturer information</Lbl>
              <span className="text-sm text-muted">{cfg.standard.manufacturer_information || "—"}</span>
              {d.observed["127505"] && <TankCapacity d={d} armed={armed} />}
            </div>
          )}
          {tab === "general" && !cfg && !loadErr && <p className="text-muted">Reading configuration…</p>}
          {tab === "pgns" && <PgnRates d={d} armed={armed} />}
          {tab === "instances" && <DataFields d={d} armed={armed} />}
          {tab === "address" && <AddressTab d={d} armed={armed} />}
          {tab === "proprietary" && <Proprietary d={d} cfg={cfg} model={model} armed={armed} onReload={get} />}
        </div>
        <footer className="flex flex-wrap items-center gap-2 border-t hairline p-3">
          <button className="btn" onClick={() => fileRef.current?.click()}>
            <Upload size={14} /> Load config from file
          </button>
          <input ref={fileRef} type="file" accept=".kwcfg,.json" hidden onChange={(e) => e.target.files?.[0] && loadFile(e.target.files[0])} />
          <button className="btn" onClick={saveFile} disabled={!edit}>
            <Download size={14} /> Save config to file
          </button>
          <span className="flex-1" />
          <button className="btn" onClick={get} disabled={busy}>
            <RotateCcw size={14} /> Get config from device
          </button>
          <button className="btn btn-primary" onClick={putStandard} disabled={busy || !armed || dirty.length === 0}>
            <Send size={14} /> Put config to device{dirty.length ? ` (${dirty.length})` : ""}
          </button>
        </footer>
      </div>
    </div>
  );
}

function Lbl({ children, changed }: { children: ReactNode; changed?: boolean }) {
  return <label className={`font-cond text-[15px] ${changed ? "text-port" : "text-muted"}`}>{children}{changed ? " (changed)" : ""}</label>;
}

export function PgnRates({ d, armed }: { d: Device; armed: boolean }) {
  const toast = useApp((s) => s.toast);
  const metrics = useApp((s) => s.metrics);
  const rates = new Map((metrics?.per_pgn ?? []).filter(([, s]) => s === d.address).map(([p, , r]) => [p, r]));
  const periodic = Object.keys(d.observed).map(Number).filter((p) => p >= 127000 || (p >= 65280 && p <= 65535) || p === 126992);
  const [vals, setVals] = useState<Record<number, string>>({});
  const apply = async (pgn: number) => {
    const v = (vals[pgn] ?? "").trim().toLowerCase();
    const interval_ms = v === "default" || v === "" ? null : v === "disable" || v === "off" ? 0 : Number(v);
    if (interval_ms !== null && (!Number.isFinite(interval_ms) || interval_ms < 0)) return toast("error", "Use Default, Disable or a number of milliseconds");
    try {
      await post(`/devices/${d.key}/pgn-rate`, { pgn, interval_ms });
      toast("ok", `PGN ${pgn} ${interval_ms === 0 ? "disabled" : interval_ms === null ? "restored to default rate" : `set to every ${interval_ms} ms`}`);
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };
  return (
    <div>
      <p className="mb-3 text-sm text-muted">Turn periodic messages off, restore the default rate, or set a transmit interval in milliseconds. Not every device supports changing intervals.</p>
      <table className="tbl w-full text-sm">
        <thead><tr><th>PGN</th><th>Mode</th><th>Current rate</th><th>New setting</th><th /></tr></thead>
        <tbody>
          {periodic.map((p) => (
            <tr key={p}>
              <td className="font-mono">{p}</td>
              <td>{rates.get(p) ? "On" : "Off"}</td>
              <td>{rates.get(p) ? `${rates.get(p)!.toFixed(1)} /s` : "—"}</td>
              <td>
                <input className="input w-32" list="rate-opts" placeholder="Default" value={vals[p] ?? ""} onChange={(e) => setVals({ ...vals, [p]: e.target.value })} />
              </td>
              <td><button className="btn" disabled={!armed} onClick={() => apply(p)}>Apply</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      <datalist id="rate-opts"><option value="Default" /><option value="Disable" /><option value="100" /><option value="250" /><option value="1000" /></datalist>
    </div>
  );
}

export function AddressTab({ d, armed }: { d: Device; armed: boolean }) {
  const toast = useApp((s) => s.toast);
  const [a, setA] = useState(String(d.address ?? ""));
  return (
    <div className="max-w-lg space-y-3">
      <p className="text-sm text-muted">Commanded Address (PGN 65240) asks the device to move to another source address. Devices normally choose their own address; use this only to resolve conflicts.</p>
      <div className="flex items-center gap-2">
        <input className="input w-24" type="number" min={0} max={251} value={a} onChange={(e) => setA(e.target.value)} />
        <button className="btn" disabled={!armed} onClick={() => post(`/devices/${d.key}/address`, { address: Number(a) }).then(() => toast("ok", "Commanded address sent")).catch((e) => toast("error", e.message))}>
          Send commanded address
        </button>
      </div>
      <h3 className="pt-2 font-semibold">Address history</h3>
      <ul className="text-sm">
        {d.address_history.map(([t, ad], i) => (
          <li key={i}>Address {ad} at {(t / 1e6).toFixed(1)} s</li>
        ))}
      </ul>
    </div>
  );
}

export function TankCapacity({ d, armed }: { d: Device; armed: boolean }) {
  const toast = useApp((s) => s.toast);
  const [current, setCurrent] = useState<number | null>(null);
  const [v, setV] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api<{ pgn: number; fields: { id: string; value: unknown }[] }[]>(`/devices/${d.key}/messages`)
      .then((msgs) => {
        const f = msgs.find((m) => m.pgn === 127505)?.fields.find((x) => x.id === "capacity");
        if (typeof f?.value === "number") setCurrent(f.value);
      })
      .catch(() => {});
  }, [d.key]);
  const apply = async () => {
    const litres = Number(v);
    if (!Number.isInteger(litres) || litres < 1 || litres > 100000) return toast("error", "Enter whole litres from 1 to 100000");
    setBusy(true);
    try {
      const r = await post<{ confirmed: boolean }>(`/devices/${d.key}/tank-capacity`, { litres });
      toast(r.confirmed ? "ok" : "info", r.confirmed ? `Capacity set to ${litres} L and confirmed by the device` : `Capacity command accepted; the device has not reported ${litres} L yet`);
      setCurrent(litres);
      setV("");
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Lbl>Tank capacity</Lbl>
      <div className="flex items-center gap-2">
        <input className="input w-28" type="number" min={1} max={100000} placeholder={current != null ? String(current) : "litres"} value={v} onChange={(e) => setV(e.target.value)} />
        <span className="text-sm text-muted">L{current != null ? `, currently ${current}` : ""}</span>
        <button className="btn" disabled={!armed || busy || !v} onClick={apply}>Set capacity</button>
      </div>
    </>
  );
}

interface ActionInfo {
  id: string;
  group: string;
  label: string;
  help: string;
  control: boolean;
  experimental: boolean;
  value: string | null;
  value2: string | null;
}
interface MaretronReply {
  acknowledged: boolean;
  response: { command: number; data: string } | null;
  decoded: { description: string; fields: { id: string; name: string; value: unknown; unit?: string; text?: string }[] } | null;
}

function replyText(r: MaretronReply): string {
  if (r.decoded && r.decoded.fields.length) {
    const parts = r.decoded.fields
      .filter((f) => !["manufacturerCode", "industryCode", "productCode", "softwareCode", "command", "rotCommand"].includes(f.id) && !f.id.startsWith("reserved"))
      .map((f) => `${f.name}: ${friendly(f)}`);
    if (parts.length) return parts.join(", ");
  }
  if (r.response) return `Reply to command ${r.response.command}: ${r.response.data || "(no data)"}`;
  return r.acknowledged ? "Acknowledged by the device" : "No reply";
}

export function MaretronActions({ d, armed, expert }: { d: Device; armed: boolean; expert: boolean }) {
  const toast = useApp((s) => s.toast);
  const [actions, setActions] = useState<ActionInfo[]>([]);
  const [vals, setVals] = useState<Record<string, string>>({});
  const [results, setResults] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  useEffect(() => {
    api<ActionInfo[]>(`/devices/${d.key}/maretron/actions`).then(setActions).catch(() => setActions([]));
  }, [d.key]);
  if (actions.length === 0) return null;
  const run = async (a: ActionInfo) => {
    if (a.id === "compass.start_calibration" && !confirm("Start deviation calibration? The compass resets and you then need to turn the boat in slow circles until it reports completion.")) return;
    const num = (k: string) => (vals[k] === undefined || vals[k] === "" ? undefined : Number(vals[k]));
    setBusy(a.id);
    try {
      const r = await post<MaretronReply>(`/devices/${d.key}/maretron/action`, { id: a.id, value: num(a.id), value2: num(a.id + ".2") });
      setResults((x) => ({ ...x, [a.id]: replyText(r) }));
    } catch (e) {
      setResults((x) => ({ ...x, [a.id]: "" }));
      toast("error", (e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  const groups = Array.from(new Set(actions.map((a) => a.group)));
  return (
    <div className="space-y-3">
      <p className="text-sm text-muted">
        Commands documented in Maretron's SSC300 manual and observed on real devices by the open-source n2k project. Actions marked "unconfirmed" use a layout the available sources disagree on and need expert mode.
      </p>
      {groups.map((g) => (
        <section key={g} className="rounded-md border hairline">
          <h3 className="border-b hairline px-3 py-1.5 font-semibold">{g}</h3>
          <ul className="divide-y divide-rule/60">
            {actions.filter((a) => a.group === g).map((a) => {
              const blocked = (a.control && !armed) || (a.experimental && !expert);
              return (
                <li key={a.id} className="px-3 py-2">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="min-w-[220px] flex-1">
                      {a.label}
                      {a.experimental && <span className="ml-2 rounded bg-caution/20 px-1.5 py-0.5 text-xs">unconfirmed</span>}
                      {a.help && <span className="block text-xs text-muted">{a.help}</span>}
                    </span>
                    {a.value && <input className={`input ${a.value2 ? "w-36" : "w-28"}`} placeholder={a.value} aria-label={a.value} value={vals[a.id] ?? ""} onChange={(e) => setVals({ ...vals, [a.id]: e.target.value })} />}
                    {a.value2 && <input className="input w-36" placeholder={a.value2} aria-label={a.value2} value={vals[a.id + ".2"] ?? ""} onChange={(e) => setVals({ ...vals, [a.id + ".2"]: e.target.value })} />}
                    <button className="btn" disabled={blocked || busy !== null} title={blocked ? (a.control && !armed ? "Arm TX in Settings first" : "Enable expert mode in Settings first") : undefined} onClick={() => run(a)}>
                      {busy === a.id ? "Waiting…" : a.control ? "Send" : "Read"}
                    </button>
                  </div>
                  {results[a.id] && <p className="mt-1 text-sm font-medium">{results[a.id]}</p>}
                </li>
              );
            })}
          </ul>
        </section>
      ))}
    </div>
  );
}

export function Proprietary({ d, cfg, model, armed, onReload }: { d: Device; cfg: ConfigResp | null; model: string; armed: boolean; onReload: () => void }) {
  const { status, toast } = useApp();
  const expert = !!status?.expert;
  const [op, setOp] = useState(93);
  const [payload, setPayload] = useState("FF");
  const [reply, setReply] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const isMaretron = d.name?.manufacturer_code === 137;
  const exchange = async () => {
    try {
      const r = await post<MaretronReply>(`/devices/${d.key}/maretron`, { command: op, data: payload });
      setReply(replyText(r) + (r.response ? `\nRaw reply: command ${r.response.command}, data ${r.response.data || "(none)"}` : ""));
    } catch (e) {
      setReply(null);
      toast("error", (e as Error).message);
    }
  };
  return (
    <div className="space-y-4">
      {isMaretron && <MaretronActions d={d} armed={armed} expert={expert} />}
      {cfg?.proprietary && cfg.proprietary.length > 0 ? (
        cfg.proprietary.map((s) => (
          <section key={s.schema} className="rounded-md border hairline p-3">
            <h3 className="font-semibold">{s.schema} {!s.verified && <span className="ml-1 rounded bg-caution/20 px-1.5 py-0.5 text-xs font-normal">unverified layout</span>}</h3>
            {s.error && <p className="text-sm text-port">{s.error}</p>}
            {s.values && (
              <>
                <textarea className="input mt-2 h-48 w-full font-mono text-xs" value={drafts[s.schema] ?? JSON.stringify(s.values, null, 2)} onChange={(e) => setDrafts({ ...drafts, [s.schema]: e.target.value })} />
                <div className="mt-2 flex gap-2">
                  <button
                    className="btn"
                    disabled={!armed || (!s.verified && !expert)}
                    onClick={async () => {
                      try {
                        await put(`/devices/${d.key}/config`, { schema: s.schema, values: JSON.parse(drafts[s.schema] ?? JSON.stringify(s.values)), experimental: !s.verified });
                        toast("ok", "Proprietary settings put to device");
                        onReload();
                      } catch (e) {
                        toast("error", (e as Error).message);
                      }
                    }}
                  >
                    Put to device
                  </button>
                </div>
              </>
            )}
          </section>
        ))
      ) : (
        MODEL_TABS[model] && (
          <div className="rounded-md border hairline bg-shoal/30 p-3 text-sm">
            <p>
              Not available yet for the {model}: {MODEL_TABS[model]}. These appear here once the message layout for this model is known and added as a schema.
            </p>
          </div>
        )
      )}
      {isMaretron && (
        <section className="rounded-md border hairline p-3">
          <h3 className="font-semibold">Maretron message tool</h3>
          <p className="text-sm text-muted">Sends any Maretron command to this device, the way Maretron's own tools do (a 126208 command carrying the device's product code), and shows the reply. Some commands change settings. Requires expert mode{expert ? "" : " (enable it in Settings)"} and armed TX.</p>
          <div className="mt-2 flex flex-wrap items-center gap-2">
            <input className="input w-20" type="number" min={0} max={255} aria-label="Command number" value={op} onChange={(e) => setOp(Number(e.target.value))} disabled={!expert} list="maretron-cmds" />
            <datalist id="maretron-cmds">
              {MARETRON_COMMANDS.map(([v, n]) => (
                <option key={v} value={v}>{n}</option>
              ))}
            </datalist>
            <input className="input w-64 font-mono" placeholder="data hex (optional)" value={payload} onChange={(e) => setPayload(e.target.value)} disabled={!expert} />
            <button className="btn" onClick={exchange} disabled={!expert || !armed}>Send</button>
            <span className="text-xs text-muted">{MARETRON_COMMANDS.find(([v]) => v === op)?.[1] ?? "Unknown command"}</span>
          </div>
          {reply && <pre className="mt-2 overflow-auto rounded bg-shoal/40 p-2 font-mono text-xs">{reply}</pre>}
        </section>
      )}
    </div>
  );
}
