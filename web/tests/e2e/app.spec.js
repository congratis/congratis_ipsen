import { expect, test } from "@playwright/test";

const participant = { subject_id: "1", role: "participant", csrf_token: "csrf-test" };
const admin = { subject_id: "admin", role: "admin", csrf_token: "csrf-admin" };

test.beforeEach(async ({ page }) => {
  page.on("console", message => console.log("[console]", message.type(), message.text()));
  page.on("pageerror", error => {
    console.error("[pageerror]", error);
    throw error;
  });
  page.on("response", response => {
    if (response.url().includes("/api/")) {
      console.log("[api]", response.status(), response.url());
    }
  });
});

async function mockParticipantSession(page, { exchangeStatus = 200, participantCount = 20, meUser = null } = {}) {
  await page.route("**/api/me", route =>
    meUser
      ? route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(meUser) })
      : route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ message: "Требуется авторизация" }) })
  );
  await page.route("**/api/auth/participant/exchange", route => {
    if (exchangeStatus !== 200) {
      return route.fulfill({
        status: exchangeStatus,
        contentType: "application/json",
        body: JSON.stringify({ message: "Приглашение недействительно или уже использовано" })
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      headers: { "set-cookie": "congratis_session=test; Path=/; SameSite=Lax" },
      body: JSON.stringify(participant)
    });
  });
  await page.route("**/api/campaign", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ participant_count: participantCount, submitted_count: 0, campaign_name: "Зимняя кампания" })
    })
  );
  await page.route("**/api/greetings", route => {
    if (route.request().method() === "GET") {
      return route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
    }
    return route.continue();
  });
  await page.route("**/api/events", route =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: ": connected\n\n" })
  );
}

test("successful invite exchange opens the tree and clears the token from the address", async ({ page }) => {
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");

  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect(page.getByText("Зимняя кампания", { exact: true })).toBeVisible();
  await expect(page.getByText("Твой шар №1")).toBeVisible();
  await expect(page.locator(".ornament")).toHaveCount(20);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
});

test("campaign size from the backend controls the number of ornaments", async ({ page }) => {
  await mockParticipantSession(page, { participantCount: 35 });
  await page.goto("/#invite=valid-invite-token");

  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect(page.locator(".ornament")).toHaveCount(35);
  await expect(page.locator(".ornament.dense")).toHaveCount(35);
});

for (const participantCount of [1, 2, 20, 35]) {
  test(`campaign layout supports ${participantCount} participants on mobile and desktop`, async ({ page }) => {
    for (const viewport of [{ width: 375, height: 812 }, { width: 1440, height: 900 }]) {
      await page.setViewportSize(viewport);
      await mockParticipantSession(page, { participantCount });
      await page.goto("/#invite=valid-invite-token");

      const ornaments = page.locator(".ornament");
      await expect(ornaments).toHaveCount(participantCount);
      await expect(ornaments.first()).toHaveAttribute("aria-label", "Шар номер 1");
      await expect(ornaments.last()).toHaveAttribute("aria-label", `Шар номер ${participantCount}`);

      const layout = await page.evaluate(() => {
        const frame = document.querySelector(".tree-frame").getBoundingClientRect();
        const items = [...document.querySelectorAll(".ornament")].map(item => item.getBoundingClientRect());
        return {
          viewportWidth: window.innerWidth,
          documentWidth: document.documentElement.scrollWidth,
          frameLeft: frame.left,
          frameRight: frame.right,
          ornaments: items.map(rect => ({ left: rect.left, right: rect.right, top: rect.top, bottom: rect.bottom }))
        };
      });

      expect(layout.documentWidth).toBeLessThanOrEqual(layout.viewportWidth);
      for (const item of layout.ornaments) {
        expect(item.left).toBeGreaterThanOrEqual(layout.frameLeft - 1);
        expect(item.right).toBeLessThanOrEqual(layout.frameRight + 1);
        expect(item.top).toBeGreaterThanOrEqual(0);
      }
      await page.unrouteAll({ behavior: "wait" });
    }
  });
}

test("ornaments are keyboard-operable and expose an accessible focus target", async ({ page }) => {
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");

  const ownOrnament = page.getByRole("button", { name: "Шар номер 1", exact: true });
  await ownOrnament.focus();
  await expect(ownOrnament).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Добавь своё пожелание" })).toBeVisible();
});


test("failed invite exchange clears the consumed invitation token from the address", async ({ page }) => {
  await mockParticipantSession(page, { exchangeStatus: 409 });
  await page.goto("/#invite=one-time-token");

  await expect(page.getByText(/Не удалось активировать приглашение/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
});

test("failed invite exchange preserves an existing cookie session", async ({ page }) => {
  await mockParticipantSession(page, { exchangeStatus: 409, meUser: participant });
  await page.goto("/#invite=one-time-token");

  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect(page.getByText(/Не удалось активировать приглашение/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
});

test("participant draft is restored from local storage and updated while typing", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("congratis:draft:1", JSON.stringify({
      surname: "Иванова",
      first_name: "Мария",
      message: "Пусть новый год принесёт радость!"
    }));
  });
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await page.getByRole("button", { name: "Шар номер 1", exact: true }).click();

  const surname = page.getByRole("textbox", { name: "Фамилия", exact: true });
  const firstName = page.getByRole("textbox", { name: "Имя", exact: true });
  const message = page.getByRole("textbox", { name: "Твоё поздравление", exact: true });
  await expect(surname).toHaveValue("Иванова");
  await expect(firstName).toHaveValue("Мария");
  await expect(message).toHaveValue("Пусть новый год принесёт радость!");

  await surname.fill("Петров");
  await firstName.fill("Иван");
  await message.fill("Здоровья и исполнения желаний!");
  const draft = await page.evaluate(() => JSON.parse(localStorage.getItem("congratis:draft:1")));
  expect(draft).toEqual({
    surname: "Петров",
    first_name: "Иван",
    message: "Здоровья и исполнения желаний!"
  });
});


test("submission dialog traps keyboard focus and restores it without discarding the draft", async ({ page }) => {
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();

  const ornament = page.getByRole("button", { name: "Шар номер 1", exact: true });
  await ornament.click();
  let dialog = page.getByRole("dialog", { name: "Добавь своё пожелание" });
  await expect(dialog).toBeVisible();

  const close = dialog.getByRole("button", { name: "Закрыть форму поздравления" });
  const submit = dialog.getByRole("button", { name: "Опубликовать поздравление" });
  await expect(close).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(submit).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();

  await dialog.getByRole("textbox", { name: "Фамилия", exact: true }).fill("Иванов");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(ornament).toBeFocused();
  await expect.poll(() => page.evaluate(() => localStorage.getItem("congratis:draft:1")))
    .toContain("Иванов");

  await ornament.click();
  dialog = page.getByRole("dialog", { name: "Добавь своё пожелание" });
  await expect(dialog.getByRole("textbox", { name: "Фамилия", exact: true })).toHaveValue("Иванов");
  await dialog.getByRole("button", { name: "Закрыть форму поздравления" }).click();
  await expect(dialog).toHaveCount(0);
  await expect(ornament).toBeFocused();
});

test("participant can submit a greeting only once even after a rapid double click", async ({ page }) => {
  await mockParticipantSession(page);
  let submissions = 0;

  await page.route("**/api/greetings", async route => {
    if (route.request().method() === "GET") {
      return route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
    }
    submissions += 1;
    await new Promise(resolve => setTimeout(resolve, 350));
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({ message: "Поздравление сохранено", ornament_number: 1 })
    });
  });

  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await page.getByRole("button", { name: "Шар номер 1", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Добавь своё пожелание" })).toBeVisible();

  await page.getByRole("textbox", { name: "Фамилия", exact: true }).fill("Смирнов");
  await page.getByRole("textbox", { name: "Имя", exact: true }).fill("Алексей");
  await page.getByRole("textbox", { name: "Твоё поздравление", exact: true }).fill("Счастья и добра в новом году!");
  const submit = page.getByRole("button", { name: "Опубликовать поздравление" });
  await submit.click();
  await submit.click();

  await expect.poll(() => submissions).toBe(1);
});

test("participant can attach an optional image avatar to a greeting", async ({ page }) => {
  await mockParticipantSession(page);
  let submitted = null;
  await page.route("**/api/greetings", async route => {
    if (route.request().method() === "GET") {
      return route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
    }
    submitted = route.request().postDataJSON();
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({ message: "Поздравление сохранено", ornament_number: 1 })
    });
  });

  await page.goto("/#invite=valid-invite-token");
  await page.getByRole("button", { name: "Шар номер 1", exact: true }).click();
  await page.getByRole("textbox", { name: "Фамилия", exact: true }).fill("Иванов");
  await page.getByRole("textbox", { name: "Имя", exact: true }).fill("Иван");
  await page.getByRole("textbox", { name: "Твоё поздравление", exact: true }).fill("Счастья!");
  await page.locator('input[type="file"]').setInputFiles({
    name: "avatar.png",
    mimeType: "image/png",
    buffer: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC", "base64")
  });
  await expect(page.getByRole("dialog").locator(".note")).toContainText("Фотография добавлена");
  await page.getByRole("button", { name: "Опубликовать поздравление" }).click();
  await expect.poll(() => submitted).not.toBeNull();
  expect(submitted.surname).toBe("Иванов");
  expect(submitted.first_name).toBe("Иван");
  expect(submitted.avatar_data_url).toMatch(/^data:image\/png;base64,/);
});

test("admin can reissue an invitation and select the resulting URL for copying", async ({ page }) => {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );
  const participants = [{
    ornament_number: 1,
    activated: false,
    submitted: false,
    revoked: false,
    display_name: null,
    created_at: 0,
    used_at: null
  }];
  await page.route("**/api/admin/participants", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(participants) })
  );
  await page.route("**/api/admin/invitations/1/reissue", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ornament_number: 1, token: "fresh-token" })
    })
  );

  await page.goto("/#/admin");
  await expect(page.getByRole("heading", { name: /Панель администратора/ })).toBeVisible();
  await page.getByRole("button", { name: "Перевыпустить ссылку" }).click();

  const invite = page.getByLabel("Одноразовая ссылка-приглашение");
  await expect(invite).toHaveValue(/#invite=fresh-token$/);
  await invite.click();
  await expect.poll(() => invite.evaluate(element => element.selectionEnd - element.selectionStart))
    .toBeGreaterThan(10);
});

test("admin can revoke an unused invitation and the participant list reflects the change", async ({ page }) => {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );

  let revoked = false;
  await page.route("**/api/admin/participants", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([{
        ornament_number: 1,
        activated: false,
        submitted: false,
        revoked,
        display_name: null,
        created_at: 0,
        used_at: null
      }])
    })
  );
  await page.route("**/api/admin/invitations/1/revoke", async route => {
    expect(route.request().method()).toBe("POST");
    revoked = true;
    await route.fulfill({ status: 204 });
  });

  await page.goto("/#/admin");
  await expect(page.getByRole("heading", { name: /Панель администратора/ })).toBeVisible();
  await expect(page.getByText("Ожидает участника")).toBeVisible();

  page.once("dialog", async dialog => {
    expect(dialog.type()).toBe("confirm");
    expect(dialog.message()).toContain("шара №1");
    await dialog.accept();
  });
  await page.getByRole("button", { name: "Отозвать" }).click();

  await expect(page.getByText("Приглашение отозвано")).toBeVisible();
  await expect(page.getByRole("button", { name: "Отозвать" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Перевыпустить ссылку" })).toHaveCount(0);
});


test("admin can cancel invitation revocation without changing its status", async ({ page }) => {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );
  let revokeRequests = 0;
  await page.route("**/api/admin/participants", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([{
        ornament_number: 1,
        activated: false,
        submitted: false,
        revoked: false,
        display_name: null,
        created_at: 0,
        used_at: null
      }])
    })
  );
  await page.route("**/api/admin/invitations/1/revoke", async route => {
    revokeRequests += 1;
    await route.fulfill({ status: 204 });
  });

  await page.goto("/#/admin");
  await expect(page.getByText("Ожидает участника")).toBeVisible();
  page.once("dialog", async dialog => {
    expect(dialog.type()).toBe("confirm");
    await dialog.dismiss();
  });
  await page.getByRole("button", { name: "Отозвать" }).click();

  await expect(page.getByText("Ожидает участника")).toBeVisible();
  await expect(page.getByRole("button", { name: "Отозвать" })).toBeVisible();
  expect(revokeRequests).toBe(0);
});



test("mobile viewport keeps the tree and all ornaments inside the screen", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");

  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect(page.locator(".ornament")).toHaveCount(20);

  const layout = await page.evaluate(() => {
    const frame = document.querySelector(".tree-frame").getBoundingClientRect();
    const ornaments = [...document.querySelectorAll(".ornament")].map(element => {
      const rect = element.getBoundingClientRect();
      return {
        left: rect.left,
        right: rect.right,
        top: rect.top,
        bottom: rect.bottom
      };
    });
    return {
      viewportWidth: window.innerWidth,
      documentWidth: document.documentElement.scrollWidth,
      frameLeft: frame.left,
      frameRight: frame.right,
      ornaments
    };
  });

  expect(layout.documentWidth).toBeLessThanOrEqual(layout.viewportWidth);
  for (const ornament of layout.ornaments) {
    expect(ornament.left).toBeGreaterThanOrEqual(layout.frameLeft - 1);
    expect(ornament.right).toBeLessThanOrEqual(layout.frameRight + 1);
  }
});

test("participant receives a published greeting through the SSE stream", async ({ page }) => {
  await mockParticipantSession(page);
  let campaignReads = 0;
  await page.route("**/api/campaign", route => {
    campaignReads += 1;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ participant_count: 20, submitted_count: campaignReads > 1 ? 1 : 0 })
    });
  });
  await page.route("**/api/events", route =>
    route.fulfill({
      status: 200,
      contentType: "text/event-stream",
      headers: { "cache-control": "no-cache", connection: "keep-alive" },
      body: 'event: greeting\ndata: {"campaign_id":"legacy-default","ornament_number":2,"display_name":"Мария","message":"Счастья и радости!"}\n\n'
    })
  );

  await page.goto("/#invite=valid-invite-token");
  const ornament = page.getByRole("button", { name: "Шар номер 2", exact: true });
  await expect(ornament).toHaveClass(/filled/);
  await expect(page.getByRole("progressbar", { name: "Опубликованные поздравления" })).toHaveAttribute("aria-valuenow", "1");
  await expect(page.locator(".progress-count")).toHaveText("1 / 20");
  await ornament.click();
  await expect(page.getByRole("heading", { name: "С теплом, Мария" })).toBeVisible();
  await expect(page.getByText("Счастья и радости!")).toBeVisible();
});

test("SSE reconnect reconciles greetings and campaign progress after a dropped connection", async ({ page }) => {
  await mockParticipantSession(page);
  let eventConnections = 0;
  let greetingReads = 0;
  let campaignReads = 0;

  await page.route("**/api/greetings", route => {
    greetingReads += 1;
    const greetings = greetingReads >= 3
      ? [{ ornament_number: 2, display_name: "Мария", message: "Счастья и радости!" }]
      : [];
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(greetings)
    });
  });
  await page.route("**/api/campaign", route => {
    campaignReads += 1;
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ participant_count: 20, submitted_count: campaignReads >= 3 ? 1 : 0 })
    });
  });
  await page.route("**/api/events", route => {
    eventConnections += 1;
    const retryDelay = eventConnections === 1 ? 100 : 60000;
    return route.fulfill({
      status: 200,
      contentType: "text/event-stream",
      headers: { "cache-control": "no-cache" },
      body: `retry: ${retryDelay}\n\n`
    });
  });

  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect.poll(() => eventConnections).toBeGreaterThanOrEqual(2);

  const ornament = page.getByRole("button", { name: "Шар номер 2", exact: true });
  await expect(ornament).toHaveClass(/filled/);
  await expect(page.getByRole("progressbar", { name: "Опубликованные поздравления" }))
    .toHaveAttribute("aria-valuenow", "1");
  await expect(page.locator(".progress-count")).toHaveText("1 / 20");
});

test("preloads greeting avatars before opening a greeting", async ({ page }) => {
  await mockParticipantSession(page);
  let avatarRequests = 0;
  await page.route("**/avatars/*.png", async route => {
    avatarRequests += 1;
    await route.fulfill({
      status: 200,
      contentType: "image/png",
      headers: { "cache-control": "public, max-age=31536000, immutable" },
      body: Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC", "base64")
    });
  });
  await page.route("**/api/greetings", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { ornament_number: 2, display_name: "Борис", surname: "Петров", first_name: "Борис", avatar_url: "/avatars/0123456789abcdef0123456789abcdef.png", message: "Счастья!" }
      ])
    })
  );

  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await expect.poll(() => avatarRequests).toBe(1);

  await page.getByRole("button", { name: "Шар номер 2", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "С теплом, Борис" });
  const avatar = dialog.locator("img.participant-avatar");
  await expect(avatar).toBeVisible();
  await expect.poll(() => avatar.evaluate(image => image.complete && image.naturalWidth > 0)).toBe(true);
});

test("greeting viewer navigates in deterministic ornament order and exposes dialog semantics", async ({ page }) => {
  await mockParticipantSession(page);
  await page.route("**/api/greetings", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { ornament_number: 2, display_name: "Борис", message: "Пусть сбудутся мечты!" },
        { ornament_number: 1, display_name: "Анна", message: "Счастья и здоровья!" }
      ])
    })
  );

  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await page.getByRole("button", { name: "Шар номер 1", exact: true }).click();

  const dialog = page.getByRole("dialog", { name: "С теплом, Анна" });
  await expect(dialog).toBeVisible();
  const close = dialog.getByRole("button", { name: "Закрыть поздравление" });
  const returnButton = dialog.getByRole("button", { name: "Вернуться к ёлке" });
  const dialogState = await dialog.evaluate(element => ({
    tag: element.tagName,
    open: element.open,
    modal: element.matches(":modal"),
    active: document.activeElement?.outerHTML
  }));
  console.log("native dialog state", dialogState);
  await expect(close).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  console.log("active after Shift+Tab", await page.evaluate(() => document.activeElement?.outerHTML));
  await expect(returnButton).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();
  await expect(dialog.getByText("Поздравление 1 из 2")).toBeVisible();
  await expect(dialog.getByText("Счастья и здоровья!")).toBeVisible();

  await dialog.getByRole("button", { name: "Следующее поздравление" }).click();
  await expect(page.getByRole("dialog", { name: "С теплом, Борис" })).toBeVisible();
  await expect(page.getByText("Поздравление 2 из 2")).toBeVisible();
  await expect(page.getByText("Пусть сбудутся мечты!")).toBeVisible();

  await page.getByRole("dialog").getByRole("button", { name: "Предыдущее поздравление" }).click();
  await expect(page.getByRole("dialog", { name: "С теплом, Анна" })).toBeVisible();
  await expect(page.locator("#greeting-dialog")).toHaveJSProperty("open", true);
  await expect(page.locator("#greeting-dialog").locator(":focus")).toHaveCount(1);

  await expect(page.locator("#greeting-dialog")).toHaveAttribute("open", "");
  await expect(page.locator("#greeting-dialog")).toHaveJSProperty("open", true);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Шар номер 1", exact: true })).toBeFocused();
});

test("campaign summary drives the accessible candy cane progress indicator", async ({ page }) => {
  await mockParticipantSession(page);
  await page.route("**/api/campaign", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ participant_count: 4, submitted_count: 2 })
    })
  );

  await page.goto("/#invite=valid-invite-token");
  const progress = page.getByRole("progressbar", { name: "Опубликованные поздравления" });
  await expect(progress).toHaveAttribute("aria-valuemin", "0");
  await expect(progress).toHaveAttribute("aria-valuemax", "4");
  await expect(progress).toHaveAttribute("aria-valuenow", "2");
  await expect(page.locator(".progress-count")).toHaveText("2 / 4");
  await expect.poll(() => page.locator(".candy-fill").evaluate(element => element.style.width)).toBe("50%");
});

test("presentation mode navigates greetings and supports audio controls and silent fallback", async ({ page }) => {
  await mockParticipantSession(page);
  await page.route("**/api/greetings", route =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify([
        { ornament_number: 2, display_name: "Борис", message: "Пусть сбудутся мечты!" },
        { ornament_number: 1, display_name: "Анна", message: "Счастья и здоровья!" }
      ])
    })
  );
  await page.route("https://upload.wikimedia.org/**", route => route.abort());

  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await page.getByRole("button", { name: "▶ Play" }).click();

  let dialog = page.getByRole("dialog", { name: "С теплом, Анна" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Поздравление 1 из 2")).toBeVisible();
  await page.getByRole("button", { name: "Следующее поздравление" }).click();
  dialog = page.getByRole("dialog", { name: "С теплом, Борис" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Поздравление 2 из 2")).toBeVisible();
  await expect(page.locator(".ornament.presenting")).toHaveAttribute("aria-label", "Шар номер 2");
  const likeButton = dialog.locator(".presentation-like");
  await expect(likeButton).toBeVisible();
  await expect(likeButton.locator(".presentation-like-heart")).toHaveCSS("font-size", "36px");

  // The last forward action must lead to a distinct finale, not a blank/overflow state.
  await page.getByRole("button", { name: "Следующее поздравление" }).click();
  const finale = page.locator(".presentation-finale");
  await expect(finale).toBeVisible();
  await expect(finale.locator("h2")).toHaveText("С Новым годом!");
  await expect(finale.getByText("Все 2 поздравлений просмотрены")).toBeVisible();

  // Replay starts from the first greeting and restores its ornament highlight.
  // Footer controls are siblings of the finale content inside the presentation card.
  await page.getByRole("button", { name: "Повторить" }).click();
  await expect(page.locator(".presentation-finale")).toHaveCount(0);
  dialog = page.getByRole("dialog", { name: "С теплом, Анна" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Поздравление 1 из 2")).toBeVisible();
  await expect(page.locator(".ornament.presenting")).toHaveAttribute("aria-label", "Шар номер 1");

  await page.getByRole("button", { name: "Включить или выключить звук" }).click();
  await page.getByRole("slider", { name: "Громкость музыки" }).fill("0.4");
  await page.getByRole("button", { name: "Вернуться к ёлке" }).click();
  await expect(page.locator(".presentation-overlay")).toHaveCount(0);
  await expect(page.locator(".tree-frame")).toBeVisible();
});

test("presentation mode handles an empty campaign without starting audio", async ({ page }) => {
  await mockParticipantSession(page);
  await page.goto("/#invite=valid-invite-token");
  await expect(page.locator(".tree-frame")).toBeVisible();
  await page.getByRole("button", { name: "▶ Play" }).click();

  const dialog = page.getByRole("dialog", { name: "Ёлка ждёт добрых слов" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByText("Пока пусто")).toBeVisible();
  await page.getByRole("button", { name: "Вернуться к ёлке" }).click();
  await expect(page.locator(".presentation-overlay")).toHaveCount(0);
});

test("admin rejects invalid participant counts for named campaigns", async ({ page }) => {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );
  await page.route("**/api/campaign", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ participant_count: 0, submitted_count: 0 }) })
  );
  await page.route("**/api/greetings", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  await page.route("**/api/events", route =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: ": connected\\n\\n" })
  );
  await page.route("**/api/admin/participants", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  let creationRequests = 0;
  await page.route("**/api/admin/campaigns", route => {
    if (route.request().method() === "POST") {
      creationRequests += 1;
      return route.fulfill({ status: 400, contentType: "application/json", body: JSON.stringify({ message: "unexpected creation request" }) });
    }
    return route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
  });

  await page.goto("/#/admin");
  await page.getByRole("textbox", { name: "Название кампании" }).fill("Неверная кампания");
  await page.getByRole("spinbutton", { name: "Размер новой кампании" }).fill("36");
  await page.getByRole("button", { name: "Создать именованную кампанию" }).click();

  await expect(page.locator(".note")).toContainText("от 1 до 35");
  expect(creationRequests).toBe(0);
});

test("admin can create and delete a named campaign with exact-name confirmation", async ({ page }) => {
  let campaigns = [{
    id: "legacy-default", name: "Существующая кампания", created_at: 1,
    participant_count: 0, submitted_count: 0
  }];
  let deleteRequest = null;

  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );
  await page.route("**/api/campaign", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ participant_count: 0, submitted_count: 0 }) })
  );
  await page.route("**/api/greetings", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  await page.route("**/api/admin/participants", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  await page.route("**/api/events", route =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: ": connected\\n\\n" })
  );
  await page.route("**/api/admin/campaigns", async route => {
    const request = route.request();
    if (request.method() === "GET") {
      return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(campaigns) });
    }
    if (request.method() === "POST") {
      expect(request.headers()["x-csrf-token"]).toBe("csrf-admin");
      expect(request.postDataJSON()).toEqual({ name: "Зимний вечер", participant_count: 2 });
      const created = {
        id: "new-campaign", name: "Зимний вечер", created_at: 2,
        participant_count: 2, submitted_count: 0
      };
      campaigns = [...campaigns, created];
      return route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({
          campaign: created,
          invitations: [
            { ornament_number: 1, token: "winter-one" },
            { ornament_number: 2, token: "winter-two" }
          ]
        })
      });
    }
    return route.continue();
  });
  await page.route("**/api/admin/campaigns/new-campaign", async route => {
    const request = route.request();
    expect(request.method()).toBe("DELETE");
    expect(request.headers()["x-csrf-token"]).toBe("csrf-admin");
    expect(request.postDataJSON()).toEqual({ confirm_name: "Зимний вечер" });
    deleteRequest = request;
    campaigns = campaigns.filter(item => item.id !== "new-campaign");
    return route.fulfill({ status: 204 });
  });

  await page.goto("/#/admin");
  await expect(page.getByRole("heading", { name: "Кампании" })).toBeVisible();
  await page.getByRole("textbox", { name: "Название кампании" }).fill("Зимний вечер");
  await page.getByRole("spinbutton", { name: "Размер новой кампании" }).fill("2");
  await page.getByRole("button", { name: "Создать именованную кампанию" }).click();

  await expect(page.getByText("Зимний вечер", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Ссылка для шара №1")).toHaveValue(/#invite=winter-one$/);
  const legacyCard = page.locator(".admin-item").filter({ hasText: "Системная кампания — удаление запрещено" });
  await expect(legacyCard.getByRole("button", { name: "Удалить кампанию" })).toHaveCount(0);

  page.once("dialog", dialog => {
    expect(dialog.type()).toBe("prompt");
    dialog.accept("Зимний вечер");
  });
  const campaignCard = page.locator(".admin-item").filter({ hasText: "Зимний вечер" });
  await campaignCard.getByRole("button", { name: "Удалить кампанию" }).click();
  await expect.poll(() => deleteRequest !== null).toBe(true);
  await expect(campaignCard).toHaveCount(0);
  await expect(page.getByText("Кампания «Зимний вечер» удалена.")).toBeVisible();
});


test("admin manages invitations within the selected campaign scope", async ({ page }) => {
  const campaign = {
    id: "winter-2026", name: "Зимняя кампания", created_at: 1,
    participant_count: 2, submitted_count: 0
  };
  const participants = [
    { ornament_number: 1, activated: false, submitted: false, revoked: false, display_name: null, created_at: 1, used_at: null },
    { ornament_number: 2, activated: false, submitted: false, revoked: false, display_name: null, created_at: 1, used_at: null }
  ];
  const calls = { list: 0, add: 0, reissue: 0, revoke: 0 };

  await page.route("**/api/me", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(admin) })
  );
  await page.route("**/api/campaign", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ participant_count: 0, submitted_count: 0 }) })
  );
  await page.route("**/api/greetings", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  await page.route("**/api/admin/participants", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" })
  );
  await page.route("**/api/events", route =>
    route.fulfill({ status: 200, contentType: "text/event-stream", body: ": connected\\n\\n" })
  );
  await page.route("**/api/admin/campaigns", route =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify([campaign]) })
  );
  await page.route("**/api/admin/campaigns/winter-2026/participants", route => {
    calls.list += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(participants) });
  });
  await page.route("**/api/admin/campaigns/winter-2026/invitations", async route => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("csrf-admin");
    expect(route.request().postDataJSON()).toEqual({ count: 2 });
    calls.add += 1;
    return route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify([
        { ornament_number: 3, token: "winter-three" },
        { ornament_number: 4, token: "winter-four" }
      ])
    });
  });
  await page.route("**/api/admin/campaigns/winter-2026/invitations/1/reissue", async route => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("csrf-admin");
    calls.reissue += 1;
    return route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ token: "winter-one-reissued" }) });
  });
  await page.route("**/api/admin/campaigns/winter-2026/invitations/2/revoke", async route => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("csrf-admin");
    calls.revoke += 1;
    return route.fulfill({ status: 204 });
  });

  await page.goto("/#/admin");
  const campaignCard = page.locator(".admin-item").filter({ hasText: "Зимняя кампания" });
  await campaignCard.getByRole("button", { name: "Участники и приглашения" }).click();
  await expect(page.getByRole("heading", { name: "Участники: Зимняя кампания" })).toBeVisible();
  await expect(page.getByText("Шар №1")).toBeVisible();

  await page.getByRole("spinbutton", { name: "Количество новых приглашений" }).fill("2");
  await page.getByRole("button", { name: "Добавить приглашения" }).click();
  await expect.poll(() => calls.add).toBe(1);
  await expect(page.getByLabel("Ссылка для шара №3")).toHaveValue(/#invite=winter-three$/);

  const participantOneCard = page.locator(".admin-item").filter({ hasText: "Шар №1" });
  await participantOneCard.getByRole("button", { name: "Перевыпустить ссылку" }).click();
  await expect.poll(() => calls.reissue).toBe(1);
  await expect(page.getByLabel("Одноразовая ссылка-приглашение")).toHaveValue(/#invite=winter-one-reissued$/);

  page.once("dialog", dialog => dialog.accept());
  const participantTwoCard = page.locator(".admin-item").filter({ hasText: "Шар №2" });
  await participantTwoCard.getByRole("button", { name: "Отозвать" }).click();
  await expect.poll(() => calls.revoke).toBe(1);
  await expect.poll(() => calls.list).toBeGreaterThan(1);
});
