# Christmas Greetings

Интерактивная новогодняя ёлка с 20 шарами и персональными одноразовыми приглашениями.

## Stack
- Rust, Axum, Tokio, SQLx
- Supabase PostgreSQL
- HTML/CSS/JavaScript
- Docker Compose

## Быстрый старт
1. Создайте проект в Supabase.
2. Скопируйте `.env.example` в `.env` и укажите `DATABASE_URL` из Supabase → Project Settings → Database → Connection string.
3. Запустите `docker compose up -d --build`.
4. Выпустите персональные приглашения: `docker compose exec app generate-tokens 20`.
5. Раздайте каждому сотруднику только его ссылку из CSV-вывода.

Локально сайт будет доступен на `http://localhost:8080`. Production-развёртывание на Ubuntu 24.04 LTS с Nginx и HTTPS для `congratis.ru` описано в [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Безопасность
- Токены генерируются из 256 случайных бит; в БД хранится только SHA-256.
- Одноразовая отметка и запись поздравления выполняются в одной транзакции.
- Уникальные ограничения БД не позволяют привязать два поздравления к одному сотруднику или шару.
- Секреты Supabase остаются только на сервере.

Перед публичным запуском настройте HTTPS reverse proxy, резервное копирование и сетевые ограничения Supabase. Не коммитьте `.env` или CSV-файл приглашений.

Подробности: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
