/** E2E stub for `@tauri-apps/plugin-dialog` (no Tauri runtime in the browser). */

import { FIXTURE_GAME_DIR } from "../fixtures/data";

export async function open(): Promise<string | string[] | null> {
  return FIXTURE_GAME_DIR;
}

export async function save(): Promise<string | null> {
  return "C:\\fixture\\bg2vg-transfer.zip";
}
