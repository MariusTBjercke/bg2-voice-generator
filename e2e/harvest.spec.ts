import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Harvest manual-only fallback", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Harvest");
  });

  test("fills uncovered exact-character voices without changing the safe action", async ({ page }) => {
    const fallback = page.getByRole("button", { name: "Fill gaps with manual-only" });
    await expect(fallback).toBeVisible();
    await expect(page.getByRole("button", { name: "Auto-approve best for all characters" })).toBeVisible();
    await fallback.click();
    await expect(page.getByText(/Filled 2 exact-character voice gaps with manual-only samples/)).toBeVisible();
    await expect(page.getByText(/left 3 already-covered or unsafe characters unchanged/)).toBeVisible();
  });
});
