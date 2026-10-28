import { useEffect, useMemo, useRef, useState } from "react";
import { Home, Cpu, Activity, ToggleRight, Settings as Cog, Search, LockOpen, X, Sun, Moon, MoonStar, CircleDot, Ship } from "lucide-react";
import { connect, useApp, deviceLabel, type Page, type Theme } from "./lib/store";
import { LockButton, useUnlock } from "./components/Lock";
import { Overview } from "./pages/Overview";
import { Sniffer } from "./pages/Sniffer";
import { Devices } from "./pages/Devices";
import { Switching } from "./pages/Switching";
import { Settings } from "./pages/Settings";
import { Connect } from "./pages/Connect";
import { SessionPage } from "./pages/Session";
import { Boats } from "./pages/Boats";
import { EndSessionDialog, RecButton, ResumeDialog, VesselBadge, VesselDialog } from "./components/SessionControls";
import { startSessionPolling, useSession } from "./lib/session";
import { ReportViewer } from "./components/ReportViewer";

const NAV: { id: Page; label: string; icon: typeof Home; key: string; wide?: boolean }[] = [
  { id: "overview", label: "Overview", icon: Home, key: "1" },
  { id: "session", label: "Session", icon: CircleDot, key: "5" },
  { id: "devices", label: "Devices", icon: Cpu, key: "2" },
  { id: "sniffer", label: "Traffic", icon: Activity, key: "3" },
  { id: "switching", label: "Switching", icon: ToggleRight, key: "4" },
  { id: "boats", label: "Boats", icon: Ship, key: "6", wide: true },
];

export function App() {
  const { page, setPage, theme, status } = useApp();

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    connect();
    startSessionPolling();
  }, [theme]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.altKey || e.ctrlKey || e.metaKey) return;
      const t = e.target as HTMLElement;
      if (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT") return;
      if (e.key === "/") {
        e.preventDefault();
        document.getElementById("global-search")?.focus();
        return;
      }
      const n = NAV.find((x) => x.key === e.key);
      if (n) setPage(n.id);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPage]);

  // No bus and no boat file open: the first-run Connect screen comes first.
  const needsConnect = !!status && !status.interface && !status.offline;
  const show: Page = page === "connect" || (needsConnect && page !== "settings") ? "connect" : page;

  return (
    <div className="safe-shell flex h-full flex-col sm:flex-row">
      <Nav current={show} />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {show !== "connect" && <TopBar />}
        <UnlockBanner />
        <main className="min-h-0 flex-1 overflow-hidden">
          {show === "overview" && <Overview />}
          {show === "session" && <SessionPage />}
          {show === "boats" && <Boats />}
          {show === "devices" && <Devices />}
          {show === "sniffer" && <Sniffer />}
          {show === "switching" && <Switching />}
          {show === "settings" && <Settings />}
          {show === "connect" && <Connect />}
        </main>
      </div>
      <Toasts />
      <VesselDialog />
      <ResumeDialog />
      <EndSessionDialog />
      <ReportViewer />
    </div>
  );
}

function Nav({ current }: { current: Page }) {
  const { setPage, theme, setTheme } = useApp();
  const sess = useSession((x) => x.status?.state);
  const item = (id: Page, label: string, Icon: typeof Home, key?: string, wide?: boolean) => {
    const on = current === id || (id === "settings" && current === "connect");
    const rec = id === "session" && sess === "recording";
    return (
      <button
        key={id}
        onClick={() => setPage(id)}
        aria-current={on ? "page" : undefined}
        title={key ? `${label} (${key})` : label}
        className={`${wide ? "hidden sm:flex" : "flex"} flex-1 flex-col items-center gap-1 rounded-xl py-2.5 text-[11.5px] font-medium sm:flex-none ${on ? (id === "session" ? "bg-port/10 text-port" : "bg-sea/10 text-sea") : "text-muted hover:bg-shoal hover:text-ink"}`}
      >
        {id === "session" ? (
          <span className="relative flex h-5 w-5 items-center justify-center" aria-hidden>
            <span className={`h-3.5 w-3.5 rounded-full ${rec ? "rec-dot bg-port" : sess === "paused" ? "border-2 border-caution" : "border-2 border-current"}`} />
          </span>
        ) : (
          <Icon size={20} strokeWidth={on ? 2.1 : 1.8} />
        )}
        {label}
      </button>
    );
  };
  return (
    <nav aria-label="Main" className="safe-bottom order-last flex shrink-0 gap-1 border-t hairline bg-panel px-2 pt-1 sm:order-none sm:w-[88px] sm:flex-col sm:border-r sm:border-t-0 sm:px-2.5 sm:pt-4">
      <div className="mb-3 hidden h-10 items-center justify-center sm:flex" title="Keelwire">
        <svg width="32" height="32" viewBox="100 100 824 824" role="img" aria-label="Keelwire">
          <rect x="100" y="100" width="824" height="824" rx="186" fill="rgb(var(--ink))" />
          <path d="M392 262V762" stroke="rgb(var(--paper))" strokeWidth="66" />
          <path d="M322 262H462M322 762H462" stroke="rgb(var(--paper))" strokeWidth="34" strokeLinecap="round" />
          <path d="M400 512L612 330M400 512L612 694" stroke="rgb(var(--paper))" strokeWidth="50" strokeLinecap="round" />
          <circle cx="650" cy="300" r="56" fill="rgb(var(--sea))" />
          <circle cx="650" cy="724" r="56" fill="rgb(var(--sea))" />
        </svg>
      </div>
      {NAV.map((n) => item(n.id, n.label, n.icon, n.key, n.wide))}
      <div className="hidden flex-1 sm:block" />
      <div className="hidden sm:block">
        <ThemeSwitch theme={theme} setTheme={setTheme} />
      </div>
      {item("settings", "Settings", Cog)}
    </nav>
  );
}

function ThemeSwitch({ theme, setTheme }: { theme: Theme; setTheme: (t: Theme) => void }) {
  const opts: [Theme, typeof Sun, string][] = [
    ["day", Sun, "Day"],
    ["dark", Moon, "Dark"],
    ["night", MoonStar, "Night (red)"],
  ];
  return (
    <div className="mb-2 flex flex-col gap-0.5 rounded-xl border hairline p-1" role="radiogroup" aria-label="Display mode">
      {opts.map(([t, Icon, label]) => (
        <button key={t} role="radio" aria-checked={theme === t} title={label} aria-label={label} onClick={() => setTheme(t)} className={`flex justify-center rounded-lg py-1.5 ${theme === t ? "bg-shoal text-ink" : "text-muted hover:text-ink"}`}>
          <Icon size={16} />
        </button>
      ))}
    </div>
  );
}

function TopBar() {
  const { status, connected, setPage, metrics } = useApp();
  const st = status?.interface_status?.state;
  const chip = !connected
    ? { cls: "bg-port/10 text-port", text: "Server unreachable" }
    : status?.offline
      ? { cls: "bg-shoal text-muted", text: "Offline" }
      : st === "connected"
        ? { cls: "bg-stbd/10 text-stbd", text: "Connected" }
        : st === "connecting"
          ? { cls: "bg-caution/10 text-caution", text: "Connecting" }
          : { cls: "bg-port/10 text-port", text: "Not connected" };
  return (
    <header className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b hairline bg-panel px-4 py-2.5 sm:px-7">
      <VesselBadge />
      <button className="flex min-w-0 items-center gap-3 rounded-lg text-left hover:opacity-80" onClick={() => setPage("connect")} title="Change connection">
        <span className={`chip ${chip.cls}`}>
          <span className="h-2 w-2 rounded-full bg-current" aria-hidden />
          {chip.text}
        </span>
        <span className="hidden max-w-[240px] truncate font-semibold xl:inline">{status?.offline ? status.offline : (status?.interface?.name ?? "No gateway")}</span>
        {status?.interface?.uri && <span className="hidden truncate font-mono text-[13px] text-muted 2xl:inline">{status.interface.uri.replace(/^[a-z0-9]+:/, "")}</span>}
      </button>
      <GlobalSearch />
      <span className="flex-1" />
      {status?.capture && (
        <button className="chip bg-port/10 text-port" onClick={() => setPage("sniffer")}>
          <span className="h-2 w-2 animate-pulse rounded-full bg-current" aria-hidden />
          Capture running
        </button>
      )}
      {(metrics?.errors_per_sec ?? 0) > 0 && <span className="chip bg-port/10 text-port">{metrics?.errors_per_sec} errors/s</span>}
      <LockButton />
      <RecButton />
    </header>
  );
}

function GlobalSearch() {
  const { devices, selectDevice, setPage } = useApp();
  const [q, setQ] = useState("");
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLInputElement>(null);
  const hits = useMemo(() => {
    const t = q.trim().toLowerCase();
    if (!t) return [];
    return devices
      .filter((d) => `${deviceLabel(d)} ${d.manufacturer ?? ""} ${d.product?.serial_code ?? ""} ${d.address ?? ""} ${d.device_function_text ?? ""} ${d.config?.installation_description1 ?? ""}`.toLowerCase().includes(t))
      .slice(0, 8);
  }, [q, devices]);
  const pgn = /^\d{5,6}$/.test(q.trim()) ? Number(q.trim()) : null;
  const go = (k: string) => {
    selectDevice(k);
    setPage("devices");
    setQ("");
    setOpen(false);
  };
  return (
    <div className="relative order-last w-full sm:order-none sm:ml-1 sm:w-auto sm:flex-[1_1_200px] sm:max-w-sm">
      <label htmlFor="global-search" className="sr-only">
        Search devices and messages
      </label>
      <Search size={16} className="pointer-events-none absolute left-3 top-2.5 text-muted" aria-hidden />
      <input
        id="global-search"
        ref={ref}
        className="input w-full !bg-shoal pl-9 pr-8"
        placeholder="Search devices, PGNs"
        value={q}
        autoComplete="off"
        onChange={(e) => {
          setQ(e.target.value);
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && hits[0]) go(hits[0].key);
          if (e.key === "Enter" && !hits[0] && pgn) {
            useApp.getState().setFilter(`pgn == ${pgn}`);
            setPage("sniffer");
            setQ("");
          }
          if (e.key === "Escape") {
            setQ("");
            ref.current?.blur();
          }
        }}
      />
      {!q && <kbd className="pointer-events-none absolute right-2.5 top-2 rounded border hairline px-1.5 font-mono text-[11px] text-muted">/</kbd>}
      {open && (hits.length > 0 || pgn) && (
        <ul className="absolute left-0 right-0 top-11 z-30 overflow-hidden rounded-xl border hairline bg-panel py-1 shadow-xl" role="listbox">
          {hits.map((d) => (
            <li key={d.key}>
              <button className="flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-shoal" onMouseDown={() => go(d.key)}>
                <span className={`h-2 w-2 rounded-full ${d.online ? "bg-stbd" : "bg-muted"}`} aria-hidden />
                <span className="font-semibold">{deviceLabel(d)}</span>
                <span className="truncate text-muted">{d.device_function_text}</span>
                <span className="ml-auto font-mono text-xs text-muted">{d.address}</span>
              </button>
            </li>
          ))}
          {pgn && (
            <li>
              <button
                className="flex w-full items-center gap-3 px-3 py-2 text-left hover:bg-shoal"
                onMouseDown={() => {
                  useApp.getState().setFilter(`pgn == ${pgn}`);
                  setPage("sniffer");
                  setQ("");
                }}
              >
                <Activity size={14} /> Show traffic for PGN <span className="font-mono">{pgn}</span>
              </button>
            </li>
          )}
        </ul>
      )}
    </div>
  );
}

/** Shown while changes are unlocked, with a live countdown. */
function UnlockBanner() {
  const status = useApp((s) => s.status);
  const unlock = useUnlock();
  const [left, setLeft] = useState<number | null>(null);
  const base = useRef<{ at: number; s: number } | null>(null);
  useEffect(() => {
    if (status?.armed && status.armed_remaining_s != null) base.current = { at: Date.now(), s: status.armed_remaining_s };
    else base.current = null;
  }, [status?.armed, status?.armed_remaining_s]);
  useEffect(() => {
    const t = setInterval(() => setLeft(base.current ? Math.max(0, base.current.s - Math.floor((Date.now() - base.current.at) / 1000)) : null), 1000);
    return () => clearInterval(t);
  }, []);
  if (!status?.armed) return null;
  const l = left ?? status.armed_remaining_s ?? 0;
  return (
    <div role="status" className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b hairline bg-caution/10 px-4 py-2 text-[13px] text-caution sm:px-7">
      <LockOpen size={15} />
      <strong className="font-semibold">Changes unlocked</strong>
      <span className="hidden md:inline">Settings you apply and switches you press are sent to the devices. Locks again after 15 minutes without changes.</span>
      <span className="flex-1" />
      <span className="font-mono">
        {Math.floor(l / 60)}:{String(l % 60).padStart(2, "0")} left
      </span>
      <button className="btn btn-sm" onClick={() => unlock(false)}>
        Lock now
      </button>
    </div>
  );
}

function Toasts() {
  const { toasts, dismiss } = useApp();
  return (
    <div className="pointer-events-none fixed bottom-20 right-4 z-50 flex w-[380px] max-w-[calc(100vw-2rem)] flex-col gap-2 sm:bottom-4" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`pointer-events-auto flex items-start gap-2 rounded-xl border bg-panel px-3.5 py-3 text-sm shadow-xl ${t.kind === "error" ? "border-port/50" : t.kind === "ok" ? "border-stbd/50" : "hairline"}`}>
          <span className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${t.kind === "error" ? "bg-port" : t.kind === "ok" ? "bg-stbd" : "bg-sea"}`} aria-hidden />
          <span className="flex-1">{t.text}</span>
          <button onClick={() => dismiss(t.id)} aria-label="Dismiss" className="text-muted hover:text-ink">
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
