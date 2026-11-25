import { expect, test } from "@playwright/test";

const participant = { subject_id: "1", role: "participant", csrf_token: "csrf-test" };
const admin = { subject_id: "admin", role: "admin", csrf_token: "csrf-admin" };

async function mockParticipantSession(page, { exchangeStatus = 200 } = {}) {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ message: "Требуется авторизация" }) })
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
  await expect(page.getByText("Твой шар №1")).toBeVisible();
  await expect(page.locator(".ornament")).toHaveCount(20);
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
});

test("failed invite exchange keeps the invitation token available for retry", async ({ page }) => {
  await mockParticipantSession(page, { exchangeStatus: 409 });
  await page.goto("/#invite=one-time-token");

  await expect(page.getByText(/Не удалось активировать приглашение/)).toBeVisible();
  await expect.poll(() => page.evaluate(() => location.hash)).toBe("#invite=one-time-token");
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

  await page.getByRole("textbox", { name: "Как тебя зовут?", exact: true }).fill("Алексей");
  await page.getByRole("textbox", { name: "Твоё поздравление", exact: true }).fill("Счастья и добра в новом году!");
  const submit = page.getByRole("button", { name: "Опубликовать поздравление" });
  await submit.click();
  await submit.click();

  await expect.poll(() => submissions).toBe(1);
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
  await page.route("**/api/events", route =>
    route.fulfill({
      status: 200,
      contentType: "text/event-stream",
      headers: { "cache-control": "no-cache", connection: "keep-alive" },
      body: 'event: greeting\ndata: {"ornament_number":2,"display_name":"Мария","message":"Счастья и радости!"}\n\n'
    })
  );

  await page.goto("/#invite=valid-invite-token");
  const ornament = page.getByRole("button", { name: "Шар номер 2", exact: true });
  await expect(ornament).toHaveClass(/filled/);
  await ornament.click();
  await expect(page.getByRole("heading", { name: "С теплом, Мария" })).toBeVisible();
  await expect(page.getByText("Счастья и радости!")).toBeVisible();
});
