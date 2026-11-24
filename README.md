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
5. Раздайте каждому сотруднику только его персональную ссылку из вывода команды.

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

Ветка `feat/rethinkdb-sse` содержит RethinkDB, атомарное погашение приглашений, Changefeed/SSE и Nginx в Docker. Для production-развёртывания используйте Ubuntu 26.04 LTS; установите Docker Engine скриптом `deploy/scripts/install-docker-ubuntu.sh`. Перед реальным запуском укажите домен, настройте DNS и firewall, получите Let's Encrypt сертификат и завершите production smoke-test. Не публикуйте RethinkDB или порт приложения напрямую.

Не коммитьте `.env` или файл с приглашениями. Первый выпуск TLS: `bash deploy/scripts/issue-certificate.sh`. Автоматическое продление настраивается systemd unit/timer из `deploy/systemd/`.

Подробности — [ADR RethinkDB](docs/ADR-RETHINKDB-REALTIME.md) и [Deployment](docs/DEPLOYMENT.md).
