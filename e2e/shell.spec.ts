import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo, pipelineScreens } from "./helpers/bootstrap";

test.describe("App shell", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
  });

  test("footer reports a healthy backend", async ({ page }) => {
    await expect(page.getByText(/Backend v0\.1\.0-e2e/)).toBeVisible();
    await expect(page.getByText(/schema 2/)).toBeVisible();
  });

  test("pipeline nav reaches every screen", async ({ page }) => {
    for (const screen of pipelineScreens) {
      await goTo(page, screen.label);
      await expect(page.getByRole("heading", { name: screen.title, level: 2 })).toBeVisible();
      await expect(
        page.getByRole("navigation").getByRole("link", { name: screen.label }),
      ).toHaveClass(/active/);
    }
  });

  test("top bar matches snapshot", async ({ page }) => {
    await expect(page.locator("header.topbar")).toHaveScreenshot("shell-topbar.png");
  });
});
