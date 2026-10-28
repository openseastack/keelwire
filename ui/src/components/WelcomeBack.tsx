// "Welcome back" banner: what changed on this boat since its last visit.
import { useEffect, useState } from "react";
import { api } from "../lib/api";
import { load, save, useApp } from "../lib/store";
import { fmtDate, useSession, vesselName } from "../lib/session";

interface Diff {
  previous: { dir: string; started_ms: number };
  items: { kind: string; text: string; plain: string }[];
}

const KIND: Record<string, [string, string]> = {
  added: ["Added", "bg-stbd/10 text-stbd"],
  gone: ["Gone", "bg-port/10 text-port"],
  software: ["Software", "bg-sea/10 text-sea"],
  setting: ["Setting", "bg-caution/10 text-caution"],
};

function ago(ms: number): string {
  const d = Math.round((Date.now() - ms) / 86_400_000);
  if (d < 1) return "earlier today";
  if (d < 45) return `${d} ${d === 1 ? "day" : "days"} ago`;
  const m = Math.round(d / 30.4);
  return m < 18 ? `${m} months ago` : `${(d / 365).toFixed(1)} years ago`;
}

export function WelcomeBack() {
  const s = useSession((x) => x.status);
  const devCount = useApp((x) => x.devices.length);
  const sess = s?.session;
  const [diff, setDiff] = useState<Diff | null>(null);
  const [open, setOpen] = useState(false);
  const [hidden, setHidden] = useState<string[]>(() => load("keelwire.welcome", []));
  const id = sess?.previous ? `${sess.id}` : null;
  useEffect(() => {
    if (!id) return setDiff(null);
    let alive = true;
    const get = () => api<Diff | null>("/session/since-last").then((d) => alive && setDiff(d)).catch(() => {});
    get();
    // Devices keep appearing for the first minute: refresh a few times.
    const t = setInterval(get, 15_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [id, devCount]);
  if (!sess || !id || !diff || hidden.includes(id)) return null;
  const count = (k: string) => diff.items.filter((i) => i.kind === k).length;
  const parts = [
    count("added") && `${count("added")} ${count("added") === 1 ? "device" : "devices"} added`,
    count("gone") && `${count("gone")} gone`,
    count("software") && `${count("software")} software ${count("software") === 1 ? "update" : "updates"}`,
    count("setting") && `${count("setting")} ${count("setting") === 1 ? "setting" : "settings"} changed`,
  ].filter(Boolean);
  const hide = () => {
    const n = [...hidden, id].slice(-50);
    save("keelwire.welcome", n);
    setHidden(n);
  };
  return (
    <div role="status" className="rounded-xl border border-sea/25 bg-sea/5 px-4 py-3">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <p className="text-[13.5px]">
          <strong className="font-semibold">Welcome back to {vesselName(sess.vessel)}.</strong>{" "}
          <span className="text-muted">
            Last visit {fmtDate(diff.previous.started_ms)}, {ago(diff.previous.started_ms)}. {parts.length ? `Since then: ${parts.join(", ")}.` : "No changes to devices, software or settings since then."}
          </span>
        </p>
        <div className="flex gap-2">
          {diff.items.length > 0 && (
            <button className="btn btn-sm btn-primary" onClick={() => setOpen((o) => !o)} aria-expanded={open}>
              {open ? "Hide" : "What changed"}
            </button>
          )}
          <button className="btn btn-sm" onClick={hide}>
            Dismiss
          </button>
        </div>
      </div>
      {open && (
        <ul className="mt-3 flex flex-col gap-1.5">
          {diff.items.map((i, k) => (
            <li key={k} className="flex items-start gap-2.5 text-[13.5px]">
              <span className={`chip shrink-0 ${KIND[i.kind]?.[1] ?? "bg-shoal"}`}>{KIND[i.kind]?.[0] ?? i.kind}</span>
              <span>{i.text}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
