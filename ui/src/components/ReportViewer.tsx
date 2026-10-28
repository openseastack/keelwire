// In-app viewer for saved reports (HTML and PDF), used instead of opening a
// new browser tab, which app web views (desktop and iOS) do not do.
import { useEffect, useRef } from "react";
import { create } from "zustand";
import { Share, X } from "lucide-react";
import { historyFileUrl } from "../lib/session";
import { canShare, shareHistory } from "../lib/files";

interface ViewerState {
  path: string | null;
  open: (path: string) => void;
  close: () => void;
}

export const useViewer = create<ViewerState>((set) => ({
  path: null,
  open: (path) => set({ path }),
  close: () => set({ path: null }),
}));

/** Open a saved report: HTML always in the viewer; PDF in the viewer except on
 *  iPhone/iPad, where the share sheet (Quick Look, Mail, Files) is better. */
export function openReport(path: string) {
  if (canShare() && path.toLowerCase().endsWith(".pdf")) {
    shareHistory([path]);
    return;
  }
  useViewer.getState().open(path);
}

export function ReportViewer() {
  const { path, close } = useViewer();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!path) return;
    ref.current?.focus();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    document.addEventListener("keydown", esc);
    return () => document.removeEventListener("keydown", esc);
  }, [path, close]);
  if (!path) return null;
  const name = path.split("/").pop() ?? "Report";
  return (
    <div className="fixed inset-0 z-[60] flex flex-col bg-ink/50 p-0 sm:p-6" style={{ paddingTop: "max(env(safe-area-inset-top), 0px)" }}>
      <div ref={ref} tabIndex={-1} role="dialog" aria-modal="true" aria-label={name} style={{ outline: "none" }} className="mx-auto flex h-full w-full max-w-5xl flex-col overflow-hidden bg-panel shadow-2xl sm:rounded-2xl">
        <header className="flex items-center gap-2 border-b hairline px-4 py-2.5">
          <h2 className="min-w-0 flex-1 truncate font-semibold">{name}</h2>
          <button className="btn btn-sm" onClick={() => shareHistory([path])}>
            <Share size={14} /> {canShare() ? "Share" : "Download"}
          </button>
          <button className="btn btn-sm" aria-label="Close" onClick={close}>
            <X size={15} />
          </button>
        </header>
        <iframe title={name} src={historyFileUrl(path, true)} className="min-h-0 w-full flex-1 bg-white" />
      </div>
    </div>
  );
}
