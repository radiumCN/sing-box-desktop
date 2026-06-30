import { invoke } from "@tauri-apps/api/core";

/** Thin wrapper around Tauri's `invoke` that logs failures with the command
 *  name before rethrowing, so call sites can keep their own try/catch for UI
 *  state while diagnostics stay consistent. */
export async function invokeCommand<T = void>(
  cmd: string,
  args?: Record<string, unknown>
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    console.error(`[invoke] ${cmd} failed:`, error);
    throw error;
  }
}
