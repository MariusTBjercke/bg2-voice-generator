import { expect, type Page } from "@playwright/test";
import { FIXTURE_GAME_DIR } from "../fixtures/data";

/** Visit Setup and wait for the mocked install to hydrate the shared project store. */
export async function bootstrapProject(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "BG2 Voice Generator" })).toBeVisible();
  await expect(page.getByText("Folder selected")).toBeVisible();
  await expect(page.getByText(FIXTURE_GAME_DIR)).toBeVisible();
  await expect(page.getByText("Active language")).toBeVisible();
}

/** Navigate via the pipeline nav after bootstrap. */
export async function goTo(page: Page, label: string): Promise<void> {
  await page.getByRole("navigation").getByRole("link", { name: label }).click();
}

export const pipelineScreens = [
  { label: "Setup", title: "Setup" },
  { label: "Dictionary", title: "Dictionary" },
  { label: "Attribution", title: "Attribution" },
  { label: "Harvest", title: "Harvest" },
  { label: "Binding", title: "Binding" },
  { label: "Generation", title: "Generation" },
  { label: "Export", title: "Export" },
  { label: "Transfer", title: "Transfer" },
] as const;
