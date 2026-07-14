import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";
import { generatableLines } from "./fixtures/data";

test.describe("Generation screen", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Generation");
    await expect(page.getByRole("heading", { name: "Generation", level: 2 })).toBeVisible();
  });

  test("lists fixture generatable lines", async ({ page }) => {
    await expect(page.getByRole("heading", { name: /Generatable lines \(104\)/ })).toBeVisible();
    for (const line of generatableLines.slice(0, 3)) {
      await expect(page.getByText(`#${line.strref}`)).toBeVisible();
    }
  });

  test("shows voiced-in-game badge for installed pack audio", async ({ page }) => {
    await expect(page.getByText("voiced in game")).toBeVisible();
    await expect(page.locator('span[title="Z0002A00"]')).toBeVisible();
  });

  test("engine panel offers Start when installed but stopped", async ({ page }) => {
    await expect(page.getByRole("button", { name: "Start engine" })).toBeVisible();
    await expect(page.getByText("Stopped")).toBeVisible();
    await expect(page.getByRole("button", { name: /Generate missing/ })).toBeDisabled();
  });

  test("per-line generate buttons are present", async ({ page }) => {
    const generateButtons = page.locator(".line-action").getByRole("button", {
      name: "Generate",
      exact: true,
    });
    await expect(generateButtons).toHaveCount(100);
  });

  test("shows lazy-loaded synthesis preview under subtitles", async ({ page }) => {
    await expect(page.getByText("Generation text only — subtitle/export unchanged.").first()).toBeVisible();
    await expect(page.getByText("Plain", { exact: true }).first()).toBeVisible();
  });

  test("combines filters, removes chips, and clears the full scope", async ({ page }) => {
    await page.getByRole("button", { name: "More filters" }).click();
    const speakers = page.locator("details").filter({ has: page.locator("summary", { hasText: "Speakers" }) });
    await speakers.locator("summary").click();
    await speakers.getByRole("checkbox", { name: /Xzar/ }).check();

    const packAudio = page.getByRole("group", { name: "Attached pack audio" });
    await packAudio.getByLabel("Present", { exact: true }).check();
    await expect(page.getByRole("heading", { name: "Generatable lines (1 of 104)" })).toBeVisible();
    await expect(page.getByText("#22571")).toBeVisible();

    await page.getByRole("button", { name: "Remove filter Pack audio present" }).click();
    await expect(page.getByRole("heading", { name: "Generatable lines (103 of 104)" })).toBeVisible();

    await page.getByRole("button", { name: "Clear all", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Generatable lines (104)" })).toBeVisible();
  });

  test("batch actions target every filtered line, including lines beyond the current page", async ({ page }) => {
    await page.getByRole("button", { name: "Start engine" }).click();
    await page.getByRole("button", { name: "More filters" }).click();
    const speakers = page.locator("details").filter({ has: page.locator("summary", { hasText: "Speakers" }) });
    await speakers.locator("summary").click();
    await speakers.getByRole("checkbox", { name: /Xzar/ }).check();
    await page.getByRole("group", { name: "Attached pack audio" }).getByLabel("Absent", { exact: true }).check();

    await expect(page.getByRole("button", { name: "Generate missing (102)" })).toBeEnabled();
    await expect(page.locator(".line-action")).toHaveCount(100);
    await page.getByRole("button", { name: "Generate missing (102)" }).click();

    await expect.poll(async () => page.evaluate(() => JSON.parse(localStorage.getItem("e2e.last-generation-batch") ?? "[]") as number[]))
      .toHaveLength(102);
    const targetIds = await page.evaluate(() => JSON.parse(localStorage.getItem("e2e.last-generation-batch") ?? "[]") as number[]);
    expect(targetIds).toContain(200);
    expect(targetIds).not.toContain(2);
    expect(targetIds).not.toContain(3);
  });

  test("edits generation text inline without changing the subtitle", async ({ page }) => {
    const row = page.locator(".line").filter({ hasText: "#22570" });
    await expect(row.getByText("<losing battle>", { exact: true }).first()).toBeVisible();
    await row.getByRole("button", { name: "Edit generation text" }).click();
    await row.getByLabel("Generation text").fill("losing battle");
    await row.getByRole("button", { name: "Save override" }).click();

    await expect(row.getByText("Override", { exact: true })).toBeVisible();
    await expect(row.getByText("losing battle", { exact: true })).toBeVisible();
    await expect(row.getByText("<losing battle>", { exact: true })).toBeVisible();

    await row.getByRole("button", { name: "Edit generation text" }).click();
    await row.getByRole("button", { name: "Clear override" }).click();
    await expect(row.getByText("Plain", { exact: true })).toBeVisible();
    await expect(row.getByText(/Override cleared/)).toBeVisible();
  });

  test("keeps a candidate separate until explicit acceptance", async ({ page }) => {
    const row = page.locator(".line").filter({ hasText: "#22570" });
    await page.getByRole("button", { name: "Start engine" }).click();
    await row.getByRole("button", { name: "Try candidate" }).click();
    await expect(row.getByText("candidate ready", { exact: true })).toBeVisible();
    await expect(row.getByRole("button", { name: "Accept candidate" })).toBeVisible();
    await row.getByRole("button", { name: "Accept candidate" }).click();
    await expect(row.getByText("Candidate accepted.")).toBeVisible();
    await expect(row.getByRole("button", { name: "Re-generate" })).toBeVisible();
  });

  test("Refresh hydrates newly completed generations without a tab switch", async ({ page }) => {
    const completedIds = generatableLines.slice(0, 3).map((line) => line.id);
    await page.evaluate((ids) => {
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(ids));
    }, completedIds);

    await page.getByRole("button", { name: "Refresh" }).click();

    await expect(page.getByRole("button", { name: "Generate missing (101)" })).toBeVisible();
    await expect(page.locator(".line-action").getByRole("button", { name: "Re-generate" })).toHaveCount(3);
  });

  test("keeps voice-changed clips playable, regeneratable, and removable", async ({ page }) => {
    const completedIds = generatableLines.slice(0, 3).map((line) => line.id);
    const staleIds = completedIds.slice(0, 2);
    await page.evaluate(({ completedIds, staleIds }) => {
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(completedIds));
      localStorage.setItem("e2e.voice-changed-generation-ids", JSON.stringify(staleIds));
    }, { completedIds, staleIds });
    await page.getByRole("button", { name: "Refresh" }).click();

    await expect(page.getByText("voice changed", { exact: true })).toHaveCount(2);
    await expect(page.getByRole("button", { name: "Regenerate voice-changed (2)" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Remove clip" })).toHaveCount(3);

    await page.getByRole("button", { name: "Start engine" }).click();
    await page.getByRole("button", { name: "Regenerate voice-changed (2)" }).click();
    await expect.poll(async () => page.evaluate(() => JSON.parse(localStorage.getItem("e2e.last-generation-batch") ?? "[]") as number[]))
      .toEqual(staleIds);
    await expect(page.getByText("voice changed", { exact: true })).toHaveCount(0);

    await page.getByRole("button", { name: "Remove clip" }).first().click();
    await expect(page.getByRole("button", { name: "Remove clip" })).toHaveCount(2);
  });
});
