/** Answer of the Rust check_update_safety command (farm GET /system/update-safety). */
export interface SafetyReason {
  code: string;
  text: string;
}

export interface UpdateSafety {
  safe: boolean;
  /** false = the farm didn't answer (old farm / not running) — treated as safe. */
  known: boolean;
  reasons: SafetyReason[];
}

/** Set right before Flux relaunches into a new version: the next launch updates the farm too,
 *  even when the farm is still running (otherwise a running farm skips the startup pull). */
const UPDATE_FARM_ON_LAUNCH = "flux.updateFarmOnLaunch";

export function markUpdateFarmOnLaunch() {
  try { localStorage.setItem(UPDATE_FARM_ON_LAUNCH, String(Date.now())); } catch {}
}

/** Reads and clears the flag. */
export function takeUpdateFarmOnLaunch(): boolean {
  try {
    const v = localStorage.getItem(UPDATE_FARM_ON_LAUNCH);
    localStorage.removeItem(UPDATE_FARM_ON_LAUNCH);
    return v !== null;
  } catch {
    return false;
  }
}

export function reasonsText(reasons: SafetyReason[]): string {
  const texts = reasons.map((r) => r.text).filter(Boolean);
  return texts.length ? texts.join(", ") : "work in progress";
}
