# Congratis

Интерактивная новогодняя ёлка с 20 шарами и персональными одноразовыми приглашениями.

## Stack

- Rust, Axum, Tokio
- RethinkDB 2.4.x через закреплённый Rust-драйвер `reql = 0.11.2`
- HTML/CSS/JavaScript
- Docker Compose
- Nginx 1.30.8 в отдельном контейнере, SSE через Changefeed

## Локальный запуск RethinkDB-ветки

1. Скопируйте `.env.example` в `.env`.
2. Установите длинный случайный `RETHINKDB_PASSWORD`.
3. Запустите приложение и базу: `docker compose up -d --build`.
4. Создайте приглашения один раз: `docker compose exec app generate-tokens 20`.
5. Сохраните одноразовые токены из вывода команды и передайте их участникам защищённым способом. После подключения frontend токен будет обмениваться на сессию через backend API.

Сайт будет доступен на `http://localhost` (до выпуска сертификата) или `https://$DOMAIN` после настройки DNS и Let's Encrypt.

## Архитектура текущей ветки

- Одна запись `ornaments` на шар; `id` равен номеру шара и защищает от повторного назначения номера.
- В БД хранится SHA-256 токена, исходный токен выводится только в персональной ссылке.
- Погашение приглашения — условный ReQL update одного документа; проверка `used` и запись поздравления выполняются атомарно.
- Конкурентный интеграционный тест проверяет, что из двух одновременных отправок проходит ровно одна.
- Отдельная сессия RethinkDB читает Changefeed; Axum публикует события через SSE.
- `EventSource` обновляет отдельный шар, а при подключении/переподключении загружает snapshot.
- Nginx обслуживает frontend, проксирует API и отключает buffering для SSE.

## Важное ограничение

Текущая ветка `feat/dominator-spa` содержит RethinkDB, атомарное погашение приглашений, Changefeed/SSE и Nginx в Docker. Для production-развёртывания используйте Ubuntu 24.04 LTS; установите Docker Engine скриптом `deploy/scripts/install-docker-ubuntu.sh`. Перед реальным запуском укажите домен, настройте DNS и firewall, получите Let's Encrypt сертификат и завершите production smoke-test. Не публикуйте RethinkDB или порт приложения напрямую.

Не коммитьте `.env` или файл с приглашениями. Первый выпуск TLS: `bash deploy/scripts/issue-certificate.sh`. Автоматическое продление настраивается systemd unit/timer из `deploy/systemd/`.

Подробности — [ADR RethinkDB](docs/ADR-RETHINKDB-REALTIME.md) и [Deployment](docs/DEPLOYMENT.md).


## Backend authentication API and frontend

The backend uses role-based sessions. Participant invitation tokens are exchanged once at `POST /api/auth/participant/exchange`; the raw token is sent in the JSON body, never in a query string. The response sets an HttpOnly session cookie and a readable CSRF cookie. Send the CSRF cookie value in the `X-CSRF-Token` header for state-changing authenticated requests.

Administrator login is `POST /api/auth/admin/login` with `username` and `password`. Configure `ADMIN_USERNAME` and an Argon2id `ADMIN_PASSWORD_HASH` in the private `.env` file. Generate the hash with `docker compose run --rm -e ADMIN_PASSWORD='your-long-password' app hash-admin-password`. Do not store the plaintext password in `.env` or source control.

Authenticated endpoints:
- `GET /api/me` — current identity and CSRF token.
- `POST /api/auth/logout` — revoke current session (CSRF required).
- `GET /api/greetings` and `GET /api/events` — shared team tree (participant or admin).
- `POST /api/greetings` — participant-only, one greeting per invitation (CSRF required).
- `GET /api/admin/participants` — administrator-only participant and invitation status.
- `POST /api/admin/invitations/{number}/reissue` — rotate an unactivated invitation; returns the new raw token once.
- `POST /api/admin/invitations/{number}/revoke` — revoke an unactivated invitation.

`/api/health` remains public and returns only service availability. The Dominator/WASM frontend uses these endpoints for participant and administrator flows; browser tests cover invitation activation, greeting submission, admin invitation management, mobile layout and SSE updates.
