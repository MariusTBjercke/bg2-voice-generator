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
 * Append an opaque cache-bust token to a URL. Exported for unit tests.
 */
export function appendCacheBust(url: string, cacheBust?: string | number): string {
  if (cacheBust === undefined || cacheBust === "") return url;
  const joiner = url.includes("?") ? "&" : "?";
  return `${url}${joiner}v=${encodeURIComponent(String(cacheBust))}`;
}

/**
 * Convert an absolute filesystem path into an asset-protocol URL the webview can
 * load (e.g. an `<audio src>`). This is NOT a raw FS read: the path is served by
 * Tauri's asset protocol, gated by the `assetProtocol.scope` allow-list in
 * `tauri.conf.json` (see docs/adr/0003-repo-module-layout.md).
 *
 * Pass `cacheBust` when the same path may be overwritten in-place (e.g. after a
 * forced Re-generate). The webview caches asset URLs aggressively, so without a
 * changing query token Play keeps serving the previous bytes even though the
 * file on disk already changed.
 */
export function assetUrl(path: string, cacheBust?: string | number): string {
  return appendCacheBust(convertFileSrc(path), cacheBust);
}

/** Subscribe to a backend event; returns the unlisten fn. */
export function listen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  return tauriListen<T>(event, (e) => handler(e.payload as T));
}
