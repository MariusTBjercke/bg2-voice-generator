import { expect, test } from "@playwright/test";

test("shows placeholders and previews global pronunciation rules", async ({ page }) => {
  await page.goto("/dictionary");

  await expect(page.getByRole("heading", { name: "Dictionary" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "PC profile" })).toBeVisible();

  await page.getByRole("tab", { name: "Global Rules" }).click();
  await expect(page.getByRole("button", { name: "Disable wwaaAAAAHHHH" })).toBeVisible();
  await page.getByRole("button", { name: "Test pronunciation" }).click();
  await expect(page.getByText(/But\.\.\. I\.\.\. I\.\.\. Wah!/)).toBeVisible();
});

test("adds a user dictionary rule", async ({ page }) => {
  await page.goto("/dictionary");
  await page.getByRole("tab", { name: "Global Rules" }).click();
  await page.getByRole("button", { name: "+ Add rule" }).click();
  await page.getByLabel("Find text").fill("Cyrodiil");
  await page.getByLabel("Speak as").fill("Searohdiil");
  await page.getByRole("button", { name: "Save", exact: true }).click();

  await expect(page.getByText("Cyrodiil", { exact: true })).toBeVisible();
  await expect(page.getByText("Searohdiil", { exact: true })).toBeVisible();
});
