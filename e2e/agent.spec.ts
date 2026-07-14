import { expect, test } from "@playwright/test";
import { resetSynthesisFixtures } from "./fixtures/data";

test.beforeEach(() => {
  resetSynthesisFixtures();
});

test("shows synthesis progress and launches agents", async ({ page }) => {
  await page.goto("/agent");

  await expect(page.getByRole("heading", { name: "Dialogue Review" })).toBeVisible();
  await expect(page.getByText(/bounded named pacing presets through bg2-synthesis/)).toBeVisible();
  await expect(page.getByText(/cannot render, audition, or accept candidate audio/)).toBeVisible();
  await expect(page.getByText("120", { exact: true })).toBeVisible();
  await expect(page.getByText("2", { exact: true }).first()).toBeVisible();

  await page.getByRole("button", { name: "Launch Codex" }).click();
  await expect(page.getByRole("button", { name: "Launch Codex" })).toBeEnabled();

  await page.getByRole("button", { name: "Reveal workspace" }).click();
  await expect(page.getByRole("button", { name: "Reveal workspace" })).toBeEnabled();
});

test("lists processed decisions and clears an override", async ({ page }) => {
  await page.goto("/agent");

  await expect(page.getByRole("heading", { name: "Review queue and decisions" })).toBeVisible();
  await page.getByRole("tab", { name: /Overrides/ }).click();
  await expect(page.getByText("Please leave me alone *sigh*")).toBeVisible();

  await page.getByRole("button", { name: "Clear override" }).click();
  await expect(page.getByText("Please leave me alone *sigh*")).not.toBeVisible();
  await expect(page.getByText(/Cleared override/)).toBeVisible();
});

test("switches reviewed tab and unmarks a review", async ({ page }) => {
  await page.goto("/agent");

  await page.getByRole("tab", { name: /Reviewed/ }).click();
  await expect(page.getByText("A fine day for murder.").first()).toBeVisible();

  await page.getByRole("button", { name: "Unmark review" }).click();
  await expect(page.getByText("A fine day for murder.")).toHaveCount(0);
});

test("accepts a flagged string without an override", async ({ page }) => {
  await page.goto("/agent");

  const row = page.locator(".decision-row").filter({ hasText: "*hic* Excuse me." });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Accept current text" }).click();
  await expect(page.getByText("*hic* Excuse me.")).not.toBeVisible();
  await expect(page.getByText(/marked reviewed/)).toBeVisible();
});

test("edits a remaining string and records a generation-only override", async ({ page }) => {
  await page.goto("/agent");
  await page.getByRole("tab", { name: /Remaining/ }).click();

  const row = page.locator(".decision-row").filter({ hasText: "<losing battle>" });
  await row.getByRole("button", { name: "Edit generation text" }).click();
  await row.getByLabel("Generation text").fill("different words");
  await row.getByRole("button", { name: "Save override" }).click();
  await expect(row.getByRole("alert")).toContainText("must preserve the spoken words");
  await row.getByLabel("Generation text").fill("losing battle");
  await row.getByRole("button", { name: "Save override" }).click();

  await expect(page.getByText(/Override saved/)).toBeVisible();
  await page.getByRole("tab", { name: /Overrides/ }).click();
  await expect(page.getByText("losing battle", { exact: true })).toBeVisible();
});

test("shows flagged corpus audit tab and auto-review action", async ({ page }) => {
  await page.goto("/agent");

  await expect(page.getByRole("heading", { name: "Corpus audit" })).toBeVisible();
  await expect(page.getByText("flagged undecided", { exact: true })).toBeVisible();

  await page.getByRole("tab", { name: /Flagged/ }).click();
  await expect(page.getByText("*hic* Excuse me.")).toBeVisible();

  page.on("dialog", (dialog) => dialog.accept());
  await page.getByRole("button", { name: "Auto-review plain lines" }).click();
  await expect(page.getByText(/Auto-reviewed 100 plain line/)).toBeVisible();
});

test("shows suspicious overrides and resets all agent state", async ({ page }) => {
  page.on("dialog", (dialog) => dialog.accept());

  await page.goto("/agent");

  await page.getByRole("tab", { name: /Suspicious/ }).click();
  await expect(page.getByText(/--db C:\\fixture\\bg2vg.db/)).toBeVisible();

  await page.getByRole("button", { name: "Reset all review state" }).click();
  await expect(page.getByText(/Reset complete/)).toBeVisible();
  await expect(page.getByText("120", { exact: true }).first()).toBeVisible();
});
