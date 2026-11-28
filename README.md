# Congratis

Интерактивная новогодняя кампания: организатор создаёт ёлку, выпускает персональные одноразовые приглашения, участники публикуют поздравления, а команда смотрит их в общей презентации.

## Возможности

- Несколько изолированных кампаний; каждая рассчитана на **1–35 участников**.
- Персональные одноразовые ссылки и сессии с HttpOnly cookie и CSRF-защитой.
- Одно поздравление на участника; повторная отправка блокируется backend.
- Динамическая ёлка, статусы шаров и прогресс в реальном времени через SSE.
- Просмотр поздравлений с навигацией и режим презентации.
- Управление музыкой и лицензированная запись Jingle Bells.
- Админ-панель: создание именованных кампаний, просмотр участников, добавление приглашений, перевыпуск и отзыв неиспользованных ссылок.
- Удаление кампании с подтверждением точного названия; системная legacy-default защищена.
- Сохранение черновика в браузере до успешной публикации.

## Стек

- Backend: Rust, Axum, Tokio.
- Frontend: Rust, Dominator, WebAssembly, **Rollup**; Binaryen 132.0.0 is pinned for reproducible `wasm-opt` optimization.
- Database: RethinkDB 2.4.4.
- Deployment: Docker Compose, Nginx 1.30.8, GlobalSign TLS in production.
- Browser tests: Playwright.

## Локальный запуск

1. Скопируйте `.env.example` в `.env`.
2. Задайте длинный `RETHINKDB_PASSWORD`, локальный `DOMAIN` и `COOKIE_SECURE=false`.
3. Сгенерируйте Argon2id-хеш пароля администратора:

   ```bash
   docker compose run --rm -e ADMIN_PASSWORD='your-long-password' app hash-admin-password
   ```

4. Запишите полученный хеш в `ADMIN_PASSWORD_HASH` в `.env`. Открытый пароль в `.env` не храните.
5. Соберите и запустите приложение:

   ```bash
   docker compose up -d --build
   docker compose ps
   ```

6. Откройте `http://localhost`, перейдите во вход администратора и войдите с `ADMIN_USERNAME` и паролем, соответствующим хешу.
7. Создайте кампанию в админ-панели на 1–35 участников. Сохраните одноразовые ссылки: исходные токены повторно получить нельзя.

Для production используйте HTTPS и `COOKIE_SECURE=true`. Подробности — [Production deployment](docs/DEPLOYMENT.md).

## Важные правила

- Создавайте кампанию либо через админ-панель, либо отдельной CLI-командой `generate-tokens N`. Не запускайте оба способа: кампания создаётся только в пустой таблице.
- `generate-tokens N` поддерживает N от 1 до 35 и выводит исходные токены один раз. Не используйте эту команду при обычном обновлении приложения.
- Не коммитьте `.env`, открытые пароли или файлы с одноразовыми ссылками.
- Не публикуйте RethinkDB и порт приложения в интернет. Публичные порты — только 80/443 на Nginx.
- Никогда не выполняйте `docker compose down -v` на production: команда удалит постоянный volume базы данных и другие persistent volumes.

## Документация

- [Roadmap](Roadmap.md)
- [Production deployment](docs/DEPLOYMENT.md)
- [RethinkDB architecture decision](docs/ADR-RETHINKDB-REALTIME.md)
- [Audio attribution and license](AUDIO_ATTRIBUTION.md)