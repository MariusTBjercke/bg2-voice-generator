import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo, pipelineScreens } from "./helpers/bootstrap";

test.describe("App shell", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
  });

  test("footer reports a healthy backend", async ({ page }) => {
    await expect(page.getByText(/Backend v0\.1\.0-e2e/)).toBeVisible();
    await expect(page.getByText(/schema 1/)).toBeVisible();
  });

  test("pipeline nav reaches every screen", async ({ page }) => {
    for (const screen of pipelineScreens) {
      await goTo(page, screen.label);
      await expect(page.getByRole("heading", { name: screen.title, level: 2 })).toBeVisible();
      await expect(
        page.getByRole("navigation", { name: "Workflow" }).getByRole("link", { name: screen.label }),
      ).toHaveClass(/active/);
    }
  });

  test("top bar matches snapshot", async ({ page }) => {
    await expect(page.locator("header.topbar")).toHaveScreenshot("shell-topbar.png");
  });

  test("brand, workflow stages, and profile tools stay clearly separated", async ({ page }) => {
    const header = page.locator("header.topbar");
    await expect(header.locator(".brand img")).toBeVisible();
    await expect(header.locator(".brand img")).toHaveAttribute("src", "/app-icon.png");
    await expect(header.getByRole("navigation", { name: "Workflow" }).getByText("Optional", { exact: true })).toBeVisible();
    await header.getByText("Manage", { exact: true }).click();
    await expect(header.getByRole("button", { name: "New empty profile" })).toBeVisible();
    await expect(header.getByRole("button", { name: "Duplicate profile" })).toBeVisible();
    await expect(header.getByRole("button", { name: "Rename profile" })).toBeVisible();
  });

  test("shell remains usable at the supported minimum width", async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 700 });
    await expect(page.locator("header.topbar")).toHaveScreenshot("shell-topbar-narrow.png");
  });

  test("uses the packaged app icon for branding and the favicon", async ({ page }) => {
    const response = await page.request.get("/app-icon.png");
    expect(response.ok()).toBe(true);
    expect(response.headers()["content-type"]).toContain("image/png");
    await expect(page.locator('link[rel="icon"]')).toHaveAttribute("href", /\/app-icon\.png$/);
  });

  test("switches profiles with the pointer and marks the active profile", async ({ page }) => {
    const trigger = page.getByRole("button", { name: "Active profile" });
    await trigger.click();
    const list = page.getByRole("listbox", { name: "Profiles" });
    await expect(list).toBeVisible();
    await expect(list.getByRole("option", { name: "Default" })).toHaveAttribute("aria-selected", "true");

    await list.getByRole("option", { name: "Campaign archive 03" }).click();
    await expect(list).toBeHidden();
    await expect(trigger).toContainText("Campaign archive 03");

    await trigger.click();
    await expect(list.getByRole("option", { name: "Campaign archive 03" })).toHaveAttribute("aria-selected", "true");
  });

  test("supports keyboard profile selection and restores focus when dismissed", async ({ page }) => {
    const trigger = page.getByRole("button", { name: "Active profile" });
    await trigger.focus();
    await trigger.press("Enter");
    const list = page.getByRole("listbox", { name: "Profiles" });
    await expect(list).toBeVisible();
    await expect(list.getByRole("option").first()).toBeFocused();
    await page.keyboard.press("End");
    await expect(list.getByRole("option").last()).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(trigger).toContainText("Campaign archive 24");
    await expect(trigger).toBeFocused();

    await trigger.press("ArrowDown");
    await expect(list).toBeVisible();
    await page.keyboard.press("Home");
    await page.keyboard.press("Escape");
    await expect(list).toBeHidden();
    await expect(trigger).toBeFocused();

    await trigger.press("Space");
    await expect(list).toBeVisible();
    await page.locator("main").click({ position: { x: 4, y: 4 } });
    await expect(list).toBeHidden();
  });

  test("closes profile tools mutually and bounds long profile lists", async ({ page }) => {
    const trigger = page.getByRole("button", { name: "Active profile" });
    const manage = page.getByText("Manage", { exact: true });
    const list = page.getByRole("listbox", { name: "Profiles" });

    await trigger.click();
    await expect(list).toBeVisible();
    await manage.click();
    await expect(list).toBeHidden();
    await expect(page.getByRole("button", { name: "New empty profile" })).toBeVisible();

    await trigger.click();
    await expect(page.getByRole("button", { name: "New empty profile" })).toBeHidden();
    await expect(list).toBeVisible();
    const dimensions = await list.evaluate((element) => ({
      clientHeight: element.clientHeight,
      scrollHeight: element.scrollHeight,
      overflowY: getComputedStyle(element).overflowY,
    }));
    expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);
    expect(dimensions.overflowY).toBe("auto");

    await page.keyboard.press("Tab");
    await expect(list).toBeHidden();
  });

  for (const width of [1280, 960]) {
    test(`keeps Transfer pinned to the right edge at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 700 });
      const transfer = page.getByRole("navigation", { name: "Workflow" }).getByRole("link", { name: "Transfer" });
      const box = await transfer.boundingBox();
      expect(box).not.toBeNull();
      expect(Math.abs((box!.x + box!.width) - width)).toBeLessThanOrEqual(1);
      await expect(transfer).toBeVisible();
    });
  }

  test("restores the selected locale after a reload", async ({ page }) => {
    const locale = page.getByLabel("Active language");
    await locale.selectOption("de_DE");
    await page.reload();
    await expect(locale).toHaveValue("de_DE");
  });
});
