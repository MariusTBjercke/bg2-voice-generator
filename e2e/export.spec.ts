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
});
