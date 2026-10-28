import { useEffect, useRef, useState } from "react";
import { Wifi, Usb, RefreshCw, Ship, FileUp, PlayCircle, Cpu, Server, Radio } from "lucide-react";
import { api, getToken, post } from "../lib/api";
import { useApp } from "../lib/store";
import type { InterfaceKind } from "../lib/types";

interface Port {
  path: string;
  description: string;
  suggested: string | null;
}

/** First run, or "Change connection": pick how this computer reaches the bus. */
export function Connect() {
  const { status, toast, setPage } = useApp();
  const [ports, setPorts] = useState<Port[] | null>(null);
  const [searchedAt, setSearchedAt] = useState<number>(0);
  const [kinds, setKinds] = useState<InterfaceKind[]>([]);
  const [ipg, setIpg] = useState("");
  const [bus, setBus] = useState("6543");
  const [uri, setUri] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const passive = status?.tx_mode === "passive";
  // iPhone / iPad: no USB, no file paths; gateways are reached over Wi-Fi.
  const mobile = status?.platform === "ios" || status?.platform === "android";
  const [recent, setRecent] = useState<{ uri: string; label: string }[]>(() => {
    try {
      return JSON.parse(localStorage.getItem("keelwire.recent") ?? "[]");
    } catch {
      return [];
    }
  });
  const [wifi, setWifi] = useState("");
  const [wifiKind, setWifiKind] = useState<"ydraw" | "actisense-ascii">("ydraw");

  const search = () =>
    mobile ? Promise.resolve(setPorts([])) :
    api<Port[]>("/serial-ports")
      .then((p) => {
        setPorts(p);
        setSearchedAt(Date.now());
      })
      .catch(() => setPorts([]));
  useEffect(() => {
    search();
    api<InterfaceKind[]>("/interfaces").then(setKinds).catch(() => {});
  }, []);

  const go = async (u: string, label: string) => {
    setBusy(u);
    try {
      await post("/interface", { uri: u });
      toast("info", `Connecting to ${label}`);
      if (!u.startsWith("sim:")) {
        const n = [{ uri: u, label }, ...recent.filter((r) => r.uri !== u)].slice(0, 5);
        setRecent(n);
        try {
          localStorage.setItem("keelwire.recent", JSON.stringify(n));
        } catch {
          /* storage unavailable */
        }
      }
      setPage("overview");
    } catch (e) {
      toast("error", (e as Error).message);
    } finally {
      setBusy(null);
    }
  };
  const openBoat = async (f: File) => {
    try {
      const r = await fetch("/api/v1/boatfile", { method: "POST", headers: { "Content-Type": "application/json", "X-File-Name": f.name, ...(getToken() ? { Authorization: `Bearer ${getToken()}` } : {}) }, body: await f.text() });
      const j = await r.json();
      if (!r.ok) throw new Error(j.message);
      toast("ok", `Opened ${f.name}: ${j.devices} devices (offline)`);
      setPage("overview");
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };
  const usable = (ports ?? []).filter((p) => p.suggested);
  const other = (ports ?? []).filter((p) => !p.suggested);
  const usb100 = kinds.find((k) => k.scheme === "usb100");

  return (
    <div className="scroll-thin h-full overflow-auto">
      <main className="mx-auto flex max-w-3xl flex-col gap-6 px-4 py-10 sm:py-16">
        <div className="flex items-center gap-3">
          <svg width="36" height="36" viewBox="0 0 30 30" role="img" aria-label="Keelwire">
            <rect width="30" height="30" rx="8" fill="rgb(var(--ink))" />
            <path d="M6 19h18M9 19v-5M15 19v-8M21 19v-3" stroke="rgb(var(--paper))" strokeWidth="2.2" strokeLinecap="round" />
          </svg>
          <span className="text-base font-semibold">Keelwire</span>
        </div>
        <div>
          <h1 className="text-3xl font-semibold tracking-tight sm:text-[34px]">{status?.interface ? "Change connection" : "Connect to your boat's network"}</h1>
          <p className="mt-2 text-base text-muted">Pick the gateway that links this {mobile ? "device" : "computer"} to the NMEA 2000 backbone.</p>
          {mobile && <p className="mt-2 text-sm text-muted">Join the boat’s Wi-Fi first. The first time, allow Keelwire to find devices on your local network; if you said no, turn it on in Settings › Privacy & Security › Local Network.</p>}
          {status?.interface && (
            <p className="mt-2 text-sm">
              Now connected to <strong>{status.interface.name}</strong>.{" "}
              <button className="text-sea hover:underline" onClick={() => setPage("overview")}>
                Keep it
              </button>
            </p>
          )}
        </div>

        {recent.length > 0 && (
          <section className="card overflow-hidden" aria-labelledby="recent">
            <h2 id="recent" className="px-5 py-3.5 text-sm font-semibold">
              Recent gateways
            </h2>
            {recent.map((r) => (
              <div key={r.uri} className="flex flex-wrap items-center gap-3 border-t hairline px-5 py-3">
                <div className="min-w-0 flex-1">
                  <p className="font-semibold">{r.label}</p>
                  <p className="truncate font-mono text-[12.5px] text-muted">{r.uri.replace(/^[a-z0-9-]+:(tcp:\/\/)?/, "")}</p>
                </div>
                <button className="btn btn-primary" disabled={busy != null} onClick={() => go(r.uri, r.label)}>
                  Connect
                </button>
              </div>
            ))}
          </section>
        )}

        {mobile && (
          <section className="card p-5" aria-labelledby="wifi">
            <div className="flex items-start gap-4">
              <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-shoal">
                <Radio size={20} />
              </span>
              <div className="min-w-0 flex-1">
                <h2 id="wifi" className="font-semibold">
                  Wi-Fi gateway
                </h2>
                <p className="text-[13px] text-muted">Keelwire Gateway, Yacht Devices YDWG-02 / YDEN-02 or Actisense W2K-1: enter its address.</p>
                <form
                  className="mt-3 flex flex-wrap gap-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    const host = wifi.trim();
                    if (!host) return;
                    const u = wifiKind === "ydraw" ? `ydraw:tcp://${host.includes(":") ? host : `${host}:1457`}` : `actisense-ascii:tcp://${host.includes(":") ? host : `${host}:60001`}`;
                    go(u, wifiKind === "ydraw" ? "Wi-Fi gateway" : "Actisense W2K-1");
                  }}
                >
                  <label className="sr-only" htmlFor="wifi-addr">
                    Gateway address
                  </label>
                  <input id="wifi-addr" className="input min-w-0 flex-[1_1_180px] font-mono" inputMode="url" autoCapitalize="off" autoCorrect="off" placeholder="192.168.4.1" value={wifi} onChange={(e) => setWifi(e.target.value)} />
                  <label className="sr-only" htmlFor="wifi-kind">
                    Gateway type
                  </label>
                  <select id="wifi-kind" className="input" value={wifiKind} onChange={(e) => setWifiKind(e.target.value as "ydraw" | "actisense-ascii")}>
                    <option value="ydraw">Keelwire / Yacht Devices</option>
                    <option value="actisense-ascii">Actisense W2K-1</option>
                  </select>
                  <button className="btn btn-primary" type="submit" disabled={!wifi.trim() || busy != null}>
                    Connect
                  </button>
                </form>
              </div>
            </div>
          </section>
        )}

        {!mobile && (
        <section className="card overflow-hidden" aria-labelledby="usb">
          <h2 id="usb" className="flex flex-wrap items-center gap-2 px-5 py-3.5 text-sm font-semibold">
            USB gateways
            <span className="label font-normal">{ports == null ? "searching…" : `checked ${Math.max(0, Math.round((Date.now() - searchedAt) / 1000))} s ago`}</span>
            <button className="btn btn-sm ml-auto" onClick={search}>
              <RefreshCw size={13} /> Search again
            </button>
          </h2>
          {ports && usable.length === 0 && <p className="border-t hairline px-5 py-4 text-sm text-muted">No recognised USB gateway is plugged into the computer running Keelwire.</p>}
          {usable.map((p) => {
            const isUsb100 = p.suggested!.startsWith("usb100:");
            const off = isUsb100 && !usb100?.available;
            return (
              <div key={p.path} className={`flex flex-wrap items-center gap-4 border-t hairline px-5 py-4 ${off ? "bg-shoal/60" : ""}`}>
                <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-shoal">
                  <Usb size={20} />
                </span>
                <div className="min-w-0 flex-[1_1_260px]">
                  <p className="font-semibold">{p.description}</p>
                  <p className="font-mono text-[13px] text-muted">{off ? "Not supported yet. Use an IPG100, or record a capture to help add it." : p.path}</p>
                </div>
                <button className="btn btn-primary" disabled={off || busy != null} onClick={() => go(p.suggested!, p.description)}>
                  Connect
                </button>
              </div>
            );
          })}
          {other.length > 0 && (
            <details className="border-t hairline px-5 py-3 text-sm">
              <summary className="cursor-pointer text-muted">{other.length} other serial port{other.length > 1 ? "s" : ""}</summary>
              <ul className="mt-2 space-y-1">
                {other.map((p) => (
                  <li key={p.path} className="flex items-center gap-3">
                    <span className="font-mono">{p.path}</span>
                    <span className="text-muted">{p.description}</span>
                    <button className="btn btn-sm ml-auto" onClick={() => go(`actisense:${p.path}`, p.path)}>
                      Try as Actisense
                    </button>
                  </li>
                ))}
              </ul>
            </details>
          )}
        </section>
        )}

        <section className="card p-5" aria-labelledby="ipg">
          <div className="flex items-start gap-4">
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-shoal">
              <Wifi size={20} />
            </span>
            <div className="min-w-0 flex-1">
              <h2 id="ipg" className="font-semibold">
                Maretron IPG100
              </h2>
              <p className="text-[13px] text-muted">Enter the address shown in its web page or your router.</p>
              <form
                className="mt-3 flex flex-wrap gap-2"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (ipg.trim()) go(`ipg100:tcp://${ipg.trim()}:${bus}`, "IPG100");
                }}
              >
                <label className="sr-only" htmlFor="ipg-addr">
                  IPG100 address
                </label>
                <input id="ipg-addr" className="input min-w-0 flex-[1_1_180px] font-mono" placeholder="192.168.1.50" value={ipg} onChange={(e) => setIpg(e.target.value)} />
                <label className="sr-only" htmlFor="ipg-bus">
                  Bus
                </label>
                <select id="ipg-bus" className="input" value={bus} onChange={(e) => setBus(e.target.value)}>
                  <option value="6543">Bus 0</option>
                  <option value="6553">Bus 1</option>
                </select>
                <button className="btn btn-primary" type="submit" disabled={!ipg.trim() || busy != null}>
                  Connect
                </button>
              </form>
            </div>
          </div>
        </section>

        <div className="flex items-start gap-3 rounded-xl border hairline bg-panel px-4 py-3.5">
          <input id="listen" type="checkbox" className="mt-1 h-4 w-4" checked={passive} onChange={(e) => post("/tx", { mode: e.target.checked ? "passive" : "query" }).catch((er) => toast("error", er.message))} />
          <label htmlFor="listen">
            <span className="font-semibold">Only listen</span>
            <span className="block text-sm text-muted">Nothing is sent to the bus. Device names and settings appear only if devices broadcast them. You can change this later in Settings.</span>
          </label>
        </div>

        <section aria-labelledby="else">
          <h2 id="else" className="mb-2.5 text-sm font-semibold">
            Something else
          </h2>
          <div className="grid gap-3 sm:grid-cols-3">
            <Tile icon={Ship} title={mobile ? "Demo boat" : "Try the simulated boat"} sub="A simulated boat with AIS and devices to fix; no hardware needed" onClick={() => go("sim:?conflict=1", "the demo boat")} />
            <Tile icon={FileUp} title="Open a boat file" sub="Review a network someone saved aboard" onClick={() => fileRef.current?.click()} />
            {!mobile && <Tile icon={PlayCircle} title="Replay a capture" sub="pcapng, candump or CANboat log" onClick={() => setUri("replay:/path/to/capture.pcapng")} />}
            {!mobile && <Tile icon={Radio} title="Yacht Devices Wi-Fi" sub="YDWG-02 or YDEN-02 by address" onClick={() => setUri("ydraw:tcp://192.168.4.1:1457")} />}
            {!mobile && <Tile icon={Cpu} title="CAN interface" sub="Raspberry Pi CAN HAT (SocketCAN)" onClick={() => setUri("socketcan:can0")} />}
            <Tile icon={Server} title="Signal K server" sub="Through the keelwire-tap plugin" onClick={() => setUri(kinds.find((k) => k.scheme === "signalk")?.example ?? "signalk:ws://localhost:3000")} />
          </div>
          <input ref={fileRef} type="file" accept=".keelwire,.json" hidden onChange={(e) => e.target.files?.[0] && openBoat(e.target.files[0])} />
          <form
            className="mt-4 flex flex-wrap gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (uri.trim()) go(uri.trim(), uri.trim());
            }}
          >
            <label className="sr-only" htmlFor="uri">
              Connection address
            </label>
            <input id="uri" className="input min-w-0 flex-1 font-mono" placeholder="Connection address, e.g. ydraw:tcp://192.168.4.1:1457" value={uri} onChange={(e) => setUri(e.target.value)} />
            <button className="btn" type="submit" disabled={!uri.trim() || busy != null}>
              Connect
            </button>
          </form>
        </section>
      </main>
    </div>
  );
}

function Tile({ icon: Icon, title, sub, onClick }: { icon: typeof Ship; title: string; sub: string; onClick: () => void }) {
  return (
    <button onClick={onClick} className="flex flex-col items-start gap-1 rounded-xl border hairline bg-panel px-4 py-3.5 text-left hover:border-sea/50 hover:bg-shoal/40">
      <Icon size={18} className="mb-1 text-muted" />
      <span className="font-semibold">{title}</span>
      <span className="text-xs text-muted">{sub}</span>
    </button>
  );
}
