import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";
import { generatableLines } from "./fixtures/data";

test("warns that voice-changed clips remain included in exports", async ({ page }) => {
  await bootstrapProject(page);
  const staleIds = generatableLines.slice(0, 2).map((line) => line.id);
  await page.evaluate((ids) => {
    localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(ids));
    localStorage.setItem("e2e.voice-changed-generation-ids", JSON.stringify(ids));
  }, staleIds);

  await goTo(page, "Export");
  const warning = page.getByRole("status");
  await expect(warning).toContainText("2 generated clips use an earlier speaker binding");
  await expect(warning).toContainText("will still be included in this export");
  await expect(warning.getByRole("link", { name: "Generation" })).toHaveAttribute(
    "href",
    "/generation?focus=voice_changed",
  );
  await expect(warning.getByRole("link", { name: "blocked/skipped" })).toHaveAttribute(
    "href",
    "/generation?focus=orphans",
  );
});

test("export deep-link focuses voice-changed clips on Generation", async ({ page }) => {
  await bootstrapProject(page);
  const staleIds = generatableLines.slice(0, 2).map((line) => line.id);
  await page.evaluate((ids) => {
    localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(ids));
    localStorage.setItem("e2e.voice-changed-generation-ids", JSON.stringify(ids));
  }, staleIds);

  await goTo(page, "Export");
  await page.getByRole("status").getByRole("link", { name: "Generation" }).click();
  await expect(page.getByRole("heading", { name: "Generation", level: 2 })).toBeVisible();
  await expect(page.getByRole("heading", { name: /Generatable lines \(2 of 104\)/ })).toBeVisible();
  await expect(page.getByText("voice changed", { exact: true })).toHaveCount(2);
  await expect(page).toHaveURL(/\/generation$/);
});

test("restores the pack name after reload", async ({ page }) => {
  await bootstrapProject(page);
  await goTo(page, "Export");
  await page.getByLabel("Pack name").fill("My Persistent Voice Pack");
  await page.reload();
  await expect(page.getByLabel("Pack name")).toHaveValue("My Persistent Voice Pack");
});
