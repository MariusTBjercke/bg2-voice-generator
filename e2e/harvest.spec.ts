import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Harvest manual-only fallback", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Harvest");
  });

  test("filters characters by sex", async ({ page }) => {
    await expect(page.getByLabel("Sex", { exact: true })).toBeVisible();
    await page.getByLabel("Sex", { exact: true }).selectOption("male");
    await expect(page.getByRole("button", { name: /Xzar/ })).toBeVisible();
    await page.getByLabel("Sex", { exact: true }).selectOption("female");
    await expect(page.getByText("No characters match your search.")).toBeVisible();
  });

  test("fills uncovered exact-character voices without changing the safe action", async ({ page }) => {
    await page.getByText("Advanced actions", { exact: true }).click();
    const fallback = page.getByRole("button", { name: "Fill gaps with manual-only" });
    await expect(fallback).toBeVisible();
    await expect(page.getByLabel("Only characters with no approved samples")).toBeChecked();
    await expect(page.getByRole("button", { name: "Auto-approve remaining (automatic)" })).toBeVisible();
    await page.getByLabel("Only characters with no approved samples").uncheck();
    await expect(page.getByRole("button", { name: "Auto-approve best for all characters" })).toBeVisible();
    await fallback.click();
    await expect(page.getByText(/Filled 2 exact-character voice gaps with manual-only samples/)).toBeVisible();
    await expect(page.getByText(/left 3 already-covered or unsafe characters unchanged/)).toBeVisible();
  });

  test("restores the selected character after reload", async ({ page }) => {
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await expect(page.getByRole("heading", { name: /Xzar/ })).toBeVisible();
    await page.reload();
    await expect(page.getByRole("heading", { name: /Xzar/ })).toBeVisible();
    await expect(page.getByText("Select a character to review their harvested samples.")).not.toBeVisible();
  });

  test("identity query selects the character and links to Binding", async ({ page }) => {
    await page.goto("/harvest?identity=22570");
    await expect(page.getByRole("heading", { name: /Xzar/ })).toBeVisible();
    await expect(page.getByRole("link", { name: "Open on Binding" })).toHaveAttribute(
      "href",
      "/binding?identity=22570%3A1",
    );
    await expect(page).toHaveURL(/\/harvest$/);
  });

  test("aligns character filters and rows without horizontal overflow", async ({ page }) => {
    await page.setViewportSize({ width: 960, height: 700 });
    const filter = page.locator(".bar.compact");
    const row = page.locator(".group-row").first();
    const [filterBox, rowBox] = await Promise.all([filter.boundingBox(), row.boundingBox()]);
    expect(filterBox).not.toBeNull();
    expect(rowBox).not.toBeNull();
    expect(Math.abs(filterBox!.x - rowBox!.x)).toBeLessThanOrEqual(1);
    expect(Math.abs(filterBox!.width - rowBox!.width)).toBeLessThanOrEqual(1);

    const viewport = await page.evaluate(() => ({
      innerWidth: window.innerWidth,
      scrollWidth: document.documentElement.scrollWidth,
    }));
    expect(viewport.scrollWidth).toBeLessThanOrEqual(viewport.innerWidth);
  });

  test("uses matching geometry for adjacent Harvest actions", async ({ page }) => {
    const harvest = page.getByRole("button", { name: "Harvest references" });
    const autoApprove = page.getByRole("button", { name: "Auto-approve remaining (automatic)" });
    const advanced = page.getByText("Advanced actions", { exact: true }).locator("..");
    const styles = await Promise.all(
      [harvest, autoApprove, advanced].map((locator) =>
        locator.evaluate((element) => {
          const style = getComputedStyle(element);
          return { height: element.getBoundingClientRect().height, fontSize: style.fontSize };
        }),
      ),
    );
    expect(styles[0].height).toBe(styles[1].height);
    expect(styles[1].height).toBe(styles[2].height);
    expect(styles[0].fontSize).toBe(styles[1].fontSize);
    expect(styles[1].fontSize).toBe(styles[2].fontSize);
  });
});
