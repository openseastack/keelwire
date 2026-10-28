import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, CircleAlert, Info, Circle, Square, CheckCircle2, ArrowRight } from "lucide-react";
import { useApp, deviceLabel, load, save } from "../lib/store";
import { api, post } from "../lib/api";
import { TimeArea, Sparkline, BarList, type TimePoint } from "../lib/charts";
import { useLive, readings, systemOf, SYSTEMS, type System } from "../lib/live";
import { fixFor } from "../lib/fixes";
import { WelcomeBack } from "../components/WelcomeBack";
import type { Finding } from "../lib/types";

interface LoadPoint {
  t: number;
  load: number | null;
  frames: number;
}

const RANGES: [number, string][] = [
  [60, "1 min"],
  [600, "10 min"],
  [3600, "1 h"],
];

export function Overview() {
  const { devices, metrics, status, alerts } = useApp();
  const [findings, setFindings] = useState<Finding[]>([]);
  const [range, setRangeRaw] = useState<number>(() => load("keelwire.loadrange", 600));
  const setRange = (r: number) => {
    save("keelwire.loadrange", r);
    setRangeRaw(r);
  };
  const [hist, setHist] = useState<LoadPoint[]>([]);
  const [dismissed, setDismissedRaw] = useState<string[]>(() => load("keelwire.dismissed", []));
  const live = useLive();

  useEffect(() => {
    let alive = true;
    const tick = () => api<Finding[]>("/analysis/instances").then((f) => alive && setFindings(f)).catch(() => {});
    tick();
    const t = setInterval(tick, 3000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);
  useEffect(() => {
    let alive = true;
    const tick = () => api<LoadPoint[]>(`/metrics/history?seconds=${range}`).then((h) => alive && setHist(h)).catch(() => {});
    tick();
    const t = setInterval(tick, range > 600 ? 5000 : 2000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [range]);

  const fid = (f: Finding) => `${f.kind}|${f.pgn}|${f.key ?? ""}|${[...f.devices].sort().join(",")}`;
  // A fixed finding can linger until the old instance ages out of the
  // analysis (30 s); hide it meanwhile.
  const [fixed, setFixed] = useState<Record<string, number>>({});
  const fixedNow = (f: Finding) => (fixed[`${f.kind}|${f.pgn}|${[...f.devices].sort().join(",")}`] ?? 0) > Date.now();
  const markFixed = (f: Finding) => setFixed((o) => ({ ...o, [`${f.kind}|${f.pgn}|${[...f.devices].sort().join(",")}`]: Date.now() + 40_000 }));
  const open = findings.filter((f) => f.severity !== "info" && !dismissed.includes(fid(f)) && !fixedNow(f));
  const info = findings.filter((f) => f.severity === "info" && !dismissed.includes(fid(f)) && !fixedNow(f));
  const dismiss = (f: Finding) => {
    const n = [...dismissed, fid(f)];
    save("keelwire.dismissed", n);
    setDismissedRaw(n);
  };

  const online = devices.filter((d) => d.online).length;
  const offline = devices.filter((d) => !d.online && d.name).length;
  const len = devices.filter((d) => d.online).reduce((a, d) => a + (d.product?.load_equivalency ?? 0), 0);
  // Devices that haven't reported Product Information (126996) add nothing to the total.
  const noLen = devices.filter((d) => d.online && d.product?.load_equivalency == null).length;
  const pts: TimePoint[] = hist.map((h) => ({ t: h.t, v: h.load }));
  const last60 = hist.slice(-60).map((h) => h.load ?? 0);
  const peak = Math.max(0, ...hist.map((h) => h.load ?? 0));
  const byAddr = new Map(devices.filter((d) => d.address != null).map((d) => [d.address!, d]));
  const busiest = [...(metrics?.per_source ?? [])].sort((a, b) => b[2] - a[2]).slice(0, 6);
  const now = new Date();

  const headline = !status?.interface && status?.offline ? `Boat file: ${status.offline}` : `${online} device${online === 1 ? "" : "s"} online.`;
  const tail = open.length === 0 ? "Everything looks right." : open.length === 1 ? "One thing to fix." : `${open.length} things to fix.`;

  return (
    <div className="scroll-thin h-full overflow-auto">
      <div className="mx-auto flex max-w-[1320px] flex-col gap-5 p-4 sm:p-7">
        <WelcomeBack />
        <section className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <p className="label">{now.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}, {now.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}</p>
            <h1 className="mt-1 text-[26px] font-semibold tracking-tight sm:text-[30px]">
              {headline} <span className={open.length ? "" : "text-muted"}>{tail}</span>
            </h1>
          </div>
          <CaptureButton />
        </section>

        <Attention findings={open} info={info} onDismiss={dismiss} onFixed={markFixed} />

        <section className="grid grid-cols-2 gap-3 lg:grid-cols-4 lg:gap-4" aria-label="Summary">
          <Kpi label="Devices" value={String(online)} unit={`of ${devices.length} online`} note={offline ? `${offline} not responding` : "All responding"} noteTone={offline ? "warn" : undefined} />
          <Kpi label="Bus load" value={(metrics?.load_pct ?? 0).toFixed(1)} unit="%" note={`Peak ${peak.toFixed(1)}% in this range`} extra={<Sparkline values={last60} width={96} height={30} label="Bus load, last minute" />} />
          <Kpi label="Errors" value={String(metrics?.errors_per_sec ?? 0)} unit="/s" note={(metrics?.total_errors ?? 0) === 0 ? "No bus errors since start" : `${metrics?.total_errors} since start`} noteTone={(metrics?.total_errors ?? 0) === 0 ? "ok" : "bad"} />
          <Kpi
            label="Declared load (LEN)"
            value={(len * 0.05).toFixed(len * 0.05 < 10 ? 2 : 1)}
            unit="A max"
            note={noLen ? `${len} LEN · ${noLen} device${noLen === 1 ? "" : "s"} didn't report LEN` : `${len} LEN · devices' stated maximum, not measured`}
            noteTone={noLen ? "warn" : undefined}
          />
        </section>

        <section className="grid gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
          <div className="card p-4 sm:p-5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h2 className="text-[15px] font-semibold">Bus load</h2>
              <div className="seg" role="group" aria-label="Time range">
                {RANGES.map(([r, l]) => (
                  <button key={r} aria-pressed={range === r} onClick={() => setRange(r)}>
                    {l}
                  </button>
                ))}
              </div>
            </div>
            <div className="mt-3">
              <TimeArea points={pts} min={0} maxHint={10} unit="%" label={`Bus load over the last ${RANGES.find((x) => x[0] === range)?.[1]}`} extra={(i) => (hist[i]?.load != null ? `${hist[i].frames} frames/s` : null)} />
            </div>
            <p className="mt-2 text-[13px] text-muted">Healthy networks stay well under 30%. Grey bands are times with no gateway connected.</p>
          </div>
          <div className="card p-4 sm:p-5">
            <h2 className="mb-4 text-[15px] font-semibold">Busiest devices</h2>
            {busiest.length ? (
              <BarList
                unit="%"
                items={busiest.map(([a, , pct]) => {
                  const d = byAddr.get(a);
                  return {
                    key: a,
                    label: deviceLabel(d),
                    sub: d?.device_function_text ?? `address ${a}`,
                    value: pct,
                    onClick: d
                      ? () => {
                          useApp.getState().selectDevice(d.key);
                          useApp.getState().setPage("devices");
                        }
                      : undefined,
                  };
                })}
              />
            ) : (
              <p className="text-sm text-muted">No traffic yet.</p>
            )}
          </div>
        </section>

        <OnTheBus live={live} />

        <section className="grid gap-4 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
          <div className="card overflow-hidden">
            <h2 className="border-b hairline px-5 py-3 text-[15px] font-semibold">Recent events</h2>
            <ul className="max-h-72 divide-y divide-rule/60 overflow-auto">
              {alerts.length === 0 && <li className="px-5 py-4 text-sm text-muted">Nothing yet.</li>}
              {[...alerts]
                .reverse()
                .slice(0, 40)
                .map((a) => (
                  <li key={a.seq} className="flex items-start gap-3 px-5 py-2.5 text-sm">
                    <SevIcon s={a.severity} />
                    <span className="min-w-0 flex-1">{a.message}</span>
                    <span className="shrink-0 font-mono text-xs text-muted">{ago(a.ts_us)}</span>
                  </li>
                ))}
            </ul>
          </div>
          <div className="card p-5">
            <h2 className="mb-3 text-[15px] font-semibold">Bus details</h2>
            <dl className="grid grid-cols-[1fr_auto] gap-x-4 gap-y-1.5 text-sm">
              <dt className="text-muted">Frames per second</dt>
              <dd className="text-right font-mono">{(metrics?.frames_per_sec ?? 0).toLocaleString()}</dd>
              <dt className="text-muted">Messages per second</dt>
              <dd className="text-right font-mono">{(metrics?.messages_per_sec ?? 0).toLocaleString()}</dd>
              <dt className="text-muted">Incomplete fast packets</dt>
              <dd className="text-right font-mono">{(metrics?.fp_timeouts ?? 0) + (metrics?.fp_orphans ?? 0)}</dd>
              <dt className="text-muted">Aborted ISO transfers</dt>
              <dd className="text-right font-mono">{metrics?.tp_aborts ?? 0}</dd>
              <dt className="text-muted">Total frames</dt>
              <dd className="text-right font-mono">{(metrics?.total_frames ?? 0).toLocaleString()}</dd>
              <dt className="text-muted">This analyzer</dt>
              <dd className="text-right font-mono">{status?.own_address != null ? `address ${status.own_address}` : status?.tx_mode === "passive" ? "listen only" : "—"}</dd>
            </dl>
          </div>
        </section>
      </div>
    </div>
  );
}

function ago(tsUs: number): string {
  // Alerts use the engine clock, as do packet timestamps.
  const pk = useApp.getState().packets;
  const nowUs = (pk[pk.length - 1]?.ts ?? 0) || tsUs;
  const s = Math.max(0, (nowUs - tsUs) / 1e6);
  if (s < 60) return `${Math.round(s)} s`;
  if (s < 3600) return `${Math.round(s / 60)} min`;
  return `${(s / 3600).toFixed(1)} h`;
}

function Kpi({ label, value, unit, note, noteTone, extra }: { label: string; value: string; unit?: string; note?: string; noteTone?: "ok" | "warn" | "bad"; extra?: React.ReactNode }) {
  return (
    <div className="card px-4 py-3.5 sm:px-5 sm:py-4">
      <p className="label">{label}</p>
      <div className="flex items-end justify-between gap-2">
        <p className="mt-1.5 text-[26px] font-semibold leading-none tracking-tight sm:text-[28px]">
          <span className="tnum">{value}</span>
          {unit && <span className="ml-1 text-[14px] font-medium text-muted">{unit}</span>}
        </p>
        {extra}
      </div>
      {note && <p className={`mt-1.5 text-[13px] ${noteTone === "ok" ? "text-stbd" : noteTone === "bad" ? "text-port" : noteTone === "warn" ? "text-caution" : "text-muted"}`}>{note}</p>}
    </div>
  );
}

function SevIcon({ s }: { s: string }) {
  if (s === "error") return <CircleAlert size={16} className="mt-0.5 shrink-0 text-port" aria-label="Error" />;
  if (s === "warning") return <AlertTriangle size={16} className="mt-0.5 shrink-0 text-caution" aria-label="Warning" />;
  return <Info size={16} className="mt-0.5 shrink-0 text-muted" aria-label="Info" />;
}

function Attention({ findings, info, onDismiss, onFixed }: { findings: Finding[]; info: Finding[]; onDismiss: (f: Finding) => void; onFixed: (f: Finding) => void }) {
  const { toast, setHighlight, setPage, selectDevice, status } = useApp();
  const [busy, setBusy] = useState<string | null>(null);
  if (findings.length === 0 && info.length === 0)
    return (
      <section className="card flex items-center gap-3 px-5 py-4">
        <CheckCircle2 size={20} className="text-stbd" />
        <p>
          <span className="font-semibold">Nothing needs attention.</span> <span className="text-muted">No instance conflicts, duplicate devices or unidentified senders.</span>
        </p>
      </section>
    );
  const rows = [...findings, ...info];
  return (
    <section className="card overflow-hidden" aria-labelledby="att">
      <h2 id="att" className="border-b hairline px-5 py-3 text-[15px] font-semibold">
        Needs attention
      </h2>
      <ul className="divide-y divide-rule">
        {rows.map((f, i) => {
          const fix = fixFor(f);
          const id = `${f.kind}${i}`;
          const tone = f.severity === "error" ? "bg-port/10 text-port" : f.severity === "warning" ? "bg-caution/10 text-caution" : "bg-shoal text-muted";
          return (
            <li key={id} className="flex flex-wrap items-center gap-x-4 gap-y-3 px-5 py-4">
              <span className={`flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ${tone}`}>
                {f.severity === "error" ? <AlertTriangle size={18} /> : f.severity === "warning" ? <CircleAlert size={18} /> : <Info size={18} />}
              </span>
              <div className="min-w-0 flex-[1_1_360px]">
                <p className="font-semibold">{f.message}</p>
                {f.suggestion && <p className="mt-0.5 text-sm text-muted">{f.suggestion}</p>}
              </div>
              <div className="flex flex-wrap gap-2">
                {f.devices.length > 1 && (
                  <button
                    className="btn"
                    onClick={() => {
                      setHighlight(f.devices);
                      selectDevice(null);
                      setPage("devices");
                    }}
                  >
                    Show devices <ArrowRight size={14} />
                  </button>
                )}
                {f.kind === "instance_conflict" && (
                  <button className="btn" onClick={() => onDismiss(f)} title="Hide this; for deliberate backups such as two GPS antennas">
                    It's intentional
                  </button>
                )}
                {fix && (
                  <button
                    className="btn btn-primary"
                    disabled={busy === id}
                    onClick={async () => {
                      if (fix.writes && !status?.armed) return toast("info", "Unlock changes first (top right), then fix.");
                      setBusy(id);
                      try {
                        toast("ok", await fix.run());
                        if (fix.writes) onFixed(f);
                      } catch (e) {
                        toast("error", (e as Error).message);
                      } finally {
                        setBusy(null);
                      }
                    }}
                  >
                    {busy === id ? "Working…" : fix.label}
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function CaptureButton() {
  const { status, toast } = useApp();
  const cap = status?.capture;
  return cap ? (
    <button className="btn !border-port/50 !text-port" onClick={() => post("/capture/stop").then(() => toast("ok", `Saved ${cap.file}. Find it under Settings → Captures.`)).catch((e) => toast("error", e.message))}>
      <Square size={14} fill="currentColor" /> Stop recording
    </button>
  ) : (
    <button className="btn" onClick={() => post("/capture/start", { format: "pcapng" }).then(() => toast("info", "Recording everything on the bus to a Wireshark file")).catch((e) => toast("error", e.message))}>
      <Circle size={14} className="text-port" fill="currentColor" /> Record a capture
    </button>
  );
}

function OnTheBus({ live }: { live: ReturnType<typeof useLive> }) {
  const { devices, selectDevice, setPage } = useApp();
  const groups = useMemo(() => {
    const g = new Map<System, { models: string[]; items: { label: string; value: string; key: string; bar?: number; bad?: boolean }[] }>();
    for (const d of devices.filter((x) => x.online)) {
      const sys = systemOf(d);
      if (sys === "Other" || sys === "Displays and gateways") continue;
      const rs = readings(live[d.key]);
      if (!rs.length) continue;
      const e = g.get(sys) ?? { models: [], items: [] };
      e.models.push(deviceLabel(d));
      for (const r of rs) {
        if (e.items.find((x) => x.label === r.label && sys !== "Tanks")) continue;
        e.items.push({ label: sys === "Tanks" ? `${r.label} · address ${d.address}` : r.label, value: r.value, key: d.key, bar: r.pgn === 127505 && r.raw != null ? r.raw : undefined });
      }
      g.set(sys, e);
    }
    return SYSTEMS.filter((s) => g.has(s)).map((s) => ({ sys: s, ...g.get(s)! }));
  }, [devices, live]);
  if (!groups.length) return null;
  const go = (k: string) => {
    selectDevice(k);
    setPage("devices");
  };
  return (
    <section aria-labelledby="live">
      <div className="mb-2.5 flex items-center justify-between">
        <h2 id="live" className="text-[15px] font-semibold">
          On the bus now
        </h2>
        <button className="text-[13px] text-sea hover:underline" onClick={() => setPage("devices")}>
          All devices
        </button>
      </div>
      <div className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        {groups.map((g) => (
          <div key={g.sys} className="card px-5 py-4">
            <div className="flex items-baseline justify-between gap-2">
              <p className="label">{g.sys}</p>
              <span className="truncate text-xs text-muted">{[...new Set(g.models)].join(" · ")}</span>
            </div>
            <ul className="mt-2.5 space-y-1.5 text-[13px]">
              {g.items.slice(0, 6).map((it, i) => (
                <li key={i}>
                  <button className="flex w-full items-baseline justify-between gap-3 text-left hover:text-sea" onClick={() => go(it.key)}>
                    <span className="truncate text-muted">{it.label}</span>
                    <span className="shrink-0 font-mono text-ink">{it.value}</span>
                  </button>
                  {it.bar != null && (
                    <div className="mt-1 h-2 overflow-hidden rounded-full bg-shoal">
                      <div className="h-full rounded-full bg-c1" style={{ width: `${Math.min(100, it.bar)}%` }} />
                    </div>
                  )}
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
    </section>
  );
}
