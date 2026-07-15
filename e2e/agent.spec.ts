import { expect, test } from "@playwright/test";
import { resetSynthesisFixtures } from "./fixtures/data";

test.beforeEach(() => {
  resetSynthesisFixtures();
});

test("shows synthesis progress and launches agents", async ({ page }) => {
  await page.goto("/agent");

  await expect(page.getByRole("heading", { name: "Dialogue Review" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Review queue and decisions" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "AI-assisted review" })).toBeVisible();
  await expect(page.getByText(/Agents cannot render, audition, or accept/)).toBeVisible();
  await expect(page.getByText("120", { exact: true })).toBeVisible();
  await expect(page.getByText("2", { exact: true }).first()).toBeVisible();

  // Human queue appears before the AI card.
  const queueBox = await page.getByRole("heading", { name: "Review queue and decisions" }).boundingBox();
  const aiBox = await page.getByRole("heading", { name: "AI-assisted review" }).boundingBox();
  expect(queueBox && aiBox && queueBox.y < aiBox.y).toBeTruthy();

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

  const flaggedBefore = page.getByRole("tab", { name: /Flagged/ }).locator(".tab-count");
  await expect(flaggedBefore).toHaveText("5");

  const row = page.locator(".decision-row").filter({ hasText: "*hic* Excuse me." });
  await expect(row).toBeVisible();
  await row.getByRole("button", { name: "Accept current text" }).click();
  await expect(page.getByText("*hic* Excuse me.")).not.toBeVisible();
  await expect(page.getByText(/marked reviewed/)).toBeVisible();

  // Tab badges refresh without switching tabs.
  await expect(page.getByRole("tab", { name: /Flagged/ }).locator(".tab-count")).toHaveText("4");
});

test("edits a remaining string and records a generation-only override", async ({ page }) => {
  await page.goto("/agent");
  await page.getByRole("tab", { name: /Remaining/ }).click();

  const row = page.locator(".decision-row").filter({ hasText: "The road is long." });
  await row.getByRole("button", { name: "Edit generation text" }).click();
  await expect(row.getByRole("button", { name: "[sigh]" })).toBeVisible();
  await row.getByLabel("Generation text").fill("different words");
  await row.getByRole("button", { name: "Save override" }).click();
  await expect(row.getByRole("alert")).toContainText("must preserve the spoken words");
  await row.getByLabel("Generation text").fill("The road is long.");
  await row.getByRole("button", { name: "[sigh]" }).click();
  await expect(row.getByLabel("Generation text")).toHaveValue("The road is long.[sigh]");
  await row.getByRole("button", { name: "Save override" }).click();

  await expect(page.getByText(/Override saved/)).toBeVisible();
  await page.getByRole("tab", { name: /Overrides/ }).click();
  await expect(page.getByText("The road is long.[sigh]", { exact: true })).toBeVisible();
});

test("searches flagged queue across the corpus", async ({ page }) => {
  await page.goto("/agent");

  const scream = page.locator(".decision-row").filter({ hasText: "Aaaahhhh!" });
  const hic = page.locator(".decision-row").filter({ hasText: "*hic* Excuse me." });
  await expect(hic).toBeVisible();
  await expect(scream).toBeVisible();

  await page.getByPlaceholder(/Search subtitle or generation text/).fill("Aaaahhhh");
  await expect(hic).not.toBeVisible({ timeout: 5_000 });
  await expect(scream).toBeVisible();

  await page.getByLabel("Flag").selectOption("tts_unfriendly_spelling");
  await expect(scream).toBeVisible();
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

  await expect(page.getByRole("tab", { name: /Suspicious/ }).locator(".tab-count")).toHaveText("1");
  await page.getByRole("tab", { name: /Suspicious/ }).click();
  await expect(page.getByText(/--db C:\\fixture\\bg2vg.db/)).toBeVisible();

  await page.getByRole("button", { name: "Reset all review state" }).click();
  await expect(page.getByText(/Reset complete/)).toBeVisible();
  await expect(page.getByText("120", { exact: true }).first()).toBeVisible();
  await expect(page.getByRole("tab", { name: /Suspicious/ }).locator(".tab-count")).toHaveCount(0);
});

test("progress refresh reloads summary and audit together", async ({ page }) => {
  await page.goto("/agent");
  await expect(page.getByText("flagged undecided", { exact: true })).toBeVisible();
  await page.getByRole("heading", { name: "Review progress" })
    .locator("..")
    .getByRole("button", { name: "Refresh" })
    .click();
  await expect(page.getByText("120", { exact: true })).toBeVisible();
  await expect(page.getByText("flagged undecided", { exact: true })).toBeVisible();
  await expect(page.getByRole("tab", { name: /Suspicious/ }).locator(".tab-count")).toHaveText("1");
});
