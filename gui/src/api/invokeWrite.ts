import { invoke } from "@tauri-apps/api/core";

/** How a write command landed. At most one of the fields is set: `error`
 * means it failed (or its outcome is unknown — the message says so);
 * `warning` means it succeeded with a caveat the user must see. Both
 * null is a clean success. */
export interface WriteResult {
  error: string | null;
  warning: string | null;
}

export async function invokeWrite(
  command: string,
  args: Record<string, unknown>,
): Promise<WriteResult> {
  try {
    const warning = await invoke<string | null>(command, args);
    return { error: null, warning: warning ?? null };
  } catch (e) {
    return { error: String(e), warning: null };
  }
}
