import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Guided voice binding", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Binding");
  });

  test("shows demographic defaults and optional speaker overrides together", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Demographic defaults" })).toBeVisible();
    await expect(page.getByRole("heading", { name: /Speaker overrides/ })).toBeVisible();
    await expect(page.getByText("Male / Human / Humanoid")).toBeVisible();
    await expect(page.getByLabel("Effective voice readiness")).toContainText("1 demographic defaults");
    await expect(page.getByRole("button", { name: "Apply defaults" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Auto-configure all" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Use personal samples for all" })).toBeVisible();
  });

  test("can expand a group pool editor with suggest and play", async ({ page }) => {
    await page.getByText("Male / Human / Humanoid").click();
    await expect(page.getByRole("button", { name: "Suggest best donor" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Clear pool", exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Add from other demographics…" }).click();
    await expect(page.locator("select.donor-select").nth(1)).toContainText("Montaron");
  });

  test("clear all pools requires confirmation", async ({ page }) => {
    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Clear pools", exact: true }).click();
    await expect(page.getByText("Cleared 1 pool(s).")).toBeVisible();
  });

  test("character rows expose inherited donor and effective-voice preview", async ({ page }) => {
    await expect(page.getByText("Demographic default", { exact: true }).first()).toBeVisible();
    await expect(page.getByText("Voice: Xzar").first()).toBeVisible();
    await expect(page.getByRole("button", { name: "Play effective voice for Xzar" })).toBeVisible();
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await expect(page.getByRole("strong").filter({ hasText: "Effective voice" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Play effective voice", exact: true })).toBeVisible();
  });

  test("edits compact and advanced voice tuning without saving immediately", async ({ page }) => {
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await expect(page.getByRole("heading", { name: "Voice tuning" })).toBeVisible();

    const speed = page.getByRole("group", { name: "Speaking speed" });
    await expect(speed.getByLabel("Automatic model pacing")).toBeChecked();
    await speed.getByLabel("Fixed speed").check();
    await speed.getByLabel("Multiplier").fill("1.15");

    const steps = page.getByRole("group", { name: "Diffusion steps" });
    await steps.getByLabel("Steps").fill("64");
    await expect(steps).toContainText("2.00× the default render time");
    await page.getByText("Advanced controls", { exact: true }).click();
    await expect(page.getByLabel("Guidance scale")).toHaveValue("2");
    await expect(page.getByText("unsaved", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Reset to defaults" }).click();
    await expect(speed.getByLabel("Automatic model pacing")).toBeChecked();
    await expect(steps.getByLabel("Steps")).toHaveValue("32");
    await expect(page.locator(".tuning-panel")).toHaveScreenshot("binding-voice-tuning.png", {
      animations: "disabled",
    });
  });

  test("renders A/B previews and saves an explicitly chosen composite", async ({ page }) => {
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await expect(page.getByRole("heading", { name: "Voice tuning" })).toBeVisible();

    const cards = page.locator(".preview-card");
    const previewA = cards.filter({ has: page.getByRole("heading", { name: "Preview A" }) });
    const previewB = cards.filter({ has: page.getByRole("heading", { name: "Preview B" }) });
    await expect(cards.locator("audio")).toHaveCount(0);

    await previewA.getByRole("button", { name: "Render A" }).click();
    await expect(previewA.getByRole("button", { name: "Rendering A…" })).toBeVisible();
    await expect(previewA.locator("audio")).toBeVisible();
    await expect(previewA).toContainText("Single clip");

    await previewB.getByRole("button", { name: "Render B" }).click();
    await expect(previewB.getByRole("button", { name: "Rendering B…" })).toBeVisible();
    await expect(previewB.locator("audio")).toBeVisible();
    await expect(previewB).toContainText("2-clip composite");
    await previewB.getByRole("button", { name: "Use this reference" }).click();
    await expect(page.getByText(/Saved 2-clip composite reference/)).toBeVisible();
  });

  test("surfaces preview errors and saves settings with scoped invalidation feedback", async ({ page }) => {
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await expect(page.getByRole("heading", { name: "Voice tuning" })).toBeVisible();

    const speed = page.getByRole("group", { name: "Speaking speed" });
    await speed.getByLabel("Fixed speed").check();
    await speed.getByLabel("Multiplier").fill("1.15");
    await page.getByRole("button", { name: "Save tuning" }).click();
    await expect(page.getByText(/Saved tuning\. Reset 2 generated clip/)).toBeVisible();

    await page.getByLabel("Preview dialogue").fill("[preview error]");
    await page.getByRole("button", { name: "Render A" }).click();
    await expect(page.getByText("Fixture preview failed")).toBeVisible();
  });

  test("keeps the demographic page after an in-group action refreshes data", async ({ page }) => {
    const groupsPanel = page.locator("#demographic-groups-panel");
    await groupsPanel.getByRole("button", { name: "Next page" }).click();
    await expect(groupsPanel.getByText("2 / 2", { exact: true })).toBeVisible();

    const group = groupsPanel.locator("li.group-row").filter({ hasText: "Fixture sex 30" });
    await group.getByRole("button").first().click();
    await group.locator("select.donor-select").first().selectOption("1");
    await group.getByRole("button", { name: "Add", exact: true }).click();

    await expect(groupsPanel.getByText("2 / 2", { exact: true })).toBeVisible();
    await expect(group).toBeVisible();
  });

  test("can collapse demographic groups and character lists", async ({ page }) => {
    const groupsToggle = page.getByRole("button", { name: /Demographic groups/ });
    await groupsToggle.click();
    await expect(page.locator("#demographic-groups-panel")).toBeHidden();
    await expect(groupsToggle).toHaveAttribute("aria-expanded", "false");

    const charactersToggle = page.getByRole("button", { name: /Characters \(2\)/ });
    await charactersToggle.click();
    await expect(page.locator("#characters-list-panel")).toBeHidden();
    await expect(charactersToggle).toHaveAttribute("aria-expanded", "false");
  });

  test("keeps the desktop speaker column bounded with vertical-only scrolling", async ({ page }) => {
    const layout = page.locator(".layout");
    await expect(layout.getByRole("heading", { name: "Characters (2)" })).toBeVisible();
    const speakerCard = layout.locator(":scope > .card").first();
    const styles = await speakerCard.evaluate((element) => {
      const computed = getComputedStyle(element);
      return {
        position: computed.position,
        overflowX: computed.overflowX,
        overflowY: computed.overflowY,
        scrollbarGutter: computed.scrollbarGutter,
      };
    });
    expect(styles.position).toBe("sticky");
    expect(styles.overflowX).toBe("hidden");
    expect(styles.overflowY).toBe("auto");
    expect(styles.scrollbarGutter).toContain("stable");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
    await expect(layout).toHaveScreenshot("binding-layout-desktop.png", { animations: "disabled" });
  });

  test("stacks binding cards and removes sticky scrolling at narrow widths", async ({ page }) => {
    await page.setViewportSize({ width: 700, height: 800 });
    const cards = page.locator(".layout > .card");
    const first = await cards.nth(0).boundingBox();
    const second = await cards.nth(1).boundingBox();
    expect(first).not.toBeNull();
    expect(second).not.toBeNull();
    expect(second!.y).toBeGreaterThanOrEqual(first!.y + first!.height);

    const styles = await cards.first().evaluate((element) => {
      const computed = getComputedStyle(element);
      return { position: computed.position, maxHeight: computed.maxHeight, overflowY: computed.overflowY };
    });
    expect(styles.position).toBe("static");
    expect(styles.maxHeight).toBe("none");
    expect(styles.overflowY).toBe("visible");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
    await expect(page.locator(".layout")).toHaveScreenshot("binding-layout-narrow.png", { animations: "disabled" });
  });
});
