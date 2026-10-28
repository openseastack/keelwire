import { useEffect, useRef, useState, type ReactNode } from "react";
import { canShare, saveFromApi, shareCapture } from "../lib/files";
import { Plug, Unplug, Download, Upload, Trash2, Circle, Square, ShieldAlert, Search, Share } from "lucide-react";
import { useApp, type Theme } from "../lib/store";
import { api, del, getToken, post, put } from "../lib/api";
import { fmtBytes, METRIC, US } from "../lib/format";
import type { InterfaceKind, Units } from "../lib/types";

export function Settings() {
  return (
    <div className="scroll-thin h-full overflow-auto p-4 sm:p-7">
      <h1 className="mx-auto mb-5 max-w-5xl text-2xl font-semibold tracking-tight">Settings</h1>
      <div className="mx-auto grid max-w-5xl gap-4 lg:grid-cols-2">
        <InterfaceCard />
        <TxCard />
        <CaptureCard />
        <BoatCard />
        <DisplayCard />
        <UnitsCard />
        <AboutCard />
      </div>
    </div>
  );
}

function Card({ title, children, wide }: { title: string; children: ReactNode; wide?: boolean }) {
  return (
    <section className={`card p-5 ${wide ? "lg:col-span-2" : ""}`}>
      <h2 className="mb-3 text-[15px] font-semibold">{title}</h2>
      {children}
    </section>
  );
}

function InterfaceCard() {
  const { status, toast } = useApp();
  const [kinds, setKinds] = useState<InterfaceKind[]>([]);
  const [uri, setUri] = useState("");
  const [ports, setPorts] = useState<{ path: string; description: string; suggested: string | null }[] | null>(null);
  useEffect(() => {
    api<InterfaceKind[]>("/interfaces").then(setKinds).catch(() => {});
  }, []);
  const findPorts = () => api<typeof ports>("/serial-ports").then((p) => setPorts(p ?? [])).catch((e) => toast("error", e.message));
  useEffect(() => {
    if (!uri && status?.interface?.uri) setUri(status.interface.uri);
  }, [status?.interface?.uri, uri]);
  const st = status?.interface_status;
  return (
    <Card title="Connection" wide>
      <p className="-mt-1 mb-3 text-sm text-muted">
        <button className="text-sea hover:underline" onClick={() => useApp.getState().setPage("connect")}>
          Choose a gateway
        </button>{" "}
        with the guided screen, or enter a connection address here.
      </p>
      <div className="mb-3 flex flex-wrap gap-2">
        {kinds.map((k) => (
          <button key={k.scheme} className={`btn ${uri.startsWith(k.scheme + ":") ? "!border-sea !bg-sea/10" : ""}`} disabled={!k.available} title={k.note} onClick={() => setUri(k.example)}>
            {k.label}
          </button>
        ))}
      </div>
      <form
        className="flex flex-wrap gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          post("/interface", { uri }).then(() => toast("info", `Connecting to ${uri}`)).catch((er) => toast("error", er.message));
        }}
      >
        <input className="input min-w-[320px] flex-1 font-mono" value={uri} onChange={(e) => setUri(e.target.value)} placeholder="ipg100:tcp://192.168.1.100:6543" aria-label="Interface URI" />
        <button className="btn btn-primary" type="submit" disabled={!uri}>
          <Plug size={14} /> Connect
        </button>
        <button className="btn" type="button" disabled={!status?.interface} onClick={() => del("/interface").catch((er) => toast("error", er.message))}>
          <Unplug size={14} /> Disconnect
        </button>
      </form>
      <p className="mt-2 text-sm text-muted">
        {kinds.find((k) => uri.startsWith(k.scheme + ":"))?.note}
      </p>
      <div className="mt-2">
        <button className="btn" type="button" onClick={findPorts}>
          <Search size={14} /> Find USB gateways
        </button>
        {ports && (
          <ul className="mt-2 divide-y divide-rule/60 rounded-md border hairline text-sm">
            {ports.length === 0 && <li className="px-3 py-2 text-muted">No serial ports found on the computer running Keelwire.</li>}
            {ports.map((p) => (
              <li key={p.path} className="flex items-center gap-3 px-3 py-1.5">
                <span className="font-mono">{p.path}</span>
                <span className="text-muted">{p.description}</span>
                {p.suggested ? (
                  <button className="btn ml-auto" type="button" onClick={() => setUri(p.suggested!)}>Use</button>
                ) : (
                  <span className="ml-auto text-xs text-muted">not recognised</span>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
      <p className="mt-2 text-sm">
        Status: <strong>{st?.state ?? "unknown"}</strong>
        {st?.detail ? `, ${st.detail}` : ""}
        {st?.reason ? `, ${st.reason}` : ""}
      </p>
    </Card>
  );
}

function TxCard() {
  const { status, toast } = useApp();
  const s = status;
  const call = (body: object) => post("/tx", body).catch((e) => toast("error", e.message));
  const claimText: Record<string, string> = {
    none: "not claimed",
    claiming: "claiming…",
    claimed: `claimed address ${s?.own_address}`,
    gateway_managed: "the gateway uses its own address",
    cannot_claim: "could not claim an address",
  };
  return (
    <Card title="Sending to the bus">
      <fieldset className="space-y-2">
        <legend className="sr-only">Transmit mode</legend>
        <label className="flex items-start gap-2">
          <input type="radio" name="txmode" checked={s?.tx_mode === "passive"} onChange={() => call({ mode: "passive" })} className="mt-1" />
          <span>
            <strong>Listen only.</strong> <span className="text-muted">Nothing is sent. Device details appear only if devices broadcast them.</span>
          </span>
        </label>
        <label className="flex items-start gap-2">
          <input type="radio" name="txmode" checked={s?.tx_mode === "query"} onChange={() => call({ mode: "query" })} className="mt-1" />
          <span>
            <strong>Identify devices.</strong> <span className="text-muted">Claims an address and asks devices for product information, descriptions and PGN lists. Changes no settings.</span>
          </span>
        </label>
      </fieldset>
      <p className="mt-2 text-sm text-muted">Own node: {claimText[s?.claim ?? "none"]}.</p>
      <div className={`mt-4 flex flex-wrap items-center gap-3 rounded-xl p-3 text-sm ${s?.armed ? "bg-caution/10 text-caution" : "bg-shoal"}`}>
        <ShieldAlert size={18} className="shrink-0" />
        <span className="min-w-0 flex-1">
          {s?.armed
            ? "Changes are unlocked. They lock again after 15 minutes without changes."
            : s?.allow_tx
              ? "Changes and switching are locked. Unlock them with the lock button at the top, or in place on a device or the Switching page."
              : <>This server was started without <code className="font-mono">--allow-tx</code>, so it can only read.</>}
        </span>
      </div>
      <label className="mt-3 flex items-center gap-2 text-sm">
        <input type="checkbox" checked={!!s?.expert} onChange={(e) => call({ expert: e.target.checked })} />
        Expert mode: raw frame injection, proprietary message tool, unverified layouts
      </label>
    </Card>
  );
}

interface CapFile {
  file: string;
  bytes: number;
  modified: number;
}

function CaptureCard() {
  const { status, toast } = useApp();
  const [files, setFiles] = useState<CapFile[]>([]);
  const [format, setFormat] = useState("pcapng");
  const refresh = () => api<{ files: CapFile[] }>("/captures").then((r) => setFiles(r.files)).catch(() => {});
  useEffect(() => {
    refresh();
  }, [status?.capture?.file]);
  const cap = status?.capture;
  return (
    <Card title="Capture to file">
      {cap ? (
        <div className="flex items-center gap-3">
          <Circle size={12} className="animate-pulse fill-port text-port" />
          <span className="flex-1 text-sm">
            Recording <strong>{cap.file}</strong>: {cap.frames.toLocaleString()} frames, {fmtBytes(cap.bytes)}, {cap.seconds} s
          </span>
          <button className="btn" onClick={() => post("/capture/stop").then(refresh).catch((e) => toast("error", e.message))}>
            <Square size={14} /> Stop
          </button>
        </div>
      ) : (
        <div className="flex gap-2">
          <select className="input" value={format} onChange={(e) => setFormat(e.target.value)} aria-label="Capture format">
            <option value="pcapng">pcapng (Wireshark)</option>
            <option value="candump">candump log</option>
            <option value="canboat">CANboat CSV</option>
          </select>
          <button className="btn btn-primary" onClick={() => post("/capture/start", { format }).catch((e) => toast("error", e.message))}>
            <Circle size={14} /> Start recording
          </button>
        </div>
      )}
      <ul className="mt-3 divide-y divide-rule/60 text-sm">
        {files.slice(0, 12).map((f) => (
          <li key={f.file} className="flex items-center gap-2 py-1">
            <span className="flex-1 truncate font-mono text-[12.5px]">{f.file}</span>
            <span className="text-muted">{fmtBytes(f.bytes)}</span>
            <button className="btn !px-2 !py-0.5" onClick={() => shareCapture(f.file)} aria-label={`${canShare() ? "Share" : "Download"} ${f.file}`}>
              {canShare() ? <Share size={13} /> : <Download size={13} />}
            </button>
            <button className="btn !px-2 !py-0.5" aria-label={`Delete ${f.file}`} onClick={() => confirm(`Delete ${f.file}?`) && del(`/captures/${encodeURIComponent(f.file)}`).then(refresh)}>
              <Trash2 size={13} />
            </button>
          </li>
        ))}
        {files.length === 0 && <li className="py-1 text-muted">No recordings yet.</li>}
      </ul>
      <p className="mt-3 text-xs text-muted">
        Live in Wireshark: <code className="font-mono">curl -sN {location.origin}/api/v1/stream/pcap{getToken() ? "?token=…" : ""} | wireshark -k -i -</code>
      </p>
    </Card>
  );
}

function BoatCard() {
  const { status, toast } = useApp();
  const [vessel, setVessel] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);
  const upload = async (f: File) => {
    try {
      const r = await fetch("/api/v1/boatfile", { method: "POST", headers: { "Content-Type": "application/json", "X-File-Name": f.name, ...(getToken() ? { Authorization: `Bearer ${getToken()}` } : {}) }, body: await f.text() });
      const j = await r.json();
      if (!r.ok) throw new Error(j.message);
      toast("ok", `Opened ${f.name}: ${j.devices} devices (offline)`);
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };
  return (
    <Card title="Boat files">
      <p className="mb-3 text-sm text-muted">A boat file stores every device, its settings and the latest messages, so someone ashore can review the network without being aboard.</p>
      <div className="flex flex-wrap gap-2">
        <input className="input w-48" placeholder="Vessel name" value={vessel} onChange={(e) => setVessel(e.target.value)} />
        <button className="btn" onClick={() => saveFromApi(`/boatfile${vessel ? `?vessel=${encodeURIComponent(vessel)}` : ""}`, `${vessel.trim() || "vessel"}.keelwire`)}>
          <Download size={14} /> Save boat file
        </button>
        <button className="btn" onClick={() => fileRef.current?.click()}>
          <Upload size={14} /> Open boat file
        </button>
        <input ref={fileRef} type="file" accept=".keelwire,.json" hidden onChange={(e) => e.target.files?.[0] && upload(e.target.files[0])} />
      </div>
      {status?.offline && (
        <div className="mt-3 flex items-center gap-2 rounded-md bg-shoal/50 p-2 text-sm">
          <span className="flex-1">Viewing <strong>{status.offline}</strong> offline.</span>
          <button className="btn" onClick={() => del("/boatfile").catch((e) => toast("error", e.message))}>Close boat file</button>
        </div>
      )}
    </Card>
  );
}

function DisplayCard() {
  const { theme, setTheme } = useApp();
  const opts: [Theme, string, string][] = [
    ["day", "Day", "Chart-paper colours for bright cockpits and offices."],
    ["dark", "Dark", "Charcoal with normal colours, for dim cabins or if you prefer dark mode."],
    ["night", "Night", "Red on black to keep your night vision at the helm."],
  ];
  return (
    <Card title="Display">
      <div className="grid gap-2">
        {opts.map(([t, label, desc]) => (
          <label key={t} className={`flex cursor-pointer items-start gap-2 rounded-md border p-2 ${theme === t ? "border-sea" : "hairline"}`}>
            <input type="radio" name="theme" checked={theme === t} onChange={() => setTheme(t)} className="mt-1" />
            <span>
              <strong>{label}</strong> <span className="text-sm text-muted">{desc}</span>
            </span>
          </label>
        ))}
      </div>
    </Card>
  );
}

const UNIT_OPTS: [keyof Units, string, [string, string][]][] = [
  ["depth", "Depth", [["m", "metres"], ["ft", "feet"], ["fathom", "fathoms"]]],
  ["distance", "Distance", [["nm", "nautical miles"], ["km", "kilometres"], ["mi", "statute miles"]]],
  ["speed", "Speed", [["kn", "knots"], ["km/h", "km/h"], ["mph", "mph"], ["m/s", "m/s"]]],
  ["temperature", "Temperature", [["C", "°C"], ["F", "°F"]]],
  ["volume", "Volume", [["L", "litres"], ["gal", "US gallons"], ["impgal", "imperial gallons"]]],
  ["pressure", "Pressure", [["kPa", "kPa (baro in hPa)"], ["bar", "bar"], ["psi", "psi"], ["inHg", "inHg"]]],
];

function UnitsCard() {
  const { status, toast } = useApp();
  const u = status?.units ?? METRIC;
  const save = async (next: Units) => {
    try {
      await put("/settings/units", next);
      const st = useApp.getState().status;
      if (st) useApp.setState({ status: { ...st, units: next } });
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };
  return (
    <Card title="Units">
      <div className="mb-3 flex gap-2">
        <button className="btn" onClick={() => save(METRIC)}>Metric</button>
        <button className="btn" onClick={() => save(US)}>US</button>
      </div>
      <div className="grid grid-cols-[110px_1fr] items-center gap-x-3 gap-y-2 text-sm">
        {UNIT_OPTS.map(([k, label, opts]) => (
          <label key={k} className="contents">
            <span className="text-muted">{label}</span>
            <select className="input" value={u[k]} onChange={(e) => save({ ...u, [k]: e.target.value })}>
              {opts.map(([v, l]) => (
                <option key={v} value={v}>{l}</option>
              ))}
            </select>
          </label>
        ))}
      </div>
      <p className="mt-3 text-xs text-muted">Changes how values are shown in every view (Traffic picks it up from the next message). Values on the bus are unchanged.</p>
    </Card>
  );
}

function AboutCard() {
  const s = useApp((x) => x.status);
  return (
    <Card title="About">
      <dl className="grid grid-cols-[160px_1fr] gap-y-1 text-sm">
        <dt className="text-muted">Version</dt>
        <dd>{s?.version}</dd>
        <dt className="text-muted">PGN database</dt>
        <dd>CANboat {s?.pgn_db_version}</dd>
        <dt className="text-muted">Proprietary layouts</dt>
        <dd>{s?.schemas ?? 0} loaded</dd>
        <dt className="text-muted">Own ISO NAME</dt>
        <dd className="font-mono text-xs">{s?.own_name}</dd>
        <dt className="text-muted">History buffer</dt>
        <dd>{s ? `${s.ring[0].toLocaleString()} of ${s.ring[1].toLocaleString()} messages` : ""}</dd>
      </dl>
      <p className="mt-3 text-xs text-muted">PGN definitions from the CANboat project (Apache-2.0), not from NMEA's published standard. NMEA 2000® is a registered trademark of the National Marine Electronics Association; Keelwire is not NMEA 2000 Certified and is not affiliated with or endorsed by NMEA. Maretron and other product names are trademarks of their owners, used only to identify the equipment Keelwire can connect to; Keelwire is not affiliated with them.</p>
    </Card>
  );
}
