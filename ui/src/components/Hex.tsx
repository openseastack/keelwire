import { memo } from "react";

/** Hex payload with port/starboard diff colouring. Keyed by seq so the fade restarts per packet. */
export const Hex = memo(function Hex({ data, diff, max = 24 }: { data: string; diff?: number[]; max?: number }) {
  const bytes: string[] = [];
  for (let i = 0; i < data.length && bytes.length < max; i += 2) bytes.push(data.slice(i, i + 2).toUpperCase());
  const more = data.length / 2 - bytes.length;
  return (
    <span className="font-mono text-[12.5px] tracking-tight">
      {bytes.map((b, i) => {
        const d = diff?.[i] ?? 0;
        return (
          <span key={i} className={d > 0 ? "diff-up" : d < 0 ? "diff-down" : undefined}>
            {b}
            {i < bytes.length - 1 ? " " : ""}
          </span>
        );
      })}
      {more > 0 && <span className="text-muted"> +{more}</span>}
    </span>
  );
});
