import { expect, test } from "@playwright/test";

test("shows placeholders and previews pronunciation rules", async ({ page }) => {
  await page.goto("/dictionary");

  await expect(page.getByRole("heading", { name: "Dictionary" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "PC profile" })).toBeVisible();

  await page.getByRole("tab", { name: "Pronunciation" }).click();
  await expect(page.getByRole("button", { name: "Disable wwaaAAAAHHHH" })).toBeVisible();
  await page.getByRole("button", { name: "Test pronunciation" }).click();
  await expect(page.getByText(/But\.\.\. I\.\.\. I\.\.\. Wah!/)).toBeVisible();
});

test("adds a user dictionary rule", async ({ page }) => {
  await page.goto("/dictionary");
  await page.getByRole("tab", { name: "Pronunciation" }).click();
  await page.getByRole("button", { name: "+ Add rule" }).click();
  await page.getByLabel("Find text").fill("Cyrodiil");
  await page.getByLabel("Speak as").fill("Searohdiil");
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect(page.getByText("Cyrodiil", { exact: true })).toBeVisible();
  await expect(page.getByText("Searohdiil", { exact: true })).toBeVisible();
});

test("lists default tag rules and adds a spoken-word Bah rule", async ({ page }) => {
  await page.goto("/dictionary");
  await page.getByRole("tab", { name: "Tag rules" }).click();
  await expect(page.getByText("sigh").first()).toBeVisible();
  await expect(page.getByText("[sigh]").first()).toBeVisible();

  await page.getByRole("button", { name: "+ Add tag rule" }).click();
  await page.getByLabel("Tag find text").fill("Bah");
  await page.getByLabel("Tag match kind").selectOption("whole_word");
  await page.getByLabel("OmniVoice tag").selectOption("[dissatisfaction-hnn]");
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect(page.getByText("Bah").first()).toBeVisible();
  await expect(page.getByText("[dissatisfaction-hnn]").first()).toBeVisible();

  await page.getByRole("button", { name: "Test tags" }).click();
  await expect(page.getByText("[dissatisfaction-hnn]! [sigh] This is annoying.")).toBeVisible();
});
