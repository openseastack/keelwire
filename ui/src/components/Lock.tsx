import { Lock, LockOpen } from "lucide-react";
import { post } from "../lib/api";
import { useApp } from "../lib/store";

/** Arm / disarm in place. Explains why when the server doesn't allow it. */
export function useUnlock() {
  const { status, toast } = useApp();
  return async (on: boolean) => {
    if (on && status?.tx_mode === "passive") return toast("info", "Listen only is on. Turn it off in Settings to make changes.");
    if (on && !status?.allow_tx) return toast("info", "This server was started without --allow-tx, so it can only read. Restart it with --allow-tx to make changes.");
    try {
      await post("/tx", { armed: on });
    } catch (e) {
      toast("error", (e as Error).message);
    }
  };
}

export function LockButton({ label = "Changes" }: { label?: string }) {
  const armed = useApp((s) => !!s.status?.armed);
  const unlock = useUnlock();
  return (
    <button className={`btn ${armed ? "!border-caution !text-caution" : ""}`} onClick={() => unlock(!armed)} aria-pressed={armed}>
      {armed ? <LockOpen size={16} /> : <Lock size={16} />}
      <span className="hidden sm:inline">{armed ? `${label} unlocked` : `${label} locked`}</span>
      <span className="sr-only sm:hidden">{armed ? `${label} unlocked` : `${label} locked`}</span>
    </button>
  );
}

