import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  page.on("pageerror", error => {
    console.error("[pageerror]", error);
    throw error;
  });
});

test("landing page shows the no-invitation artwork and keeps the admin entry point", async ({ page }) => {
  await page.route("**/api/me", route =>
    route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ message: "Требуется авторизация" }) })
  );
  await page.goto("/");

  const artwork = page.getByRole("img", { name: "У Вас нет приглашения? Свяжитесь с администратором." });
  await expect(artwork).toBeVisible();
  await expect(artwork).toHaveAttribute("src", "/no-invitation.webp");
  await expect(page.getByRole("button", { name: "Вход администратора" })).toBeVisible();
  const response = await page.request.get("/no-invitation.webp");
  expect(response.ok()).toBeTruthy();
  expect(response.headers()["content-type"]).toContain("image/webp");

  await page.getByRole("button", { name: "Вход администратора" }).click();
  await expect(page).toHaveURL(/#\/admin$/);
  await expect(page.getByRole("heading", { name: /Вход в админку/ })).toBeVisible();
});
