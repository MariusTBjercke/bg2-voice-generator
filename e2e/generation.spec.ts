import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";
import { generatableLines, orphanCompletedGenerationIds } from "./fixtures/data";

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

  test("speaker names deep-link to Binding with that identity", async ({ page }) => {
    const row = page.locator(".line").filter({ hasText: "#22570" });
    const speaker = row.getByRole("link", { name: "Xzar" });
    await expect(speaker).toHaveAttribute("href", "/binding?identity=22570%3A1");
  });

  test("identity query filters the speaker scope", async ({ page }) => {
    await page.goto("/generation?identity=33001");
    await expect(page.getByRole("heading", { name: /Generatable lines \(1\)/ })).toBeVisible();
    await expect(page.getByText("#33001")).toBeVisible();
    await expect(page).toHaveURL(/\/generation$/);
  });

  test("shows cached lines immediately while a revisit refresh is delayed", async ({ page }) => {
    await expect(page.getByText("#22570")).toBeVisible();
    await goTo(page, "Binding");
    await page.evaluate(() => localStorage.setItem("e2e.delay-generatable-ms", "1200"));
    await goTo(page, "Generation");

    await expect(page.getByText("#22570")).toBeVisible({ timeout: 300 });
    await expect(page.getByRole("heading", { name: "Generatable lines (104)" })).toBeVisible();
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

  test("expands truncated original and mapped generation text", async ({ page }) => {
    const row = page.locator(".line").filter({ hasText: "#50000" });
    const showMore = row.getByRole("button", { name: "Show more" });
    await expect(showMore.first()).toBeVisible();
    await expect(
      row.locator('[title*="deliberately long fixture generation line"]').first(),
    ).toBeVisible();

    await showMore.first().click();
    await expect(row.getByRole("button", { name: "Show less" }).first()).toBeVisible();
    await expect(
      row.getByText(/deliberately long fixture generation line used to verify/).first(),
    ).toBeVisible();
  });

  test("combines filters, removes chips, and clears the full scope", async ({ page }) => {
    await page.getByRole("button", { name: "More filters" }).click();
    const speakers = page.locator("#generation-more-filters details").filter({ hasText: "Speakers" }).first();
    await speakers.locator("summary").click();
    await speakers.getByRole("checkbox", { name: /Xzar/ }).check();

    const packAudio = page.getByRole("group", { name: "Attached pack audio" });
    await packAudio.getByLabel("Present", { exact: true }).check();
    await expect(page.getByRole("heading", { name: "Generatable lines (1)" })).toBeVisible();
    await expect(page.getByText("#22571")).toBeVisible();

    await page.getByRole("button", { name: "Remove filter Pack audio present" }).click();
    await expect(page.getByRole("heading", { name: "Generatable lines (103)" })).toBeVisible();

    await page.getByRole("button", { name: "Clear all", exact: true }).click();
    await expect(page.getByRole("heading", { name: "Generatable lines (104)" })).toBeVisible();
  });

  test("defaults to dialogue order and can filter or sort needs-review lines", async ({ page }) => {
    await expect(page.getByLabel("Sort", { exact: true })).toHaveValue("dlg_state");
    // Ready lines only: montdlg (#33001) sorts before xzardlg (#22570).
    await expect(page.locator(".line").first().getByText("#33001")).toBeVisible();

    await page.getByRole("button", { name: "More filters" }).click();
    await page.getByRole("group", { name: "Diagnostics" }).getByLabel("Needs review").check();
    await expect(page.getByRole("heading", { name: "Generatable lines (1)" })).toBeVisible();
    await expect(page.getByText("#22570")).toBeVisible();
    await expect(page.getByText("needs review", { exact: true })).toBeVisible();

    await page.getByRole("button", { name: "Clear all", exact: true }).click();
    await page.getByLabel("Sort", { exact: true }).selectOption("needs_review");
    await expect(page.locator(".line").first().getByText("#22570")).toBeVisible();
  });

  test("restores the advanced-filter panel without restoring pagination", async ({ page }) => {
    await page.getByRole("button", { name: "More filters" }).click();
    await page.getByRole("button", { name: "Next page" }).last().click();
    await page.reload();
    await expect(page.getByRole("button", { name: /Fewer filters/ })).toBeVisible();
    await expect(page.getByText("1 / 2", { exact: true }).last()).toBeVisible();
  });

  test("batch actions target every filtered line, including lines beyond the current page", async ({ page }) => {
    await page.getByRole("button", { name: "Start engine" }).click();
    await page.getByRole("button", { name: "More filters" }).click();
    const speakers = page.locator("#generation-more-filters details").filter({ hasText: "Speakers" }).first();
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
    await expect(row.getByText("I cannot hold them much longer.", { exact: true }).first()).toBeVisible();
    await row.getByRole("button", { name: "Edit generation text" }).click();
    await row.getByLabel("Generation text").fill("I cannot hold them much longer.[dissatisfaction-hnn]");
    await row.getByRole("button", { name: "Save override" }).click();

    await expect(row.getByText("Override", { exact: true })).toBeVisible();
    await expect(row.getByText("I cannot hold them much longer.[dissatisfaction-hnn]", { exact: true })).toBeVisible();
    await expect(row.getByText("I cannot hold them much longer.", { exact: true })).toBeVisible();

    await row.getByRole("button", { name: "Edit generation text" }).click();
    await row.getByRole("button", { name: "Clear override" }).click();
    await expect(row.getByText("Plain", { exact: true })).toBeVisible();
    await expect(page.getByText(/Override cleared/)).toBeVisible();
  });

  test("omits whole-line angle-bracket annotations from the generatable list", async ({ page }) => {
    await expect(page.getByText("<losing battle>", { exact: true })).not.toBeVisible();
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
    await page.getByText("More batch actions", { exact: true }).click();

    await expect(page.getByText("voice changed", { exact: true })).toHaveCount(2);
    await expect(page.getByRole("button", { name: "Re-generate voice-changed (2)" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Re-generate text-changed (0)" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Re-generate all changed (2)" })).toBeDisabled();
    await expect(page.getByRole("button", { name: "Remove clip" })).toHaveCount(3);

    await page.getByRole("button", { name: "Start engine" }).click();
    await page.getByRole("button", { name: "Re-generate all changed (2)" }).click();
    await expect.poll(async () => page.evaluate(() => JSON.parse(localStorage.getItem("e2e.last-generation-batch") ?? "[]") as number[]))
      .toEqual(staleIds);
    await expect(page.getByText("voice changed", { exact: true })).toHaveCount(0);

    page.once("dialog", (dialog) => dialog.accept());
    await page.getByRole("button", { name: "Remove clip" }).first().click();
    await expect(page.getByRole("button", { name: "Remove clip" })).toHaveCount(2);
  });

  test("surfaces blocked orphan clips for preview and removal, not batch regenerate", async ({ page }) => {
    await page.evaluate((ids) => {
      localStorage.setItem("e2e.completed-generation-ids", JSON.stringify(ids));
      localStorage.setItem("e2e.voice-changed-generation-ids", JSON.stringify(ids));
    }, orphanCompletedGenerationIds);
    await page.getByRole("button", { name: "Refresh" }).click();

    await expect(page.getByRole("heading", { name: /Generatable lines \(105\)/ })).toBeVisible();
    const banner = page.getByRole("status").filter({ hasText: "blocked or skipped" });
    await expect(banner).toContainText("1 generated clip is on blocked or skipped lines");
    await banner.getByRole("button", { name: "Show them" }).click();
    await page.getByText("More batch actions", { exact: true }).click();

    await expect(page.getByRole("heading", { name: /Generatable lines \(1\)/ })).toBeVisible();
    const row = page.locator(".line").filter({ hasText: "#99448" });
    await expect(row.getByText("blocked", { exact: true })).toBeVisible();
    await expect(row.getByText("voice changed", { exact: true })).toBeVisible();
    await expect(row.getByRole("button", { name: "Re-generate" })).toBeDisabled();
    await expect(row.getByText(/Blocked by attribution/)).toBeVisible();
    await expect(page.getByRole("button", { name: "Re-generate all (0)" })).toBeDisabled();

    page.once("dialog", (dialog) => dialog.accept());
    await row.getByRole("button", { name: "Remove clip" }).click();
    await expect(page.getByRole("button", { name: "Remove clip" })).toHaveCount(0);
  });
});
