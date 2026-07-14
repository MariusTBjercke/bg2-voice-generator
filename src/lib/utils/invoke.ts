// Single chokepoint for all backend calls. The frontend NEVER touches the
// filesystem, database, game resources, generation, or export directly -
// everything goes through a Tauri command registered in `src-tauri/src/lib.rs`
// (see docs/adr/0003-repo-module-layout.md).

import { invoke as tauriInvoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";

/** Thin typed wrapper over Tauri's `invoke`. */
export function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(cmd, args);
}

/**
 * Convert an absolute filesystem path into an asset-protocol URL the webview can
 * load (e.g. an `<audio src>`). This is NOT a raw FS read: the path is served by
 * Tauri's asset protocol, gated by the `assetProtocol.scope` allow-list in
 * `tauri.conf.json` (see docs/adr/0003-repo-module-layout.md).
 */
export function assetUrl(path: string): string {
  return convertFileSrc(path);
}

/** Subscribe to a backend event; returns the unlisten fn. */
export function listen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return tauriListen<T>(event, (e) => handler(e.payload as T));
}
