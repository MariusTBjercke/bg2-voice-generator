/** E2E stub for `@tauri-apps/plugin-dialog` (no Tauri runtime in the browser). */

import { FIXTURE_GAME_DIR } from "../fixtures/data";

export async function open(): Promise<string | string[] | null> {
  return FIXTURE_GAME_DIR;
}

export async function save(): Promise<string | null> {
  return "C:\\fixture\\bg2vg-transfer.zip";
}

/** Mirrors `@tauri-apps/plugin-dialog` confirm; defaults to OK for E2E flows. */
export async function confirm(
  _message: string,
  _options?: { title?: string; kind?: string; okLabel?: string; cancelLabel?: string },
): Promise<boolean> {
  return true;
}
