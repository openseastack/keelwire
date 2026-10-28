// Session controls shown on every screen: the vessel badge and REC button in
// the top bar, and the dialogs (confirm the boat, continue or start new,
// end the session).
import { useEffect, useRef, useState } from "react";
import { Pause, Play, PenLine, ArrowRight, Square, Radio, Check, Download, FolderOpen, X, Share } from "lucide-react";
import { useApp } from "../lib/store";
import { fmtClock, fmtDate, fmtTime, fmtBytes, liveActive, sessionApi, useSession, vesselName, type EndOptions } from "../lib/session";
import { canShare, isMobileApp, shareHistory } from "../lib/files";
import { openReport } from "./ReportViewer";
import { load, save } from "../lib/store";

function useNow(ms = 1000) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(t);
  }, [ms]);
  return now;
}

/** Boat name, MMSI and call sign at the top of every screen. */
export function VesselBadge() {
  const s = useSession((x) => x.status);
  const setPage = useApp((x) => x.setPage);
  const v = s?.session?.vessel;
  if (!s || s.state === "none" || !v) return null;
  const ids = [v.mmsi ? `MMSI ${v.mmsi}` : null, v.callsign, v.hin ? `HIN ${v.hin}` : null].filter(Boolean).join(" · ");
  return (
    <button onClick={() => setPage("session")} className="flex min-w-0 items-center gap-2.5 rounded-xl border hairline py-1 pl-1 pr-3 text-left hover:bg-shoal" title="Boat and session details">
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-sea/10 text-sea">
        <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
          <path d="M3 17h18l-2 4H5z" />
          <path d="M12 3v14" />
          <path d="M12 4l7 10h-7" />
        </svg>
      </span>
      <span className="min-w-0">
        <span className="block truncate text-[14px] font-bold tracking-wide">{v.source === "none" ? "Which boat?" : vesselName(v)}</span>
        <span className="block truncate text-[11.5px] text-muted">
          {ids ? <span className="font-mono">{ids}</span> : v.source === "none" ? "Add a name or hull ID" : null}
          {v.source === "ais" && <> · from AIS</>}
        </span>
      </span>
    </button>
  );
}

/** The red recording button and its menu. */
export function RecButton() {
  const { status: s, at, setEndOpen } = useSession();
  const setPage = useApp((x) => x.setPage);
  const now = useNow();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);
  if (!s) return null;
  if (s.state === "none") {
    return (
      <button className="btn h-9 rounded-full pl-1.5 pr-3.5" onClick={sessionApi.start} title="Start recording a service session">
        <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-port">
          <span className="h-2.5 w-2.5 rounded-full bg-port" />
        </span>
        Start session
      </button>
    );
  }
  const t = fmtClock(liveActive(s, at, now));
  const rec = s.state === "recording";
  const disc = !rec && s.reason === "disconnected";
  const cls = rec ? "border-port/30 bg-port/10 text-port" : disc ? "border-caution/30 bg-caution/10 text-caution" : "border-[rgb(var(--rule))] bg-shoal text-muted";
  const sess = s.session!;
  return (
    <div className="relative" ref={ref}>
      <button
        className={`inline-flex h-9 items-center gap-2 rounded-full border pl-1.5 pr-3.5 text-[13px] font-semibold ${cls}`}
        onClick={() => setOpen((o) => !o)}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`${rec ? "Recording" : "Paused"}, ${t}. Session menu`}
      >
        {rec ? (
          <span className="rec-dot h-6 w-6 rounded-full border-[3px] border-panel bg-port" aria-hidden />
        ) : (
          <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-current" aria-hidden>
            <Pause size={11} fill="currentColor" strokeWidth={0} />
          </span>
        )}
        {rec ? "REC" : disc ? "Paused · reconnecting" : "Paused"}
        <span className="font-mono font-medium">{t}</span>
      </button>
      {open && (
        <div role="menu" className="absolute right-0 top-11 z-40 w-[300px] overflow-hidden rounded-xl border hairline bg-panel shadow-xl">
          <div className="border-b hairline px-3.5 py-3">
            <div className="truncate font-bold tracking-wide">{vesselName(sess.vessel)}</div>
            <div className="text-[12.5px] text-muted">
              Since {fmtTime(sess.started_ms)} · {s.changes_ok ?? 0} {s.changes_ok === 1 ? "change" : "changes"} · {fmtBytes(s.bytes ?? 0)}
              {sess.technician ? ` · ${sess.technician}` : ""}
            </div>
          </div>
          {rec ? (
            <MenuItem icon={<Pause size={15} />} onClick={() => (setOpen(false), sessionApi.pause())}>
              Pause recording
            </MenuItem>
          ) : (
            <MenuItem icon={<Play size={15} />} disabled={disc} onClick={() => (setOpen(false), sessionApi.resume())}>
              {disc ? "Resumes when the gateway is back" : "Resume recording"}
            </MenuItem>
          )}
          <MenuItem icon={<PenLine size={15} />} onClick={() => (setOpen(false), setPage("session"), setTimeout(() => document.getElementById("session-note")?.focus(), 50))}>
            Add a note…
          </MenuItem>
          <MenuItem icon={<ArrowRight size={15} />} onClick={() => (setOpen(false), setPage("session"))}>
            Open session page
          </MenuItem>
          <div className="border-t hairline">
            <MenuItem icon={<Square size={13} fill="currentColor" />} strong onClick={() => (setOpen(false), setEndOpen(true))}>
              End session and save…
            </MenuItem>
          </div>
        </div>
      )}
    </div>
  );
}

function MenuItem({ icon, children, onClick, strong, disabled }: { icon: React.ReactNode; children: React.ReactNode; onClick: () => void; strong?: boolean; disabled?: boolean }) {
  return (
    <button role="menuitem" disabled={disabled} onClick={onClick} className={`flex h-11 w-full items-center gap-3 px-3.5 text-left text-sm hover:bg-shoal disabled:text-muted disabled:hover:bg-transparent ${strong ? "font-semibold" : ""}`}>
      <span className="w-4 text-muted">{icon}</span>
      {children}
    </button>
  );
}

function Modal({ label, children, onClose, width = "max-w-[560px]" }: { label: string; children: React.ReactNode; onClose?: () => void; width?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.focus();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose?.();
    document.addEventListener("keydown", esc);
    return () => document.removeEventListener("keydown", esc);
  }, [onClose]);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-ink/40 p-4" onMouseDown={(e) => e.target === e.currentTarget && onClose?.()}>
      <div ref={ref} tabIndex={-1} role="dialog" aria-modal="true" aria-label={label} style={{ outline: "none" }} className={`flex max-h-[94vh] w-full ${width} flex-col overflow-auto rounded-2xl border hairline bg-panel p-6 shadow-2xl`}>
        {children}
      </div>
    </div>
  );
}

/** "Which boat is this?" when AIS identified the boat (or none was found). */
export function VesselDialog() {
  const s = useSession((x) => x.status);
  const [dismissed, setDismissed] = useState<string[]>(() => load("keelwire.vesselskip", []));
  const sess = s?.session;
  const v = sess?.vessel;
  const now = useNow(5000);
  const [name, setName] = useState("");
  const [hin, setHin] = useState("");
  const [mode, setMode] = useState<"ais" | "manual">("ais");
  const key = sess ? `${sess.id}:${v?.mmsi ?? "none"}` : "";
  useEffect(() => {
    if (v) {
      setName(v.name ?? "");
      setHin(v.hin ?? "");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, v?.name]);
  if (!s || !sess || !v || v.confirmed || dismissed.includes(key)) return null;
  // Without AIS, ask only after a minute (static data may still be coming).
  const noAis = v.source === "none";
  if (noAis && now - sess.started_ms < 60_000) return null;
  if (s.state === "none" || sess.ask_resume) return null;
  const skip = () => {
    const n = [...dismissed, key].slice(-50);
    save("keelwire.vesselskip", n);
    setDismissed(n);
  };
  const manual = noAis || mode === "manual";
  const confirm = () => {
    if (manual) sessionApi.update({ manual: true, name, hin });
    else sessionApi.update({ confirm: true, ...(name !== (v.name ?? "") ? { name } : {}), ...(hin ? { hin } : {}) });
  };
  const seen = sess.visit_number > 1 && sess.previous;
  return (
    <Modal label="Which boat is this?" onClose={skip}>
      <p className="text-xs font-semibold uppercase tracking-wider text-muted">
        New session · {fmtDate(sess.started_ms, true)}, {fmtTime(sess.started_ms)}
      </p>
      <h2 className="mt-1 text-[22px] font-semibold">Which boat is this?</h2>
      {!manual && (
        <div className="mt-4 flex gap-3.5 rounded-xl border-[1.5px] border-sea bg-sea/5 p-4">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-sea/10 text-sea">
            <Radio size={20} />
          </span>
          <div className="min-w-0 flex-1">
            <p className="text-[13px] font-semibold text-sea">Found on this boat’s AIS transponder</p>
            <p className="mt-0.5 text-lg font-bold tracking-wide">{vesselName(v)}</p>
            <dl className="mt-1.5 grid grid-cols-[auto_1fr] gap-x-3.5 gap-y-0.5 text-[13px]">
              <dt className="text-muted">MMSI</dt>
              <dd className="font-mono">{v.mmsi}</dd>
              {v.callsign && (
                <>
                  <dt className="text-muted">Call sign</dt>
                  <dd className="font-mono">{v.callsign}</dd>
                </>
              )}
              <dt className="text-muted">AIS</dt>
              <dd>
                Class {v.ais_class ?? "?"}
                {v.ship_type ? ` · ${v.ship_type.toLowerCase()}` : ""}
                {v.length_m ? ` · ${v.length_m.toFixed(1)} × ${(v.beam_m ?? 0).toFixed(1)} m` : ""}
              </dd>
            </dl>
            <p className="mt-2 text-[12.5px] text-muted">
              The transponder marks these reports as <em>transmitted</em>, so they describe this boat, not one nearby{s.other_ships ? ` (${s.other_ships} other ${s.other_ships === 1 ? "ship" : "ships"} heard)` : ""}.
            </p>
          </div>
        </div>
      )}
      {noAis && <p className="mt-3 text-sm text-muted">No AIS transponder has been heard on this network. Add a name or hull ID so the session is filed with the boat’s history, or skip and it is saved under the date.</p>}
      {seen && !manual && (
        <div className="mt-3 flex items-center gap-2.5 rounded-lg border border-stbd/30 bg-stbd/5 px-3.5 py-2.5 text-[13px]">
          <Check size={16} className="text-stbd" />
          <span>
            <strong className="font-semibold">Seen before.</strong> {sess.visit_number - 1} earlier {sess.visit_number === 2 ? "session" : "sessions"}, last on {fmtDate(sess.previous!.started_ms)}. This one will be added to its history.
          </span>
        </div>
      )}
      <fieldset className="mt-4 flex flex-col gap-2.5">
        <legend className="mb-1.5 text-[13px] font-semibold">Details (optional)</legend>
        <label className="flex flex-col gap-1 text-[12.5px] text-muted">
          Boat name
          <input className="input h-10 text-[14px]" value={name} onChange={(e) => setName(e.target.value)} />
        </label>
        <label className="flex flex-col gap-1 text-[12.5px] text-muted">
          Hull ID (HIN)
          <input className="input h-10 font-mono text-[14px]" placeholder="12 characters, e.g. ABC12345D626" value={hin} maxLength={17} onChange={(e) => setHin(e.target.value.toUpperCase())} />
        </label>
      </fieldset>
      <footer className="mt-6 flex flex-wrap justify-between gap-2.5">
        <button className="btn h-10" onClick={skip}>
          Skip
        </button>
        <div className="flex gap-2.5">
          {!noAis && (
            <button className="btn h-10" onClick={() => setMode(mode === "ais" ? "manual" : "ais")}>
              {mode === "ais" ? "Different boat" : "Use AIS"}
            </button>
          )}
          <button className="btn btn-primary h-10 font-semibold" onClick={confirm} disabled={manual && !name.trim() && !hin.trim()}>
            {manual ? "Save details" : "Yes, this is the boat"}
          </button>
        </div>
      </footer>
    </Modal>
  );
}

/** After a long gap or a restart: continue the open session or start a new one. */
export function ResumeDialog() {
  const s = useSession((x) => x.status);
  const connected = useApp((x) => x.status?.interface_status?.state === "connected");
  const sess = s?.session;
  if (!sess?.ask_resume || !connected) return null;
  const today = new Date(sess.started_ms).toDateString() === new Date().toDateString();
  return (
    <Modal label="Continue the session?">
      <h2 className="text-[20px] font-semibold">Continue {today ? "this session" : "the last session"}?</h2>
      <p className="mt-2 text-sm text-muted">
        A session on <strong className="text-ink">{vesselName(sess.vessel)}</strong> started {today ? "today" : fmtDate(sess.started_ms)} at {fmtTime(sess.started_ms)} and is still open ({sess.changes.filter((c) => c.result !== "failed").length} changes, {sess.notes.length} notes).
      </p>
      <p className="mt-2 text-sm text-muted">Continue it if this is the same visit. Starting a new one saves the old session to the boat’s history first.</p>
      <footer className="mt-6 flex flex-wrap justify-end gap-2.5">
        <button className="btn h-10" onClick={sessionApi.startNew}>
          Start a new session
        </button>
        <button className="btn btn-primary h-10 font-semibold" onClick={sessionApi.resume}>
          Continue this session
        </button>
      </footer>
    </Modal>
  );
}

/** End the session: choose what to save. */
export function EndSessionDialog() {
  const { status: s, endOpen, setEndOpen, lastEnd } = useSession();
  const [opts, setOpts] = useState<EndOptions>(() => load("keelwire.endopts", { owner_pdf: true, technician: "pdf", bundle: true, strip_other_ais: false }));
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<typeof lastEnd>(null);
  const setPage = useApp((x) => x.setPage);
  const close = () => {
    setEndOpen(false);
    setDone(null);
  };
  if (!endOpen) return null;
  if (done) {
    return (
      <Modal label="Session saved" onClose={close}>
        <div className="flex items-start justify-between">
          <div>
            <p className="text-xs font-semibold uppercase tracking-wider text-stbd">Session saved</p>
            <h2 className="mt-1 text-[22px] font-bold tracking-wide">{done.rel.split("/")[0]}</h2>
          </div>
          <button className="btn btn-sm" aria-label="Close" onClick={close}>
            <X size={15} />
          </button>
        </div>
        {canShare() && (
          <button className="btn btn-primary mt-4 h-11 w-full font-semibold" onClick={() => shareHistory(done.files.filter((f) => /^(Owner|Technician) report/.test(f)).map((f) => `${done.rel}/${f}`))}>
            <Share size={16} /> Share the reports
          </button>
        )}
        <ul className="mt-4 divide-y rounded-xl border hairline [&>li]:border-[rgb(var(--rule))]">
          {done.files.filter((f) => !f.endsWith(".json")).map((f) => (
            <li key={f} className="flex items-center gap-3 px-3.5 py-2.5 text-sm">
              <span className="min-w-0 flex-1 truncate">{f}</span>
              {/\.(pdf|html)$/.test(f) && !(canShare() && f.endsWith(".pdf")) && (
                <button className="btn btn-sm" onClick={() => openReport(`${done.rel}/${f}`)}>
                  Open
                </button>
              )}
              <button className="btn btn-sm" aria-label={`${canShare() ? "Share" : "Download"} ${f}`} onClick={() => shareHistory([`${done.rel}/${f}`])}>
                {canShare() ? <Share size={14} /> : <Download size={14} />}
              </button>
            </li>
          ))}
        </ul>
        <p className="mt-3 break-all font-mono text-[12px] text-muted">{isMobileApp() ? `Files › On My ${/iPad/.test(navigator.userAgent) ? "iPad" : "iPhone"} › Keelwire › ${done.rel}` : done.dir}</p>
        <footer className="mt-5 flex justify-end gap-2.5">
          <button className="btn h-10" onClick={() => (close(), setPage("boats"))}>
            <FolderOpen size={15} /> Boats
          </button>
          <button className="btn btn-primary h-10" onClick={close}>
            Done
          </button>
        </footer>
      </Modal>
    );
  }
  const sess = s?.session;
  if (!s || !sess) return null;
  const set = (p: Partial<EndOptions>) => {
    const n = { ...opts, ...p };
    save("keelwire.endopts", n);
    setOpts(n);
  };
  const end = sess.ended_ms ?? s.now_ms;
  const folder = `${(sess.vessel.name ? sess.vessel.name.replace(/\b\w+/g, (w) => (w === w.toUpperCase() ? w[0] + w.slice(1).toLowerCase() : w)) : "Unnamed boats")}${sess.vessel.mmsi ? ` (${sess.vessel.mmsi})` : sess.vessel.hin ? ` (${sess.vessel.hin})` : ""}`;
  const go = async () => {
    setBusy(true);
    const r = await sessionApi.end(opts);
    setBusy(false);
    if (r) setDone(r);
  };
  const box = (on: boolean) => `flex items-start gap-3 rounded-xl border-[1.5px] p-3.5 ${on ? "border-sea bg-sea/5" : "hairline"}`;
  return (
    <Modal label="End session" onClose={busy ? undefined : close} width="max-w-[600px]">
      <p className="text-xs font-semibold uppercase tracking-wider text-muted">End session</p>
      <h2 className="mt-1 text-[22px] font-bold tracking-wide">{vesselName(sess.vessel)}</h2>
      <p className="text-[13px] text-muted">
        {sess.vessel.mmsi && <span className="font-mono">MMSI {sess.vessel.mmsi} · </span>}
        {fmtDate(sess.started_ms, true)}, {fmtTime(sess.started_ms)}–{fmtTime(end)} ({Math.round((end - sess.started_ms) / 60000)} min)
      </p>
      <div className="mt-4 grid grid-cols-3 gap-2.5">
        <Stat label="Changes made" value={String(s.changes_ok ?? 0)} note={(s.changes_ok ?? 0) > 0 && sess.changes.every((c) => c.kind !== "setting" || c.result !== "sent") ? "all confirmed" : undefined} />
        <Stat label="Problems" value={`${s.fixed ?? 0} fixed`} warn={s.open ? `${s.open} open` : undefined} />
        <Stat label="Traffic recorded" value={fmtBytes(s.bytes ?? 0)} />
      </div>
      <fieldset className="mt-4 flex flex-col gap-2">
        <legend className="mb-1.5 text-[13px] font-semibold">Save</legend>
        <label className={box(opts.owner_pdf)}>
          <input type="checkbox" className="mt-0.5 h-[18px] w-[18px] accent-[rgb(var(--sea))]" checked={opts.owner_pdf} onChange={(e) => set({ owner_pdf: e.target.checked })} />
          <span className="flex-1">
            <strong className="font-semibold">Owner report</strong> <span className="text-muted">· PDF</span>
            <br />
            <span className="text-[13px] text-muted">Plain language: what was found, what was fixed, what still needs doing.</span>
          </span>
        </label>
        <div className={box(opts.technician != null)}>
          <input type="checkbox" aria-label="Technician report" className="mt-0.5 h-[18px] w-[18px] accent-[rgb(var(--sea))]" checked={opts.technician != null} onChange={(e) => set({ technician: e.target.checked ? "pdf" : null })} />
          <span className="flex-1">
            <strong className="font-semibold">Technician report</strong>
            <br />
            <span className="text-[13px] text-muted">Every change with old and new values, devices before and after, findings and bus statistics.</span>
          </span>
          <span role="group" aria-label="Technician report format" className="flex gap-0.5 rounded-lg border hairline bg-shoal p-0.5">
            {(["pdf", "html"] as const).map((f) => (
              <button key={f} aria-pressed={opts.technician === f} onClick={() => set({ technician: f })} className={`h-7 rounded-md px-2.5 text-[12.5px] ${opts.technician === f ? "bg-panel font-semibold shadow-sm" : "text-muted"}`}>
                {f.toUpperCase()}
              </button>
            ))}
          </span>
        </div>
        <label className={box(opts.bundle)}>
          <input type="checkbox" className="mt-0.5 h-[18px] w-[18px] accent-[rgb(var(--sea))]" checked={opts.bundle} onChange={(e) => set({ bundle: e.target.checked })} />
          <span className="flex-1">
            <strong className="font-semibold">Session bundle</strong> <span className="text-muted">· .zip</span>
            <br />
            <span className="text-[13px] text-muted">The reports, the boat files before and after, the capture (opens in Wireshark), the change log and statistics as spreadsheets.</span>
          </span>
        </label>
      </fieldset>
      <div className="mt-3 rounded-xl bg-shoal px-3.5 py-2.5 text-[13px] text-muted">
        Reports cover only this boat.{" "}
        {s.other_ships > 0 ? `The ${s.other_ships} other ${s.other_ships === 1 ? "ship" : "ships"} heard on AIS stay in the capture for troubleshooting.` : "Other ships heard on AIS stay in the capture for troubleshooting."}
        <label className="mt-1.5 flex items-center gap-2 text-ink">
          <input type="checkbox" className="h-4 w-4" checked={opts.strip_other_ais} onChange={(e) => set({ strip_other_ais: e.target.checked })} />
          Remove other ships from the capture too
        </label>
      </div>
      <div className="mt-3 text-[13px]">
        <div className="text-muted">Saved to</div>
        <div className="mt-1 truncate rounded-lg border hairline bg-shoal px-2.5 py-2 text-left font-mono text-[12px]" style={{ direction: "rtl" }} title={`${s.sessions_dir}/${folder}/`}>
          <bdi>{s.sessions_dir}/{folder}/</bdi>
        </div>
        <p className="mt-1.5 text-muted">Added to the boat’s history either way, so the next visit can show what changed.</p>
      </div>
      <footer className="mt-5 flex justify-end gap-2.5">
        <button className="btn h-10" onClick={close} disabled={busy}>
          Keep working
        </button>
        <button className="btn btn-primary h-10 font-semibold" onClick={go} disabled={busy}>
          {busy ? "Saving…" : "End and save"}
        </button>
      </footer>
    </Modal>
  );
}

function Stat({ label, value, note, warn }: { label: string; value: string; note?: string; warn?: string }) {
  return (
    <div className="rounded-xl border hairline px-3 py-2.5">
      <div className="text-xs text-muted">{label}</div>
      <div className="text-[19px] font-semibold">
        {value}
        {note && <span className="ml-1.5 text-[12.5px] font-medium text-stbd">{note}</span>}
        {warn && <span className="text-caution"> · {warn}</span>}
      </div>
    </div>
  );
}
