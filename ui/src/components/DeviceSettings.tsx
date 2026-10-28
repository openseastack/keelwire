/** Device settings in plain-language sections. Edits are staged; one Apply
 *  sends them all (descriptions and instances via the standard 126208
 *  commands, message fields via 126208 Command with read-back, report rates
 *  via 126208 Request) and reports what each device confirmed. */
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { saveBlob } from "../lib/files";
import { Lock, Download, Upload } from "lucide-react";
import { api, patch, post } from "../lib/api";
import { friendly, METRIC } from "../lib/format";
import { useApp, deviceLabel } from "../lib/store";
import { useUnlock } from "./Lock";
import { AddressTab, Proprietary, type ConfigResp } from "./ConfigDialog";
import { DataFields } from "./DataFields";
import type { Device, Field, Units } from "../lib/types";
import type { LiveMsg } from "../lib/live";

interface FieldDef {
  order: number;
  id: string;
  name: string;
  type: string;
  bits: number | null;
  resolution: number;
  unit: string | null;
  lookup: string | null;
  match: number | null;
  primary_key: boolean;
  description: string | null;
}

const NM = new Set([59392, 59904, 60928, 126208, 126464, 126993, 126996, 126998, 65240]);
const READING = /^(source|type|fluidType|offset|capacity|nominalVoltage|chemistry|batteryType|supportsEqualization|temperatureCoefficient|peukertExponent|chargeEfficiencyFactor|dcType|temperatureSource|humiditySource)$/;
const CONFIG_PGNS = new Set([127513]);
const RATES: [string, string][] = [
  ["", "No change"],
  ["default", "Default rate"],
  ["100", "Every 0.1 s"],
  ["250", "Every 0.25 s"],
  ["500", "Every 0.5 s"],
  ["1000", "Every second"],
  ["2000", "Every 2 s"],
  ["5000", "Every 5 s"],
  ["10000", "Every 10 s"],
  ["off", "Off"],
];

/** How a numeric field is typed in: display units where it helps. */
function inputConv(unit: string | null, u: Units): { label: string; to: (si: number) => number; from: (v: number) => number } {
  const id = { to: (v: number) => v, from: (v: number) => v };
  switch (unit) {
    case "rad":
      return { label: "°", to: (v) => (v * 180) / Math.PI, from: (v) => (v * Math.PI) / 180 };
    case "K":
      return u.temperature === "F" ? { label: "°F", to: (v) => ((v - 273.15) * 9) / 5 + 32, from: (v) => ((v - 32) * 5) / 9 + 273.15 } : { label: "°C", to: (v) => v - 273.15, from: (v) => v + 273.15 };
    case "m":
      return u.depth === "ft" ? { label: "ft", to: (v) => v * 3.28084, from: (v) => v / 3.28084 } : { label: "m", ...id };
    case "L":
      return u.volume === "gal" ? { label: "gal", to: (v) => v / 3.785412, from: (v) => v * 3.785412 } : u.volume === "impgal" ? { label: "imp gal", to: (v) => v / 4.54609, from: (v) => v * 4.54609 } : { label: "L", ...id };
    default:
      return { label: unit ?? "", ...id };
  }
}
const round = (v: number) => Number(v.toFixed(3));

interface Row {
  id: string;
  section: "name" | "readings" | "instances" | "rates";
  label: string;
  hint?: string;
  current: string; // for display / "Was"
  currentRaw: string; // value the control shows when untouched
  kind: "text" | "number" | "select";
  options?: [string, string][];
  unit?: string;
}

interface Change {
  row: Row;
  to: string;
  toText: string;
}

export function DeviceSettings({ d }: { d: Device }) {
  const { status, toast, devices, metrics } = useApp();
  const armed = !!status?.armed;
  const units = status?.units ?? METRIC;
  const unlock = useUnlock();
  const [cfg, setCfg] = useState<ConfigResp | null>(null);
  const [cfgErr, setCfgErr] = useState<string | null>(null);
  const [msgs, setMsgs] = useState<LiveMsg[]>([]);
  const [defs, setDefs] = useState<Record<number, FieldDef[]>>({});
  const [lookups, setLookups] = useState<Record<string, [number, string][]>>({});
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);
  const model = d.model ?? d.product?.model_id ?? "";

  const loadCfg = () =>
    api<ConfigResp>(`/devices/${d.key}/config`)
      .then((c) => {
        setCfg(c);
        setCfgErr(null);
      })
      .catch((e) => setCfgErr((e as Error).message));
  const loadMsgs = () => api<LiveMsg[]>(`/devices/${d.key}/messages`).then(setMsgs).catch(() => {});
  useEffect(() => {
    setDraft({});
    setCfg(null);
    loadCfg();
    loadMsgs();
    const t = setInterval(loadMsgs, 3000);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.key]);

  const dataMsgs = useMemo(() => msgs.filter((m) => !NM.has(m.pgn) && m.pgn >= 65280 && !(m.pgn >= 65280 && m.pgn <= 65535)), [msgs]);
  useEffect(() => {
    for (const p of new Set(dataMsgs.map((m) => m.pgn))) {
      if (defs[p]) continue;
      api<{ fallback: boolean; fields: FieldDef[] }[]>(`/pgn-db/${p}`)
        .then((r) => {
          const def = r.find((x) => !x.fallback);
          if (!def) return;
          setDefs((o) => ({ ...o, [p]: def.fields }));
          for (const f of def.fields)
            if (f.lookup && !lookups[f.lookup]) api<[number, string][]>(`/lookup/${f.lookup}`).then((t) => setLookups((o) => ({ ...o, [f.lookup!]: t }))).catch(() => {});
        })
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataMsgs]);

  // Instances already used per PGN anywhere on the bus, to suggest a free one.
  const usedInst = useMemo(() => {
    const m = new Map<number, Set<number>>();
    for (const dv of devices)
      for (const [p, o] of Object.entries(dv.observed))
        for (const k of o.keys ?? [])
          for (const part of k.split(", ")) {
            const [n, v] = part.split("=");
            if (/instance/i.test(n) && v !== undefined && !Number.isNaN(Number(v))) {
              const s = m.get(Number(p)) ?? new Set();
              s.add(Number(v));
              m.set(Number(p), s);
            }
          }
    return m;
  }, [devices]);

  const rows: Row[] = [];
  const meta = new Map<string, { pgn: number; occ: LiveMsg; f: FieldDef } | { rate: number } | { std: string }>();
  if (cfg) {
    const s = cfg.standard;
    rows.push({ id: "std:installation_description1", section: "name", label: "Where it's installed", hint: "Shown on displays and in boat files", current: s.installation_description1 ?? "", currentRaw: s.installation_description1 ?? "", kind: "text" });
    rows.push({ id: "std:installation_description2", section: "name", label: "Note", hint: "Install date, installer, anything useful", current: s.installation_description2 ?? "", currentRaw: s.installation_description2 ?? "", kind: "text" });
    rows.push({ id: "std:device_instance", section: "instances", label: "Device instance", hint: `Only needed when there are two or more ${model || "devices"} of this kind`, current: String(s.device_instance), currentRaw: String(s.device_instance), kind: "number" });
    rows.push({ id: "std:system_instance", section: "instances", label: "System instance", hint: "Separates networks on boats with more than one backbone (0–15)", current: String(s.system_instance), currentRaw: String(s.system_instance), kind: "number" });
    for (const r of rows) meta.set(r.id, { std: r.id.slice(4) });
  }
  const multi = (pgn: number) => dataMsgs.filter((m) => m.pgn === pgn).length > 1;
  for (const m of dataMsgs) {
    const fds = defs[m.pgn];
    if (!fds) continue;
    for (const f of fds) {
      if (f.bits == null || f.bits > 32 || f.match != null || f.type.startsWith("STRING") || /^(reserved|spare|sid)$/i.test(f.id)) continue;
      const isInst = f.primary_key && /instance/i.test(f.id);
      const isReading = READING.test(f.id) || CONFIG_PGNS.has(m.pgn);
      if (!isInst && !isReading) continue;
      const cur = m.fields.find((x) => x.id === f.id);
      const raw = typeof cur?.value === "number" ? cur.value : null;
      const id = `fld:${m.pgn}|${m.key ?? ""}|${f.order}`;
      const where = `${m.name}${multi(m.pgn) && m.key ? ` · ${m.key}` : ""}`;
      if (isInst) {
        const used = usedInst.get(m.pgn) ?? new Set<number>();
        const free = [...Array(253).keys()].find((i) => !used.has(i));
        rows.push({ id, section: "instances", label: `${m.name} instance`, hint: multi(m.pgn) && m.key ? `${m.key}${free != null ? ` · ${free} is free` : ""}` : free != null ? `Instance ${free} is the lowest free one` : undefined, current: raw != null ? String(raw) : "—", currentRaw: raw != null ? String(raw) : "", kind: "number" });
      } else if (f.lookup) {
        const opts = (lookups[f.lookup] ?? []).map(([v, t]) => [String(v), t] as [string, string]);
        rows.push({ id, section: "readings", label: f.name, hint: where, current: cur ? friendly(cur) : "—", currentRaw: raw != null ? String(raw) : "", kind: "select", options: opts });
      } else {
        const cv = inputConv(f.unit, units);
        rows.push({ id, section: "readings", label: f.name, hint: f.description ? `${where}. ${f.description}` : where, current: cur ? friendly(cur as Field) : "—", currentRaw: raw != null ? String(round(cv.to(raw))) : "", kind: "number", unit: cv.label });
      }
      meta.set(id, { pgn: m.pgn, occ: m, f });
    }
  }
  const rateOf = new Map((metrics?.per_pgn ?? []).filter(([, s]) => s === d.address).map(([p, , r]) => [p, r]));
  for (const p of [...new Set(dataMsgs.map((m) => m.pgn))].sort((a, b) => a - b)) {
    const r = rateOf.get(p);
    const id = `rate:${p}`;
    rows.push({ id, section: "rates", label: dataMsgs.find((m) => m.pgn === p)?.name ?? `PGN ${p}`, hint: `PGN ${p}`, current: r ? (r >= 1 ? `about ${r.toFixed(1)} per second` : `about every ${Math.round(1 / r)} s`) : "not seen recently", currentRaw: "", kind: "select", options: RATES });
    meta.set(id, { rate: p });
  }

  const changes: Change[] = rows
    .filter((r) => draft[r.id] !== undefined && draft[r.id] !== "" && draft[r.id] !== r.currentRaw)
    .map((r) => ({ row: r, to: draft[r.id], toText: r.kind === "select" ? (r.options?.find((o) => o[0] === draft[r.id])?.[1] ?? draft[r.id]) : `${draft[r.id]}${r.unit ? " " + r.unit : ""}` }));

  const apply = async () => {
    if (!armed) return;
    setBusy(true);
    const done: string[] = [];
    const failed: string[] = [];
    let key = d.key;
    try {
      // 1. descriptions + system instance in one standard write
      const std: Record<string, unknown> = {};
      for (const c of changes) {
        const m = meta.get(c.row.id);
        if (m && "std" in m && m.std !== "device_instance") std[m.std] = m.std === "system_instance" ? Number(c.to) : c.to;
      }
      if (Object.keys(std).length) {
        try {
          const r = await patch<{ key: string }>(`/devices/${key}/standard`, std);
          if (r.key) key = r.key;
          done.push(...Object.keys(std).map((k) => (k.startsWith("installation") ? "description" : k.replace("_", " "))));
        } catch (e) {
          failed.push((e as Error).message);
        }
      }
      // 2. message fields, grouped per occurrence
      const groups = new Map<string, { pgn: number; occ: LiveMsg; values: { field: number; value: number }[]; names: string[] }>();
      for (const c of changes) {
        const m = meta.get(c.row.id);
        if (!m || !("f" in m)) continue;
        const n = Number(c.to);
        if (!Number.isFinite(n)) {
          failed.push(`${c.row.label}: not a number`);
          continue;
        }
        const g = groups.get(`${m.pgn}|${m.occ.key}`) ?? { pgn: m.pgn, occ: m.occ, values: [], names: [] };
        g.values.push({ field: m.f.order, value: m.f.lookup || (m.f.primary_key && /instance/i.test(m.f.id)) ? n : inputConv(m.f.unit, units).from(n) });
        g.names.push(c.row.label);
        groups.set(`${m.pgn}|${m.occ.key}`, g);
      }
      for (const g of groups.values()) {
        const select = (defs[g.pgn] ?? [])
          .filter((x) => x.primary_key)
          .map((x) => {
            const v = g.occ.fields.find((y) => y.id === x.id)?.value;
            return typeof v === "number" ? { field: x.order, value: v } : null;
          })
          .filter((x): x is { field: number; value: number } => x !== null);
        try {
          const r = await post<{ confirmed: boolean }>(`/devices/${key}/fields`, { pgn: g.pgn, select, values: g.values });
          done.push(...g.names.map((n) => (r.confirmed ? `${n} (confirmed)` : `${n} (sent, not seen yet)`)));
        } catch (e) {
          failed.push(`${g.names.join(", ")}: ${(e as Error).message}`);
        }
      }
      // 3. report rates
      for (const c of changes) {
        const m = meta.get(c.row.id);
        if (!m || !("rate" in m)) continue;
        const interval_ms = c.to === "default" ? null : c.to === "off" ? 0 : Number(c.to);
        try {
          await post(`/devices/${key}/pgn-rate`, { pgn: m.rate, interval_ms });
          done.push(`${c.row.label} rate`);
        } catch (e) {
          failed.push(`${c.row.label} rate: ${(e as Error).message}`);
        }
      }
      // 4. device instance last: it changes the device's NAME (its key)
      const di = changes.find((c) => c.row.id === "std:device_instance");
      if (di) {
        try {
          const r = await patch<{ key: string }>(`/devices/${key}/standard`, { device_instance: Number(di.to) });
          if (r.key) key = r.key;
          done.push("device instance");
        } catch (e) {
          failed.push(`Device instance: ${(e as Error).message}`);
        }
      }
    } finally {
      setBusy(false);
    }
    if (failed.length) toast("error", `Not applied: ${failed.join("; ")}`);
    if (done.length) toast("ok", `${deviceLabel(d)}: ${done.join(", ")}`);
    setDraft({});
    if (key !== d.key) useApp.getState().selectDevice(key);
    else {
      loadCfg();
      setTimeout(loadMsgs, 500);
    }
  };

  const saveFile = () => {
    const blob = new Blob([JSON.stringify({ format: "keelwire-device-config", version: 1, model, manufacturer: d.name?.manufacturer_code, software: d.product?.software_version, standard: cfg?.standard ?? null, proprietary: cfg?.proprietary ?? null }, null, 2)], { type: "application/json" });
    saveBlob(`${model || "device"}-${d.product?.serial_code || d.address}.kwcfg`, blob);
  };
  const loadFile = async (file: File) => {
    try {
      const j = JSON.parse(await file.text());
      if (j.format !== "keelwire-device-config") throw new Error("Not an Keelwire device settings file");
      if (j.model && model && j.model !== model) throw new Error(`This file is for a ${j.model}, not a ${model}`);
      const s = j.standard ?? {};
      setDraft((o) => ({
        ...o,
        ...(s.installation_description1 != null ? { "std:installation_description1": s.installation_description1 } : {}),
        ...(s.installation_description2 != null ? { "std:installation_description2": s.installation_description2 } : {}),
        ...(s.system_instance != null ? { "std:system_instance": String(s.system_instance) } : {}),
      }));
      toast("info", "Settings loaded from the file. Review the highlighted fields, then apply.");
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };

  const section = (key: Row["section"], title: string, note?: ReactNode) => {
    const rs = rows.filter((r) => r.section === key);
    if (!rs.length) return null;
    return (
      <section>
        <h3 className="mt-6 text-[15px] font-semibold">{title}</h3>
        {note && <p className="mt-0.5 text-[13px] text-muted">{note}</p>}
        <div className="mt-1">
          {rs.map((r) => (
            <SettingRow key={r.id} r={r} value={draft[r.id] ?? r.currentRaw} changed={draft[r.id] !== undefined && draft[r.id] !== "" && draft[r.id] !== r.currentRaw} onChange={(v) => setDraft({ ...draft, [r.id]: v })} />
          ))}
        </div>
      </section>
    );
  };

  return (
    <div className="flex min-h-full flex-col">
      <div className="flex-1 px-5 pb-6 sm:px-7">
        {!armed && (
          <div className="mt-5 flex flex-wrap items-center gap-3 rounded-xl bg-shoal px-4 py-3 text-sm">
            <Lock size={16} className="text-muted" />
            <span className="flex-1">You can prepare changes now. Unlock to apply them to the device.</span>
            <button className="btn btn-sm" onClick={() => unlock(true)}>
              Unlock changes
            </button>
          </div>
        )}
        {cfgErr && <p className="mt-4 rounded-lg border border-port/40 bg-port/10 p-3 text-sm">Could not read the device's settings: {cfgErr}</p>}
        {!cfg && !cfgErr && <p className="mt-6 text-muted">Reading settings from the device…</p>}
        {section("name", "Name and location")}
        {section("readings", "Readings", "Settings carried in the device's standard messages. Devices that don't support one reject it.")}
        {section("instances", "Instances", "Displays tell same-kind data apart by instance. Each sender of the same message needs its own.")}
        {section("rates", "How often it reports", "Not every device lets you change this; it will say so.")}

        <details className="mt-7 border-t hairline pt-4">
          <summary className="cursor-pointer font-semibold">Advanced: address, {d.name?.manufacturer_code === 137 ? "Maretron settings, " : ""}every message field, settings file</summary>
          <div className="mt-4 space-y-8">
            {d.name?.manufacturer_code === 137 || cfg?.proprietary ? (
              <div>
                <h4 className="mb-2 font-semibold">{d.name?.manufacturer_code === 137 ? "Maretron settings" : "Manufacturer settings"}</h4>
                <Proprietary d={d} cfg={cfg} model={model} armed={armed} onReload={loadCfg} />
              </div>
            ) : null}
            <div>
              <h4 className="mb-2 font-semibold">Every message field</h4>
              <DataFields d={d} armed={armed} />
            </div>
            <div>
              <h4 className="mb-2 font-semibold">Address</h4>
              <AddressTab d={d} armed={armed} />
            </div>
            <div>
              <h4 className="mb-2 font-semibold">Settings file</h4>
              <p className="mb-3 text-sm text-muted">Copy settings between devices of the same model, or keep a backup.</p>
              <div className="flex flex-wrap gap-2">
                <button className="btn" onClick={saveFile} disabled={!cfg}>
                  <Download size={14} /> Save to file
                </button>
                <button className="btn" onClick={() => fileRef.current?.click()}>
                  <Upload size={14} /> Load from file
                </button>
                <input ref={fileRef} type="file" accept=".kwcfg,.json" hidden onChange={(e) => e.target.files?.[0] && loadFile(e.target.files[0])} />
              </div>
            </div>
          </div>
        </details>
      </div>

      {changes.length > 0 && (
        <div className="sticky bottom-0 flex flex-wrap items-center gap-3 border-t hairline bg-panel px-5 py-3 shadow-[0_-6px_16px_rgb(0_0_0/0.06)] sm:px-7">
          <span className="chip bg-caution/10 text-caution">
            {changes.length} change{changes.length > 1 ? "s" : ""}
          </span>
          <span className="min-w-0 flex-1 truncate text-[13px] text-muted">{changes.map((c) => `${c.row.label}: ${c.row.current || "empty"} → ${c.toText}`).join(" · ")}</span>
          <button className="btn" onClick={() => setDraft({})} disabled={busy}>
            Discard
          </button>
          {armed ? (
            <button className="btn btn-primary" onClick={apply} disabled={busy}>
              {busy ? "Applying…" : `Apply to ${deviceLabel(d)}`}
            </button>
          ) : (
            <button className="btn btn-primary" onClick={() => unlock(true)}>
              <Lock size={14} /> Unlock to apply
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function SettingRow({ r, value, changed, onChange }: { r: Row; value: string; changed: boolean; onChange: (v: string) => void }) {
  const id = `set-${r.id.replace(/[^a-z0-9]/gi, "-")}`;
  return (
    <div className="grid gap-x-6 gap-y-1.5 border-t hairline py-3 sm:grid-cols-[minmax(0,1fr)_260px] sm:items-center">
      <div className="min-w-0">
        <label htmlFor={id} className="font-medium">
          {r.label}
        </label>
        {r.hint && <p className="truncate text-xs text-muted" title={r.hint}>{r.hint}</p>}
      </div>
      <div>
        <div className="flex items-center gap-2">
          {r.kind === "select" ? (
            <select id={id} className={`input w-full ${changed ? "changed" : ""}`} value={value} onChange={(e) => onChange(e.target.value)}>
              {r.currentRaw === "" && r.section !== "rates" && <option value="">—</option>}
              {(r.options ?? []).map(([v, t]) => (
                <option key={v} value={v}>
                  {r.section === "rates" && v === "" ? `No change (${r.current})` : t}
                </option>
              ))}
            </select>
          ) : (
            <input id={id} className={`input w-full ${r.kind === "number" ? "font-mono" : ""} ${changed ? "changed" : ""}`} inputMode={r.kind === "number" ? "decimal" : undefined} maxLength={r.kind === "text" ? 70 : undefined} value={value} onChange={(e) => onChange(e.target.value)} />
          )}
          {r.unit && <span className="w-10 shrink-0 text-xs text-muted">{r.unit}</span>}
        </div>
        {changed && r.section !== "rates" && <p className="mt-1 text-xs text-caution">Was {r.current || "empty"}</p>}
      </div>
    </div>
  );
}
