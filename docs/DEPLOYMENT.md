# Deployment and operations

## Supabase
1. Create a Supabase project.
2. Open **Project Settings → Database → Connection string**.
3. Use the Session Pooler or another connection mode appropriate for a persistent Docker process.
4. Copy `.env.example` to `.env`; set `DATABASE_URL`. URL-encode special characters in the password.
5. The Rust service applies the checked-in SQLx migration at startup.

The app connects directly to PostgreSQL with the credentials in `DATABASE_URL`. Browser code never receives database credentials and does not use the Supabase service-role key. Because SQLx connects directly, authorization is enforced in the application and database role grants; this application does not rely on Supabase RLS for its public routes.

## Docker
```sh
cp .env.example .env
# edit .env
docker compose up -d --build
docker compose logs -f app
```
Open `http://localhost:8080`. Health check: `/api/health`.

## Issue one-time invitations
After the app has run once and created the schema:
```sh
docker compose exec app generate-tokens 20
```
The command writes a CSV with ornament number, personal URL, and raw token to stdout. Capture it in a protected location. Each token is 32 cryptographically random bytes; only its SHA-256 digest is stored in PostgreSQL. The invitation links are bearer credentials: share each only with its intended recipient.

The generator issues ornament numbers from 1 through N and refuses to run if any of those numbers already exist. Do not run it a second time to regenerate links.

## Production checklist
- Put the app behind HTTPS (Caddy, Nginx, managed load balancer).
- Restrict access to the database role and network; never commit `.env`.
- Back up Supabase and test a restore.
- Add edge rate limiting/WAF and operational monitoring.
- Decide organizational retention and privacy requirements for employee names/messages.
- Consider moving schema migration execution to a dedicated release job before scaling replicas.

## API
- `GET /api/health`: database-aware liveness check.
- `GET /api/greetings`: public list of submitted greetings (never returns invitation tokens).
- `POST /api/greetings`: JSON body `{ "token": "...", "display_name": "...", "message": "..." }`.

The token claim is an atomic conditional UPDATE inside the same transaction as the greeting insert. Unique constraints on participant and ornament provide a second line of defense against duplicates.
