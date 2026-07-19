import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Design system visual coverage", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
  });

  for (const screen of ["Setup", "Harvest", "Generation", "Export"] as const) {
    test(`${screen} keeps the branded hierarchy`, async ({ page }) => {
      if (screen !== "Setup") await goTo(page, screen);
      await expect(page.locator("main").getByRole("heading", { name: screen, level: 2 })).toBeVisible();
      const viewport = await page.evaluate(() => ({
        scrollX: window.scrollX,
        scrollWidth: document.documentElement.scrollWidth,
        innerWidth: window.innerWidth,
      }));
      expect(viewport.scrollX).toBe(0);
      expect(viewport.scrollWidth).toBeLessThanOrEqual(viewport.innerWidth);
      await expect(page).toHaveScreenshot(`design-${screen.toLowerCase()}.png`);
    });
  }
});
