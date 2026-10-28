import { LockOpen, Lock } from "lucide-react";
import { useApp, deviceLabel } from "../lib/store";
import { post } from "../lib/api";
import { ShareBar } from "../lib/charts";
import { useUnlock } from "../components/Lock";
import type { Bank } from "../lib/types";

export function Switching() {
  const { banks, devices, status } = useApp();
  const armed = !!status?.armed;
  const unlock = useUnlock();
  const sorted = [...banks].sort((a, b) => a.instance - b.instance);
  // Keypads report key presses in the same message but can't be switched.
  const isKeypad = (b: Bank) => !b.currents.some((c) => c != null) && /button|keypad|switch interface/i.test(devices.find((d) => d.key === b.device_key)?.device_function_text ?? "");
  const loads = sorted.filter((b) => !isKeypad(b));
  const total = loads.reduce((a, b) => a + b.currents.reduce<number>((x, c) => x + (c ?? 0), 0), 0);
  const on = loads.reduce((a, b) => a + b.states.filter((s) => s === 1).length, 0);
  const tripped = loads.reduce((a, b) => a + b.states.filter((s) => s === 2).length, 0);
  const circuits = loads.reduce((a, b) => a + b.states.filter((s, i) => s !== 3 || b.currents[i] != null).length, 0);
  const metered = loads.filter((b) => b.currents.some((c) => c != null));
  const parts = metered.flatMap((b) => b.currents.map((c, i) => ({ label: b.labels[i] ?? (metered.length > 1 ? `Bank ${b.instance} ch ${i + 1}` : `Ch ${i + 1}`), value: c ?? 0 })));

  return (
    <div className="scroll-thin h-full overflow-auto">
      <div className="mx-auto flex max-w-[1352px] flex-col gap-5 p-4 sm:p-7">
        <div className="flex flex-wrap items-center gap-3">
          <h1 className="text-2xl font-semibold tracking-tight">Switching</h1>
          <span className="flex-1" />
          {armed ? (
            <button className="btn !border-caution !text-caution" onClick={() => unlock(false)}>
              <LockOpen size={16} /> Switching unlocked
            </button>
          ) : (
            <>
              <span className="hidden text-muted md:inline">Switches are locked so nothing turns on by accident.</span>
              <button className="btn" onClick={() => unlock(true)}>
                <Lock size={16} /> Unlock switching
              </button>
            </>
          )}
        </div>

        {sorted.length === 0 ? (
          <div className="card mx-auto max-w-lg p-10 text-center">
            <p className="text-lg font-semibold">No switch banks heard yet</p>
            <p className="mt-1 text-muted">Breaker modules (CLMD12/16), keypads (CKM12, VMM6) and other switching devices appear here when they send their bank status (PGN 127501).</p>
          </div>
        ) : (
          <>
            <section className="card grid gap-5 p-5 sm:grid-cols-[repeat(3,minmax(0,1fr))_minmax(0,3fr)] sm:items-center">
              <Stat label="Total load" value={total.toFixed(1)} unit="A" />
              <Stat label="Circuits on" value={String(on)} unit={`of ${circuits}`} />
              <Stat label="Tripped" value={String(tripped)} tone={tripped ? "bad" : undefined} />
              <div>
                <p className="label mb-1.5">Share of total load</p>
                <ShareBar parts={parts} unit=" A" label="Current by circuit" />
              </div>
            </section>
            {sorted.map((b) =>
              isKeypad(b) ? (
                <KeypadPanel key={b.instance} b={b} name={deviceLabel(devices.find((d) => d.key === b.device_key))} />
              ) : (
                <BankPanel key={b.instance} b={b} name={deviceLabel(devices.find((d) => d.key === b.device_key))} armed={armed} />
              ),
            )}
          </>
        )}
      </div>
    </div>
  );
}

function KeypadPanel({ b, name }: { b: Bank; name: string }) {
  const keys = b.states.map((s, i) => ({ s, i })).filter(({ s }) => s !== 3);
  return (
    <section aria-label={`${name} bank ${b.instance}`} className="card px-5 py-4">
      <header className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <h2 className="text-[15px] font-semibold">
          {name} <span className="label">bank {b.instance} · address {b.source}</span>
        </h2>
        <span className="label">Shows which keys are pressed. Keys switch circuits through their breaker module.</span>
      </header>
      <div className="mt-3 grid grid-cols-[repeat(auto-fill,minmax(64px,1fr))] gap-2">
        {keys.map(({ s, i }) => (
          <div key={i} className={`flex h-11 items-center justify-center rounded-lg border font-mono text-[13px] ${s === 1 ? "border-sea bg-sea/10 text-sea" : "hairline text-muted"}`} aria-label={`Key ${i + 1} ${s === 1 ? "pressed" : "not pressed"}`} role="img">
            {b.labels[i] ?? i + 1}
          </div>
        ))}
      </div>
    </section>
  );
}

function Stat({ label, value, unit, tone }: { label: string; value: string; unit?: string; tone?: "bad" }) {
  return (
    <div>
      <p className="label">{label}</p>
      <p className={`tnum mt-1 text-[28px] font-semibold leading-none tracking-tight ${tone === "bad" ? "text-port" : ""}`}>
        {value}
        {unit && <span className="ml-1 text-sm font-medium text-muted">{unit}</span>}
      </p>
    </div>
  );
}

function BankPanel({ b, name, armed }: { b: Bank; name: string; armed: boolean }) {
  const toast = useApp((s) => s.toast);
  const used = b.states.map((s, i) => ({ s, i })).filter(({ s, i }) => s !== 3 || b.currents[i] != null);
  const hasCurrent = b.currents.some((c) => c != null);
  const maxA = Math.max(10, ...b.currents.map((c) => c ?? 0));
  const set = (ind: number, on: boolean) => post("/switch", { instance: b.instance, indicator: ind, on }).catch((e) => toast("error", e.message));
  return (
    <section aria-label={`${name} bank ${b.instance}`}>
      <header className="mb-2.5 flex flex-wrap items-baseline gap-x-3">
        <h2 className="text-[15px] font-semibold">{name}</h2>
        <span className="label">
          bank {b.instance} · address {b.source}
        </span>
      </header>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-3">
        {used.map(({ s, i }) => {
          const a = b.currents[i];
          const onNow = s === 1;
          const trip = s === 2;
          return (
            <div key={i} className={`card flex flex-col gap-2.5 px-4 py-3.5 ${trip ? "!border-port" : ""}`}>
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className="truncate font-semibold" title={b.labels[i] ?? undefined}>
                    {b.labels[i] ?? `Channel ${i + 1}`}
                  </p>
                  {b.labels[i] && <p className="label">Channel {i + 1}</p>}
                </div>
                <button
                  role="switch"
                  aria-checked={onNow}
                  aria-label={`${b.labels[i] ?? `Channel ${i + 1}`} ${onNow ? "on" : "off"}`}
                  disabled={!armed || trip}
                  onClick={() => set(i + 1, !onNow)}
                  className={`relative h-[26px] w-11 shrink-0 rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-55 ${onNow ? "bg-stbd" : "bg-muted/40"}`}
                >
                  <span className={`absolute top-[3px] h-5 w-5 rounded-full bg-white shadow transition-[left] ${onNow ? "left-[21px]" : "left-[3px]"}`} />
                </button>
              </div>
              <div>
                <div className="flex items-center justify-between text-[13px]">
                  <span className={`font-mono ${a ? "" : "text-muted"}`}>{a != null ? `${a.toFixed(1)} A` : hasCurrent ? "—" : ""}</span>
                  <span className={`font-medium ${trip ? "text-port" : onNow ? "text-stbd" : "text-muted"}`}>{trip ? "Tripped" : onNow ? "On" : s === 0 ? "Off" : "Unknown"}</span>
                </div>
                {hasCurrent && (
                  <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-shoal" role="img" aria-label={a != null ? `${a.toFixed(1)} amps` : "current unknown"}>
                    <div className="h-full rounded-full bg-c1 transition-[width]" style={{ width: `${Math.min(100, ((a ?? 0) / maxA) * 100)}%` }} />
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
