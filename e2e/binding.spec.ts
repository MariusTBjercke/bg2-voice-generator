import { expect, test } from "@playwright/test";
import { bootstrapProject, goTo } from "./helpers/bootstrap";

test.describe("Guided voice binding", () => {
  test.beforeEach(async ({ page }) => {
    await bootstrapProject(page);
    await goTo(page, "Binding");
  });

  test("shows demographic defaults and optional speaker overrides together", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Voice library" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Import voice" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Design voice" })).toBeVisible();
    await expect(page.getByRole("link", { name: "Manage harvested samples" })).toHaveAttribute("href", "/harvest");
    await expect(page.getByText("Weathered traveler")).toBeVisible();
    await expect(page.locator("li.profile-row").filter({ hasText: "Weathered traveler" }).getByText("Imported", { exact: true })).toBeVisible();
    await expect(page.locator("li.profile-row").filter({ hasText: "Young Amnian noble" }).getByText("Designed", { exact: true })).toBeVisible();
    await expect(page.locator("#voice-library-panel li.profile-row").filter({ hasText: "Xzar — harvested" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: /Voice library/ })).toContainText("28 custom · 1 harvested");
    await expect(page.getByRole("heading", { name: "Demographic defaults" })).toBeVisible();
    await expect(page.getByRole("heading", { name: /Speaker overrides/ })).toBeVisible();
    await expect(page.getByText("Male / Human / Humanoid")).toBeVisible();
    await expect(page.getByLabel("Effective voice readiness")).toContainText("1 demographic defaults");
    await expect(page.getByRole("button", { name: "Apply defaults" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Auto-configure all" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Use personal samples for all" })).toBeVisible();
  });

  test("imports a custom voice with an exact transcript", async ({ page }) => {
    await page.getByRole("button", { name: "Import voice" }).click();
    await page.getByLabel("Profile name").first().fill("Campfire storyteller");
    await page.getByRole("button", { name: "Choose 1–4 audio files…" }).click();
    await page.getByPlaceholder("Exact words spoken in this clip").fill("The fire is warm tonight.");
    await page.getByRole("button", { name: "Save imported voice" }).click();
    await expect(page.getByText("Campfire storyteller")).toBeVisible();
  });

  test("renders and freezes one designed candidate", async ({ page }) => {
    await page.getByRole("button", { name: "Design voice" }).click();
    await page.getByLabel("Profile name").fill("Quiet courtier");
    await page.getByRole("button", { name: "Generate 3 auditions" }).click();
    await expect(page.getByText("Candidate 1 · seed 42")).toBeVisible();
    await page.getByText("Candidate 2 · seed 137").click();
    await page.getByRole("button", { name: "Save selected voice" }).click();
    await expect(page.getByText("Quiet courtier")).toBeVisible();
  });

  test("filters the library and explains playable reference details", async ({ page }) => {
    const library = page.locator("#voice-library-panel");
    await expect(library).toContainText("reference clip and its exact transcript");
    await library.getByLabel("Origin").selectOption("harvested");

    const harvested = library.locator("li.profile-row").filter({ hasText: "Xzar" });
    await expect(harvested).toContainText("1 reference clip");
    await expect(harvested.getByRole("button", { name: "Play", exact: true })).toBeVisible();
    await expect(harvested.getByRole("link", { name: "Manage in Harvest" })).toHaveAttribute(
      "href",
      "/harvest?identity=22570%3A1",
    );
    await expect(harvested.getByRole("button", { name: "Rename" })).toHaveCount(0);
    await expect(harvested.getByRole("button", { name: "Delete…" })).toHaveCount(0);

    await harvested.getByText("Reference details").click();
    await expect(harvested).toContainText("Transcript: A fine day for murder.");
    await expect(harvested).toContainText("Source: sound xzar01 · strref 1000");
    await expect(harvested.getByRole("button", { name: "Play clip" })).toBeVisible();
    await expect(library).toHaveScreenshot("binding-voice-library.png", { animations: "disabled" });

    await library.getByPlaceholder("name, transcript, source, or design attribute…").fill("xzar01");
    await expect(harvested).toBeVisible();
    await library.getByLabel("Availability").selectOption("missing_local_audio");
    await expect(library.getByText("No voices match these library filters.")).toBeVisible();
  });

  test("shows editable custom profiles and designed attributes", async ({ page }) => {
    const library = page.locator("#voice-library-panel");
    const imported = library.locator("li.profile-row").filter({ hasText: "Weathered traveler" });
    const designed = library.locator("li.profile-row").filter({ hasText: "Young Amnian noble" });
    await expect(imported.getByRole("button", { name: "Rename" })).toBeVisible();
    await expect(imported.getByRole("button", { name: "Delete…" })).toBeVisible();
    await expect(designed.getByRole("button", { name: "Rename" })).toBeVisible();
    await designed.getByText("Reference details").click();
    await expect(designed).toContainText("female · young adult · moderate pitch · british accent");
  });

  test("paginates library results and resets to the first page when filters change", async ({ page }) => {
    const library = page.locator("#voice-library-panel");
    await expect(library.getByText("1 / 2", { exact: true })).toBeVisible();
    await library.getByRole("button", { name: "Next page" }).click();
    await expect(library.getByText("2 / 2", { exact: true })).toBeVisible();

    const search = library.getByPlaceholder("name, transcript, source, or design attribute…");
    await search.fill("Young Amnian noble");
    await expect(library.getByText("Young Amnian noble")).toBeVisible();
    await search.fill("no such fixture voice");
    await expect(library.getByText("No voices match these library filters.")).toBeVisible();
  });

  test("uses imported and designed profiles in pools and personal overrides", async ({ page }) => {
    await page.getByText("Male / Human / Humanoid").click();
    const group = page.locator("li.group-row").filter({ hasText: "Male / Human / Humanoid" });
    await expect(group.getByText("Weathered traveler")).toBeVisible();
    await expect(group.getByText("Young Amnian noble")).toBeVisible();
    await group.locator("li.donor-row").filter({ hasText: "Weathered traveler" }).getByRole("button", { name: "Remove" }).click();
    await group.getByRole("combobox").first().selectOption("101");
    await group.getByRole("button", { name: "Add custom voice" }).click();

    await page.getByRole("button", { name: /Xzar/ }).first().click();
    const override = page.locator(".profile-override").filter({ hasText: "Assign an imported or designed profile" });
    await override.getByRole("combobox").selectOption("102");
    await override.getByRole("button", { name: "Assign profile" }).click();
    await expect(page.locator(".effective-voice")).toContainText("Young Amnian noble");
  });

  test("identity query selects the character and exposes harvest/generation deep links", async ({
    page,
  }) => {
    await page.goto("/binding?identity=33001");
    await expect(page.getByRole("heading", { name: "Montaron" })).toBeVisible();
    // Montaron borrows Xzar's demographic voice — Review samples opens the donor.
    await expect(page.getByRole("link", { name: "Review samples" })).toHaveAttribute(
      "href",
      "/harvest?identity=22570%3A1",
    );
    await expect(page.getByRole("link", { name: "Open on Generation" })).toHaveAttribute(
      "href",
      "/generation?identity=33001%3A1",
    );
    await expect(page).toHaveURL(/\/binding$/);
  });

  test("can expand a group pool editor with suggest and play", async ({ page }) => {
    await page.getByText("Male / Human / Humanoid").click();
    const group = page.locator("li.group-row").filter({ hasText: "Male / Human / Humanoid" });
    await expect(page.getByRole("button", { name: "Suggest harvested voice" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Clear pool", exact: true })).toBeVisible();
    await expect(group.getByRole("button", { name: "Play", exact: true }).first()).toBeVisible();
    await page.getByRole("button", { name: "Add harvested voice from other demographics…" }).click();
    await expect(group.getByRole("listbox", { name: "Other harvested voice" })).toBeVisible();
    await expect(group.getByRole("searchbox", { name: "Search other harvested voice" })).toBeVisible();
    await expect(group.getByText(/Similar creature type/)).toBeVisible();
  });

  test("renders mirrored harvested memberships once and keeps legacy donors as fallbacks", async ({ page }) => {
    await page.getByText("Male / Human / Humanoid").click();
    const group = page.locator("li.group-row").filter({ hasText: "Male / Human / Humanoid" });
    const xzarRows = group.locator("li.donor-row").filter({ hasText: "Xzar" });
    await expect(xzarRows).toHaveCount(1);
    await expect(xzarRows).toContainText("Harvested");
    await expect(xzarRows.getByRole("button", { name: "Play", exact: true })).toBeVisible();
    await expect(xzarRows.getByRole("button", { name: "Remove" })).toBeVisible();

    const legacy = group.locator("li.donor-row").filter({ hasText: "Montaron" });
    await expect(legacy).toContainText("Harvested · legacy");
    await expect(legacy.getByRole("button", { name: "Remove" })).toBeVisible();
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

  test("can follow another character's voice from the detail pane", async ({ page }) => {
    await page.getByRole("button", { name: /Montaron/ }).first().click();
    const follow = page.locator(".profile-override").filter({ hasText: "Follow another character" });
    await follow.getByRole("combobox").selectOption("1");
    await follow.getByRole("button", { name: "Follow character" }).click();
    await expect(page.locator(".effective-voice")).toContainText("Following");
    await expect(page.locator(".speaker.active")).toContainText("Following character");
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
    await expect(page.getByText(/Saved tuning\. Marked 2 clip\(s\) as voice changed/)).toBeVisible();

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

  test("persists panels and the expanded demographic group across navigation and reload", async ({ page }) => {
    await page.getByText("Male / Human / Humanoid").click();
    await page.getByRole("button", { name: /Xzar/ }).first().click();
    await page.getByPlaceholder("filter groups…").fill("Male");
    await page.getByLabel("Preview dialogue").fill("Persistent preview text");
    await page.getByRole("button", { name: /Characters \(2\)/ }).click();

    await goTo(page, "Generation");
    await goTo(page, "Binding");
    await expect(page.getByRole("button", { name: /Characters \(2\)/ })).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByRole("button", { name: "Suggest harvested voice" })).toBeVisible();

    await page.reload();
    await expect(page.getByRole("button", { name: /Characters \(2\)/ })).toHaveAttribute("aria-expanded", "false");
    await expect(page.getByRole("button", { name: "Suggest harvested voice" })).toBeVisible();
    await expect(page.getByPlaceholder("filter groups…")).toHaveValue("Male");
    await expect(page.getByRole("heading", { name: "Xzar" })).toBeVisible();
    await expect(page.getByLabel("Preview dialogue")).toHaveValue("Persistent preview text");
  });

  test("persists the demographic group page across navigation and reload", async ({ page }) => {
    const groupsPanel = page.locator("#demographic-groups-panel");
    await groupsPanel.getByRole("button", { name: "Next page" }).click();
    await expect(groupsPanel.getByText("2 / 2", { exact: true })).toBeVisible();

    await goTo(page, "Generation");
    await goTo(page, "Binding");
    await expect(page.locator("#demographic-groups-panel").getByText("2 / 2", { exact: true })).toBeVisible();

    await page.reload();
    await expect(page.locator("#demographic-groups-panel").getByText("2 / 2", { exact: true })).toBeVisible();
  });

  test("persists library filters separately from character filters", async ({ page }) => {
    const library = page.locator("#voice-library-panel");
    await library.getByLabel("Origin").selectOption("harvested");
    await page.getByPlaceholder("character name or resref…").fill("Montaron");

    await goTo(page, "Generation");
    await goTo(page, "Binding");
    await expect(page.locator("#voice-library-panel").getByLabel("Origin")).toHaveValue("harvested");
    await expect(page.getByPlaceholder("character name or resref…")).toHaveValue("Montaron");

    await page.reload();
    await expect(page.locator("#voice-library-panel").getByLabel("Origin")).toHaveValue("harvested");
    await expect(page.getByPlaceholder("character name or resref…")).toHaveValue("Montaron");
  });

  test("filters characters by sex and exposes a filtered-voice playlist", async ({ page }) => {
    const characters = page.locator("#characters-list-panel");
    await expect(characters.getByLabel("Sex", { exact: true })).toBeVisible();
    await expect(characters.getByLabel("Voice gender", { exact: true })).toBeVisible();
    await expect(characters.getByLabel("Demographics", { exact: true })).toBeVisible();
    await expect(characters.getByRole("button", { name: /Play filtered voices/ })).toBeVisible();

    await characters.getByLabel("Sex", { exact: true }).selectOption("male");
    await expect(characters.getByRole("button", { name: /^Xzar\b/ })).toBeVisible();
    await expect(characters.getByRole("button", { name: /^Montaron\b/ })).toBeVisible();

    await characters.getByLabel("Sex", { exact: true }).selectOption("female");
    await expect(characters.getByText("No characters match the current filter.")).toBeVisible();
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
