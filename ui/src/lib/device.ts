import type { Device } from "./types";

export const certText = (c: number | null | undefined) => (c == null ? "" : c === 1 ? "A" : c === 2 ? "B" : String(c));
export const devInst = (d: Device) => (d.name ? (d.name.device_instance_upper << 3) | d.name.device_instance_lower : null);

/** Instances and source types the device uses in its data messages, from
 *  the primary keys seen on the bus (e.g. "Instance=0, Type=Fuel"). */
export function dataInfo(d: Device): { instances: string; source: string } {
  const inst = new Set<string>();
  const src = new Set<string>();
  for (const o of Object.values(d.observed)) {
    for (const k of o.keys ?? []) {
      for (const part of k.split(", ")) {
        const [n, v] = part.split("=");
        if (!v) continue;
        if (/instance/i.test(n)) inst.add(v);
        else if (/^(type|source|fluid type|dc type|temperature source|humidity source|pressure source)$/i.test(n)) src.add(v);
      }
    }
  }
  const nums = [...inst].sort((a, b) => Number(a) - Number(b));
  return { instances: nums.join(", "), source: [...src].join(", ") };
}

/** Messages per second sent by each source address, from per-(PGN, source) rates. */
export function ratesBySource(perPgn: [number, number, number][] | undefined): Map<number, number> {
  const m = new Map<number, number>();
  for (const [, s, r] of perPgn ?? []) m.set(s, (m.get(s) ?? 0) + r);
  return m;
}
