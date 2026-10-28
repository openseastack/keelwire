/** One-click fixes for analysis findings. */
import { api, patch, post } from "./api";
import type { LiveMsg } from "./live";
import type { Finding } from "./types";

interface FieldDef {
  order: number;
  id: string;
  primary_key: boolean;
}

/** Move one occurrence of a message (e.g. "Fluid Level, Instance=0") to a
 *  new instance with a 126208 Command, selecting it by its key fields. */
export async function setDataInstance(deviceKey: string, pgn: number, occurrence: string | null, newInstance: number): Promise<boolean> {
  const defs = await api<{ fallback: boolean; fields: FieldDef[] }[]>(`/pgn-db/${pgn}`);
  const def = defs.find((d) => !d.fallback);
  if (!def) throw new Error(`PGN ${pgn} is not in the database`);
  const inst = def.fields.find((x) => x.primary_key && /instance/i.test(x.id));
  if (!inst) throw new Error("This message has no instance field");
  const msgs = await api<LiveMsg[]>(`/devices/${deviceKey}/messages`);
  const m = msgs.find((x) => x.pgn === pgn && (occurrence == null || x.key === occurrence)) ?? msgs.find((x) => x.pgn === pgn);
  const select = def.fields
    .filter((x) => x.primary_key)
    .map((x) => {
      const v = m?.fields.find((y) => y.id === x.id)?.value;
      return typeof v === "number" ? { field: x.order, value: v } : null;
    })
    .filter((x): x is { field: number; value: number } => x !== null);
  const r = await post<{ confirmed: boolean }>(`/devices/${deviceKey}/fields`, { pgn, select, values: [{ field: inst.order, value: newInstance }] });
  return r.confirmed;
}

export interface FixAction {
  label: string;
  run: () => Promise<string>;
  /** needs changes unlocked */
  writes: boolean;
}

export function fixFor(f: Finding): FixAction | null {
  if (f.kind === "instance_conflict" && f.target && f.pgn != null && f.new_instance != null) {
    return {
      label: `Set to instance ${f.new_instance}`,
      writes: true,
      run: async () => ((await setDataInstance(f.target!, f.pgn!, f.key ?? null, f.new_instance!)) ? `Moved to instance ${f.new_instance}, confirmed by the device` : `Sent; the device hasn't broadcast instance ${f.new_instance} yet`),
    };
  }
  if (f.kind === "duplicate_device_instance" && f.target && f.new_instance != null) {
    return {
      label: `Set device instance ${f.new_instance}`,
      writes: true,
      run: async () => {
        await patch(`/devices/${f.target}/standard`, { device_instance: f.new_instance });
        return `Device instance set to ${f.new_instance}`;
      },
    };
  }
  if (f.kind === "unclaimed_source" || f.kind === "no_product_info") {
    return {
      label: "Ask to identify",
      writes: false,
      run: async () => {
        await post("/devices/refresh");
        return "Asked every device to identify itself";
      },
    };
  }
  return null;
}
