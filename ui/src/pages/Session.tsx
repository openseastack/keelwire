// Session page: boat and visit details, controls, timeline and notes.
import { useEffect, useState } from "react";
import { Pause, Play, Trash2, CircleCheck, CircleAlert, Radio } from "lucide-react";
import { useApp } from "../lib/store";
import { fmtBytes, fmtClock, fmtDate, fmtLatLon, fmtTime, liveActive, sessionApi, timeline, useSession, type TimelineItem } from "../lib/session";

function useNow(ms = 1000) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

/** Text field that saves on blur or Enter. */
function Field({ label, value, onSave, mono, placeholder, readOnly, upper }: { label: string; value: string; onSave?: (v: string) => void; mono?: boolean; placeholder?: string; readOnly?: boolean; upper?: boolean }) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  const commit = () => v !== value && onSave?.(v);
  return (
    <label className="flex flex-col gap-1 text-[12.5px] text-muted">
      {label}
      <input
        className={`input h-10 text-[14px] ${mono ? "font-mono" : ""} ${readOnly ? "!bg-shoal text-muted" : ""}`}
        value={v}
        readOnly={readOnly}
        placeholder={placeholder}
        onChange={(e) => setV(upper ? e.target.value.toUpperCase() : e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
      />
    </label>
  );
}

export function SessionPage() {
  const { status: s, at, setEndOpen, lastEnd } = useSession();
  const setPage = useApp((x) => x.setPage);
  const iface = useApp((x) => x.status?.interface);
  const now = useNow();
  const [note, setNote] = useState("");
  const [show, setShow] = useState<"all" | "changes" | "notes">("all");

  if (!s) return <div className="p-8 text-muted">Loading…</div>;
  if (s.state === "none") {
    return (
      <div className="h-full overflow-auto">
        <div className="mx-auto flex max-w-xl flex-col items-start gap-4 px-6 py-16">
          <span className="flex h-12 w-12 items-center justify-center rounded-full border-2 border-port">
            <span className="h-5 w-5 rounded-full bg-port" />
          </span>
          <h1 className="text-[26px] font-semibold">No session open</h1>
          <p className="text-muted">
            A service session records everything on the bus and every change you make, then saves reports for the owner and for you. One starts by itself when a gateway connects{iface ? "" : "; connect one to begin"}.
          </p>
          {lastEnd && (
            <p className="rounded-xl bg-stbd/10 px-3.5 py-2.5 text-sm text-stbd">
              Last session saved: <strong>{lastEnd.rel}</strong>
            </p>
          )}
          <div className="flex gap-2.5">
            <button className="btn btn-primary h-10" onClick={sessionApi.start}>
              Start session
            </button>
            <button className="btn h-10" onClick={() => setPage("boats")}>
              Boats
            </button>
          </div>
          <div className="w-full max-w-xs">
            <Field label="Technician" value={s.technician ?? ""} placeholder="Your name, on every report" onSave={(t) => sessionApi.update({ technician: t })} />
          </div>
        </div>
      </div>
    );
  }
  const sess = s.session!;
  const v = sess.vessel;
  const rec = s.state === "recording";
  const items = timeline(sess).filter((i) => show === "all" || (show === "changes" && i.kind === "change") || (show === "notes" && i.kind === "note"));
  const pauses = [...sess.pauses, ...(sess.paused ? [sess.paused] : [])];
  const addNote = () => {
    if (!note.trim()) return;
    sessionApi.note(note);
    setNote("");
  };
  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto grid max-w-[1280px] gap-5 px-4 py-5 sm:px-7 lg:grid-cols-[380px_1fr]">
        <div className="flex flex-col gap-4">
          <section className="card flex flex-col gap-3 p-4">
            <div className="flex items-baseline justify-between">
              <h2 className="text-base font-semibold">Boat</h2>
              {v.source === "ais" ? (
                <span className="flex items-center gap-1 text-xs font-semibold text-stbd">
                  <Radio size={13} /> From its AIS transponder
                </span>
              ) : v.source === "manual" ? (
                <span className="text-xs text-muted">Typed by you</span>
              ) : (
                <span className="text-xs text-caution">Not identified yet</span>
              )}
            </div>
            <Field label="Boat name" value={v.name ?? ""} placeholder="e.g. Second Wind" onSave={(name) => sessionApi.update({ name })} />
            <div className="grid grid-cols-2 gap-2.5">
              <Field label="MMSI" value={v.mmsi ? String(v.mmsi) : "—"} mono readOnly />
              <Field label="Call sign" value={v.callsign ?? "—"} mono readOnly />
            </div>
            <Field label="Hull ID (HIN) · optional" value={v.hin ?? ""} mono upper placeholder="e.g. SER12345F526" onSave={(hin) => sessionApi.update({ hin })} />
            <p className="text-xs text-muted">
              {v.source === "ais" ? "Name, MMSI and call sign come from AIS. Edit the name if AIS has it wrong; history stays filed by MMSI." : "Without AIS, the session is filed by hull ID, then by name, else under the date."}
            </p>
            {s.ais_own && v.source !== "ais" && (
              <button className="btn btn-sm self-start" onClick={() => sessionApi.update({ confirm: true })}>
                Use AIS: {s.ais_own.name ?? s.ais_own.mmsi}
              </button>
            )}
          </section>

          <section className="card flex flex-col gap-3 p-4">
            <h2 className="text-base font-semibold">Visit</h2>
            <Field label="Technician" value={sess.technician ?? s.technician ?? ""} placeholder="Your name" onSave={(t) => sessionApi.update({ technician: t })} />
            <p className="-mt-1.5 text-xs text-muted">Remembered on this computer for the next session.</p>
            <dl className="grid grid-cols-[auto_1fr] gap-x-3.5 gap-y-1.5 text-[13px]">
              <dt className="text-muted">Started</dt>
              <dd>
                {fmtDate(sess.started_ms, true)}, {fmtTime(sess.started_ms, true)}
              </dd>
              <dt className="text-muted">Position</dt>
              <dd>{sess.position ? <span className="font-mono text-[12.5px]">{fmtLatLon(sess.position.lat, sess.position.lon)}</span> : <span className="text-muted">No GPS fix yet</span>}</dd>
              <dt className="text-muted">Gateway</dt>
              <dd className="break-words">{sess.gateway ?? "—"}</dd>
              <dt className="text-muted">Recording</dt>
              <dd>
                {fmtBytes(s.bytes ?? 0)} · {(s.frames ?? 0).toLocaleString()} frames
              </dd>
              {pauses.length > 0 && (
                <>
                  <dt className="text-muted">Paused</dt>
                  <dd>
                    {pauses.map((p, i) => (
                      <span key={i} className="block">
                        {fmtTime(p.from_ms)}–{p.to_ms ? fmtTime(p.to_ms) : "now"} ({p.reason === "disconnected" ? "gateway disconnected" : p.reason === "closed" ? "Keelwire closed" : "by you"})
                      </span>
                    ))}
                  </dd>
                </>
              )}
              {sess.previous && (
                <>
                  <dt className="text-muted">Last visit</dt>
                  <dd>
                    {fmtDate(sess.previous.started_ms)} · visit {sess.visit_number}
                  </dd>
                </>
              )}
            </dl>
          </section>

          <div className="flex gap-2.5">
            {rec ? (
              <button className="btn h-11 flex-1" onClick={sessionApi.pause}>
                <Pause size={15} /> Pause
              </button>
            ) : (
              <button className="btn h-11 flex-1" onClick={sessionApi.resume} disabled={s.reason === "disconnected"} title={s.reason === "disconnected" ? "Resumes when the gateway is back" : undefined}>
                <Play size={15} /> {s.reason === "disconnected" ? "Waiting for gateway" : "Resume"}
              </button>
            )}
            <button className="btn h-11 flex-[1.4] border-ink bg-ink font-semibold text-paper hover:!bg-ink/90" onClick={() => setEndOpen(true)}>
              End session…
            </button>
          </div>
        </div>

        <div className="flex min-w-0 flex-col gap-4">
          <section className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Tile label="Recording" value={fmtClock(liveActive(s, at, now))} mono tone={rec ? "port" : undefined} />
            <Tile label="Changes" value={String(s.changes_ok ?? 0)} note={(s.changes_ok ?? 0) > 0 && !sess.changes.some((c) => c.kind === "setting" && c.result === "sent") ? "all confirmed" : undefined} />
            <Tile label="Fixed" value={String(s.fixed ?? 0)} />
            <Tile label="Still open" value={String(s.open ?? 0)} tone={(s.open ?? 0) > 0 ? "caution" : undefined} />
          </section>

          <section className="card overflow-hidden">
            <div className="flex flex-wrap items-center justify-between gap-2 border-b hairline px-4 py-3">
              <h2 className="text-base font-semibold">Timeline</h2>
              <div className="flex gap-1.5" role="group" aria-label="Show">
                {(["all", "changes", "notes"] as const).map((k) => (
                  <button key={k} className={`chip-btn h-7 ${show === k ? "chip-btn-on" : ""}`} aria-pressed={show === k} onClick={() => setShow(k)}>
                    {k === "all" ? "All" : k === "changes" ? "Changes" : "Notes"}
                  </button>
                ))}
              </div>
            </div>
            <ol className="flex flex-col px-4 py-1.5">
              {items.length === 0 && <li className="py-6 text-center text-sm text-muted">Nothing yet.</li>}
              {items.map((i, k) => (
                <TimelineRow key={`${i.ts}-${k}`} item={i} last={k === items.length - 1} />
              ))}
            </ol>
            <div className="flex items-end gap-2.5 border-t hairline px-4 py-3">
              <label className="flex flex-1 flex-col gap-1 text-[12.5px] text-muted">
                Add a note
                <textarea
                  id="session-note"
                  rows={2}
                  className="input h-auto resize-y py-2 text-[14px]"
                  placeholder="What you saw, what you did, what the owner said…"
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  onKeyDown={(e) => (e.metaKey || e.ctrlKey) && e.key === "Enter" && addNote()}
                />
              </label>
              <button className="btn h-10" onClick={addNote} disabled={!note.trim()}>
                Add note
              </button>
            </div>
          </section>
        </div>
      </div>
    </div>
  );
}

function Tile({ label, value, note, mono, tone }: { label: string; value: string; note?: string; mono?: boolean; tone?: "port" | "caution" }) {
  return (
    <div className="card px-3.5 py-3">
      <div className="text-xs text-muted">{label}</div>
      <div className={`text-[22px] font-semibold ${mono ? "font-mono" : ""} ${tone === "port" ? "text-port" : tone === "caution" ? "text-caution" : ""}`}>
        {value}
        {note && <span className="ml-1.5 text-[13px] font-medium text-stbd">{note}</span>}
      </div>
    </div>
  );
}

function TimelineRow({ item: i, last }: { item: TimelineItem; last: boolean }) {
  const dot =
    i.kind === "change"
      ? i.change!.result === "failed"
        ? "bg-port"
        : "bg-sea"
      : i.kind === "note"
        ? "bg-ink"
        : i.sub === "start" || i.sub === "end"
          ? "bg-port"
          : i.sub === "offline" || i.sub === "finding"
            ? "bg-caution"
            : "bg-muted";
  return (
    <li className={`grid grid-cols-[64px_14px_1fr] gap-2 py-2.5 ${last ? "" : "border-b hairline"}`}>
      <span className="font-mono text-[12px] text-muted">{fmtTime(i.ts, true)}</span>
      <span className={`mt-1.5 h-2 w-2 ${i.sub === "pause" || i.sub === "resume" ? "rounded-sm" : "rounded-full"} ${dot}`} aria-hidden />
      {i.kind === "change" ? (
        <span className="text-[13.5px]">
          <strong className="font-semibold">{i.change!.device}</strong> · {i.change!.setting}{" "}
          {(i.change!.before != null || i.change!.after != null) && (
            <span className="font-mono">
              {i.change!.before ?? "—"} → {i.change!.after ?? "—"}
            </span>
          )}
          <br />
          {i.change!.result === "confirmed" ? (
            <span className="inline-flex items-center gap-1 text-[12.5px] text-stbd">
              <CircleCheck size={13} /> Confirmed by the device
            </span>
          ) : i.change!.result === "failed" ? (
            <span className="inline-flex items-center gap-1 text-[12.5px] text-port">
              <CircleAlert size={13} /> Failed: {i.change!.error}
            </span>
          ) : (
            <span className="text-[12.5px] text-caution">{i.change!.kind === "setting" ? "Accepted; not read back from the device" : "Sent"}</span>
          )}
        </span>
      ) : i.kind === "note" ? (
        <span className="group flex items-start gap-2 rounded-lg bg-shoal px-2.5 py-1.5 text-[13.5px]">
          <span className="flex-1 whitespace-pre-wrap">{i.text}</span>
          <button className="text-muted opacity-0 hover:text-port focus:opacity-100 group-hover:opacity-100" aria-label="Delete note" onClick={() => sessionApi.deleteNote(i.note!.ts_ms)}>
            <Trash2 size={14} />
          </button>
        </span>
      ) : (
        <span className={`text-[13.5px] ${i.sub === "pause" || i.sub === "resume" ? "text-muted" : ""}`}>{i.text}</span>
      )}
    </li>
  );
}

