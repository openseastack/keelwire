import { useEffect, useMemo, useState } from "react";
import { api, post } from "../lib/api";
import { friendly } from "../lib/format";
import { useApp } from "../lib/store";
import type { Device, Field } from "../lib/types";

/** Settings stored in standard message fields, changed with a 126208
 *  Command and confirmed from the device's next broadcast. Covers what
 *  Settings many devices expose: data instances, temperature /
 *  humidity / pressure sources, fluid type, depth transducer offset,
 *  battery configuration, and (expert) any other field. */

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
interface PgnDef {
  name: string;
  fallback: boolean;
  fields: FieldDef[];
}
interface Msg {
  pgn: number;
  name: string;
  key: string | null;
  fields: Field[];
}

/** Fields worth offering without expert mode. */
const COMMON = /^(instance|.*Instance|source|type|fluidType|offset|capacity|nominalVoltage|chemistry|batteryType|supportsEqualization|temperatureCoefficient|peukertExponent|chargeEfficiencyFactor|dcType)$/;
/** Configuration PGNs where every field is a setting. */
const CONFIG_PGNS = new Set([127513]);

/** Input units: degrees for angles, °C for temperatures, SI otherwise. */
function toInput(v: number, unit: string | null): number {
  if (unit === "rad") return Number(((v * 180) / Math.PI).toFixed(2));
  if (unit === "K") return Number((v - 273.15).toFixed(2));
  return v;
}
function fromInput(v: number, unit: string | null): number {
  if (unit === "rad") return (v * Math.PI) / 180;
  if (unit === "K") return v + 273.15;
  return v;
}
const inputUnit = (u: string | null) => (u === "rad" ? "°" : u === "K" ? "°C" : (u ?? ""));

export function DataFields({ d, armed }: { d: Device; armed: boolean }) {
  const { toast, status } = useApp();
  const expert = !!status?.expert;
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [defs, setDefs] = useState<Record<number, FieldDef[]>>({});
  const [lookups, setLookups] = useState<Record<string, [number, string][]>>({});
  const [all, setAll] = useState(false);
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);

  const refresh = () => api<Msg[]>(`/devices/${d.key}/messages`).then(setMsgs).catch(() => {});
  useEffect(() => {
    refresh();
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [d.key]);

  // Data PGNs only (skip network management).
  const dataMsgs = useMemo(() => msgs.filter((m) => m.pgn >= 65280 && ![126208, 126464, 126993, 126996, 126998, 60928, 59392, 59904].includes(m.pgn)), [msgs]);

  useEffect(() => {
    for (const p of new Set(dataMsgs.map((m) => m.pgn))) {
      if (defs[p]) continue;
      api<PgnDef[]>(`/pgn-db/${p}`)
        .then((r) => {
          const def = r.find((x) => !x.fallback);
          if (!def) return;
          setDefs((o) => ({ ...o, [p]: def.fields }));
          for (const f of def.fields) {
            if (f.lookup && !lookups[f.lookup]) {
              api<[number, string][]>(`/lookup/${f.lookup}`).then((t) => setLookups((o) => ({ ...o, [f.lookup!]: t }))).catch(() => {});
            }
          }
        })
        .catch(() => {});
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dataMsgs]);

  const settable = (pgn: number, f: FieldDef) =>
    f.bits != null && f.bits <= 32 && f.match == null && !/^(reserved|spare|sid)$/i.test(f.id) && !f.type.startsWith("STRING") && (all || CONFIG_PGNS.has(pgn) || COMMON.test(f.id));

  const apply = async (m: Msg, f: FieldDef) => {
    const k = `${m.pgn}|${m.key}|${f.order}`;
    const raw = edits[k];
    if (raw === undefined || raw === "") return;
    const n = Number(raw);
    if (!Number.isFinite(n)) return toast("error", `${f.name}: enter a number`);
    const fdefs = defs[m.pgn] ?? [];
    // Identify the occurrence on multi-channel devices by its key fields.
    const select = fdefs
      .filter((x) => x.primary_key)
      .map((x) => {
        const cur = m.fields.find((y) => y.id === x.id)?.value;
        return typeof cur === "number" ? { field: x.order, value: cur } : null;
      })
      .filter((x): x is { field: number; value: number } => x !== null);
    setBusy(k);
    try {
      const r = await post<{ confirmed: boolean }>(`/devices/${d.key}/fields`, { pgn: m.pgn, select, values: [{ field: f.order, value: f.lookup ? n : fromInput(n, f.unit) }] });
      toast(r.confirmed ? "ok" : "info", r.confirmed ? `${f.name} changed and confirmed by the device` : `${f.name}: the device accepted the command but has not broadcast the new value yet`);
      setEdits((o) => {
        const c = { ...o };
        delete c[k];
        return c;
      });
      setTimeout(refresh, 600);
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(null);
    }
  };

  const groups = dataMsgs
    .map((m) => ({ m, fields: (defs[m.pgn] ?? []).filter((f) => settable(m.pgn, f)) }))
    .filter((g) => g.fields.length > 0);

  return (
    <div className="space-y-3">
      <p className="text-sm text-muted">
        Settings that live in the device's standard messages (instances, sources, fluid type, depth offset, battery setup). Changes are sent as an NMEA 2000 Command (126208) and confirmed when the device next
        broadcasts the message. Devices that don't support a field reject the command.
      </p>
      <label className={`flex items-center gap-2 text-sm ${expert ? "" : "text-muted"}`}>
        <input type="checkbox" disabled={!expert} checked={all} onChange={(e) => setAll(e.target.checked)} />
        Show every field {expert ? "" : "(needs expert mode in Settings)"}
      </label>
      {groups.length === 0 && <p className="text-sm text-muted">{dataMsgs.length ? "No settable fields in this device's messages." : "Waiting for this device's data messages…"}</p>}
      {groups.map(({ m, fields }) => (
        <section key={`${m.pgn}|${m.key}`} className="rounded-md border hairline">
          <h3 className="flex items-baseline gap-2 border-b hairline px-3 py-1.5">
            <span className="font-semibold">{m.name}</span>
            <span className="font-mono text-xs text-muted">{m.pgn}</span>
            {m.key && <span className="text-xs text-muted">{m.key}</span>}
          </h3>
          <table className="w-full text-sm">
            <tbody>
              {fields.map((f) => {
                const k = `${m.pgn}|${m.key}|${f.order}`;
                const cur = m.fields.find((x) => x.id === f.id);
                const opts = f.lookup ? lookups[f.lookup] : undefined;
                const curNum = typeof cur?.value === "number" ? cur.value : null;
                return (
                  <tr key={f.order} className="border-b hairline last:border-0">
                    <td className="w-48 px-3 py-1.5">{f.name}</td>
                    <td className="px-2 py-1.5 text-muted">{cur ? friendly(cur) : "—"}</td>
                    <td className="w-56 px-2 py-1">
                      {opts ? (
                        <select className="input w-full" value={edits[k] ?? (curNum != null ? String(curNum) : "")} onChange={(e) => setEdits({ ...edits, [k]: e.target.value })}>
                          {curNum == null && <option value="">—</option>}
                          {opts.map(([v, t]) => (
                            <option key={v} value={v}>{t}</option>
                          ))}
                        </select>
                      ) : (
                        <span className="flex items-center gap-1">
                          <input
                            className="input w-full"
                            inputMode="decimal"
                            placeholder={curNum != null ? String(toInput(curNum, f.unit)) : ""}
                            value={edits[k] ?? ""}
                            onChange={(e) => setEdits({ ...edits, [k]: e.target.value })}
                            onKeyDown={(e) => e.key === "Enter" && apply(m, f)}
                          />
                          <span className="w-8 text-xs text-muted">{inputUnit(f.unit)}</span>
                        </span>
                      )}
                    </td>
                    <td className="w-20 px-2 py-1 text-right">
                      <button className="btn" disabled={!armed || edits[k] === undefined || edits[k] === "" || busy === k} onClick={() => apply(m, f)}>
                        {busy === k ? "…" : "Set"}
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </section>
      ))}
    </div>
  );
}
