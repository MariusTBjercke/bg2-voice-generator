import { expect, test } from "@playwright/test";
import { attributionCounts, blockedLines } from "./fixtures/data";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Attribution screen", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Attribution");
    await expect(page.getByRole("heading", { name: "Attribution", level: 2 })).toBeVisible();
  });

  test("hydrates attribution counts from the mock backend", async ({ page }) => {
    const stat = (label: string) =>
      page.locator(".stat").filter({ has: page.getByText(label, { exact: true }) });
    await expect(stat("Speakers").locator(".value")).toHaveText(
      String(attributionCounts.speakers),
    );
    await expect(stat("Ready").locator(".value")).toHaveText(String(attributionCounts.ready_lines));
    await expect(stat("Blocked").locator(".value")).toHaveText(
      String(attributionCounts.blocked_lines),
    );
    await expect(stat("Non-spoken").locator(".value")).toHaveText(
      String(attributionCounts.skipped_lines),
    );
    await expect(page.getByRole("note")).toContainText(
      "Re-scan merges new lines and keeps harvest, bindings, pools, and completed generations",
    );
    await expect(stat("Companion banter lines").locator(".value")).toHaveText(
      String(attributionCounts.companion_lines_added),
    );
    await expect(stat("Companion side DLGs").locator(".value")).toHaveText(
      String(attributionCounts.companion_side_dlgs_scanned),
    );
    await expect(stat("Side lines").locator(".value")).toHaveText(
      String(attributionCounts.companion_side_lines_added),
    );
  });

  test("lists blocked lines with derived reasons", async ({ page }) => {
    await expect(
      page.getByRole("heading", { name: `Blocked lines (${blockedLines.length})` }),
    ).toBeVisible();
    await expect(page.getByRole("cell", { name: "Xzar", exact: true }).first()).toBeVisible();
    await expect(page.getByRole("cell", { name: "already voiced", exact: true })).toBeVisible();
    await expect(page.getByRole("cell", { name: "dynamic token", exact: true })).toBeVisible();
    await expect(page.getByRole("cell", { name: "unattributed", exact: true })).toBeVisible();
  });

  test("blocked reason facet can isolate official VO", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Blocked lines (3)" })).toBeVisible();
    const reasonSelect = page.locator('label.field:has-text("Blocked reason") select');
    await reasonSelect.selectOption("already voiced");
    await expect(page.getByRole("cell", { name: "44001" })).toBeVisible();
    await expect(page.getByRole("cell", { name: "44002" })).not.toBeVisible();
  });
});
