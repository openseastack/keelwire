// Boats: every saved session on this computer, filed per boat.
import { useEffect, useMemo, useState } from "react";
import { Search, FileText, Package, Radio } from "lucide-react";
import { api } from "../lib/api";
import { fmtDate, fmtTime, type Session, type Vessel } from "../lib/session";
import { shareHistory } from "../lib/files";
import { openReport } from "../components/ReportViewer";

interface Summary {
  id: string;
  dir: string;
  started_ms: number;
  ended_ms: number | null;
  technician: string | null;
  changes: number;
  fixed: number;
  open: number;
  open_text: string[];
  files: string[];
}

interface History {
  id: string;
  vessel: Vessel;
  folder: string;
  sessions: Summary[];
}

const KIND: Record<string, [string, string]> = {
  added: ["Added", "bg-stbd/10 text-stbd"],
  gone: ["Gone", "bg-port/10 text-port"],
  software: ["Software", "bg-sea/10 text-sea"],
  setting: ["Setting", "bg-caution/10 text-caution"],
};

export function Boats() {
  const [data, setData] = useState<{ dir: string; vessels: History[] } | null>(null);
  const [q, setQ] = useState("");
  const [sel, setSel] = useState<string | null>(null);
  const [detail, setDetail] = useState<Session | null>(null);

  useEffect(() => {
    api<{ dir: string; vessels: History[] }>("/history")
      .then((d) => {
        setData(d);
        if (d.vessels[0]) setSel((s) => s ?? d.vessels[0].id);
      })
      .catch(() => setData({ dir: "", vessels: [] }));
  }, []);

  const list = useMemo(() => {
    const t = q.trim().toLowerCase();
    return (data?.vessels ?? []).filter((v) => !t || `${v.vessel.name ?? ""} ${v.vessel.mmsi ?? ""} ${v.vessel.hin ?? ""} ${v.vessel.callsign ?? ""}`.toLowerCase().includes(t));
  }, [data, q]);
  const cur = list.find((v) => v.id === sel) ?? list[0];

  useEffect(() => {
    setDetail(null);
    const s = cur?.sessions[0];
    if (!cur || !s) return;
    api<Session>(`/history/file?path=${encodeURIComponent(`${cur.folder}/${s.dir}/session.json`)}`)
      .then(setDetail)
      .catch(() => {});
  }, [cur?.id, cur?.sessions[0]?.dir]); // eslint-disable-line react-hooks/exhaustive-deps

  const fileOf = (s: Summary, re: RegExp) => s.files.find((f) => re.test(f));
  return (
    <div className="h-full overflow-auto">
      <div className="mx-auto flex max-w-[1280px] flex-col gap-4 px-4 py-5 sm:px-7">
        <header className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="text-2xl font-semibold">Boats</h1>
            <p className="text-[13px] text-muted">Every saved session on this computer, filed by MMSI, hull ID or name</p>
          </div>
          <label className="relative w-full max-w-xs">
            <span className="sr-only">Search boats</span>
            <Search size={16} className="pointer-events-none absolute left-3 top-2.5 text-muted" aria-hidden />
            <input className="input w-full pl-9" placeholder="Name, MMSI or HIN" value={q} onChange={(e) => setQ(e.target.value)} />
          </label>
        </header>

        {data && data.vessels.length === 0 && (
          <div className="card p-8 text-center text-muted">
            No saved sessions yet. End a session to file it here.
            <div className="mt-2 break-all font-mono text-xs">{data.dir}</div>
          </div>
        )}

        {list.length > 0 && (
          <div className="flex flex-wrap items-start gap-4">
            <nav aria-label="Boats" className="card w-full overflow-hidden md:w-[320px]">
              {list.map((v) => {
                const on = v.id === cur?.id;
                const last = v.sessions[0];
                return (
                  <button key={v.id} onClick={() => setSel(v.id)} aria-current={on ? "true" : undefined} className={`block w-full border-b hairline px-4 py-3 text-left last:border-b-0 ${on ? "bg-sea/5 shadow-[inset_3px_0_0_rgb(var(--sea))]" : "hover:bg-shoal"}`}>
                    <span className={`block font-bold tracking-wide ${v.vessel.name ? "" : "font-semibold text-muted"}`}>{v.vessel.name || "Unnamed boat"}</span>
                    <span className="block text-[12.5px] text-muted">
                      {v.vessel.mmsi ? <span className="font-mono">MMSI {v.vessel.mmsi}</span> : v.vessel.hin ? <span className="font-mono">HIN {v.vessel.hin}</span> : "no AIS"}
                      {" · "}
                      {v.sessions.length} {v.sessions.length === 1 ? "session" : "sessions"}
                      {last ? ` · ${fmtDate(last.started_ms)}` : ""}
                    </span>
                  </button>
                );
              })}
            </nav>

            {cur && (
              <section aria-label={cur.vessel.name ?? "Boat"} className="flex min-w-0 flex-[999_1_520px] flex-col gap-4">
                <div className="card flex flex-wrap justify-between gap-3 px-5 py-4">
                  <div>
                    <h2 className="text-[22px] font-bold tracking-wide">{cur.vessel.name || "Unnamed boat"}</h2>
                    <p className="text-[13px] text-muted">
                      {[cur.vessel.mmsi ? `MMSI ${cur.vessel.mmsi}` : null, cur.vessel.callsign ? `call sign ${cur.vessel.callsign}` : null, cur.vessel.hin ? `HIN ${cur.vessel.hin}` : null, cur.vessel.ais_class ? `AIS Class ${cur.vessel.ais_class}` : null, cur.vessel.length_m ? `${cur.vessel.length_m.toFixed(1)} × ${(cur.vessel.beam_m ?? 0).toFixed(1)} m` : null]
                        .filter(Boolean)
                        .join(" · ")}
                    </p>
                  </div>
                  {cur.vessel.source === "ais" && (
                    <span className="flex items-center gap-1 self-start text-xs font-semibold text-stbd">
                      <Radio size={13} /> Identified by AIS
                    </span>
                  )}
                </div>

                {detail?.previous && (
                  <div className="card overflow-hidden">
                    <div className="flex flex-wrap items-center justify-between gap-2 border-b hairline px-4 py-3">
                      <h3 className="text-[15px] font-semibold">Since the previous visit</h3>
                      <span className="text-[12.5px] text-muted">
                        {fmtDate(detail.previous.started_ms)} → {fmtDate(detail.started_ms)}
                      </span>
                    </div>
                    {(detail.since_last ?? []).length === 0 ? (
                      <p className="px-4 py-3 text-sm text-muted">No changes to devices, software or settings.</p>
                    ) : (
                      <table className="w-full text-[13.5px]">
                        <tbody>
                          {(detail.since_last ?? []).map((d, i) => (
                            <tr key={i} className="border-b hairline last:border-b-0">
                              <td className="w-[110px] px-4 py-2.5">
                                <span className={`chip ${KIND[d.kind]?.[1] ?? "bg-shoal"}`}>{KIND[d.kind]?.[0] ?? d.kind}</span>
                              </td>
                              <td className="px-4 py-2.5">{d.text}</td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    )}
                  </div>
                )}

                <div className="card overflow-hidden">
                  <div className="border-b hairline px-4 py-3">
                    <h3 className="text-[15px] font-semibold">Sessions</h3>
                  </div>
                  {cur.sessions.map((s) => {
                    const owner = fileOf(s, /^Owner report/);
                    const tech = fileOf(s, /^Technician report/);
                    const zip = fileOf(s, /\.zip$/);
                    const p = (f: string) => `${cur.folder}/${s.dir}/${f}`;
                    const dur = s.ended_ms ? Math.round((s.ended_ms - s.started_ms) / 60000) : null;
                    return (
                      <div key={s.dir} className="grid gap-x-3 gap-y-1 border-b hairline px-4 py-3 last:border-b-0 sm:grid-cols-[170px_1fr_auto] sm:items-center">
                        <span>
                          <strong className="font-semibold">{fmtDate(s.started_ms)}</strong>
                          <br />
                          <span className="text-[12.5px] text-muted">
                            {fmtTime(s.started_ms)}
                            {dur != null ? ` · ${dur >= 60 ? `${Math.floor(dur / 60)} h ${dur % 60} min` : `${dur} min`}` : ""}
                            {s.technician ? ` · ${s.technician}` : ""}
                          </span>
                        </span>
                        <span className="text-[13.5px]">
                          {s.changes === 0 && s.fixed === 0 ? "Survey only, no changes" : `${s.changes} ${s.changes === 1 ? "change" : "changes"}${s.fixed ? ` · ${s.fixed} fixed` : ""}`}
                          {s.open > 0 && <span className="text-caution"> · {s.open} open{s.open_text[0] ? ` (${s.open_text[0].slice(0, 60)}${s.open_text[0].length > 60 ? "…" : ""})` : ""}</span>}
                        </span>
                        <span className="flex flex-wrap gap-2">
                          {owner && (
                            <button className="btn btn-sm" onClick={() => openReport(p(owner))}>
                              <FileText size={14} /> Owner
                            </button>
                          )}
                          {tech && (
                            <button className="btn btn-sm" onClick={() => openReport(p(tech))}>
                              <FileText size={14} /> Technician
                            </button>
                          )}
                          {zip && (
                            <button className="btn btn-sm" onClick={() => shareHistory([p(zip)])}>
                              <Package size={14} /> Bundle
                            </button>
                          )}
                        </span>
                      </div>
                    );
                  })}
                </div>
                <p className="break-all font-mono text-[12px] text-muted">
                  {data?.dir}/{cur.folder}
                </p>
              </section>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
