# Multi-campaign administration design

**Status:** implementation contract  
**Decision:** multiple campaigns may coexist; campaign deletion is permanent and requires explicit confirmation.

## Goals

- Keep campaign data isolated while allowing multiple campaigns to run at the same time.
- Preserve the existing campaign and its invitations/greetings during the schema migration.
- Allow administrators to create campaigns, add invitations, inspect participants and permanently delete a campaign.
- Keep one-time invitation tokens secret: only token hashes are persisted; raw tokens are returned once at creation/reissue.
- Preserve the existing Rust + Axum + RethinkDB + Dominator/WASM stack.

## Data model

### `campaigns`

| Field | Type | Meaning |
|---|---|---|
| `id` | string (UUID) | Stable campaign identifier and ReQL primary key |
| `name` | string | Required display name, trimmed, 1–100 Unicode characters |
| `created_at` | integer | UTC Unix timestamp |
| `updated_at` | integer | UTC Unix timestamp |

No global "active campaign" singleton is required: campaign selection is an explicit admin UI state. Public tree/invitation flows are campaign-scoped by the invitation itself.

### `ornaments`

Add `campaign_id: string` to every invitation/participant record. Keep `ornament_number` as the 1–35 slot number **within a campaign**.

The current primary key is the numeric ornament number, which cannot represent the same slot in two campaigns. New records therefore use a deterministic string primary key `<campaign_uuid>:<ornament_number>`. Existing legacy numeric keys remain valid and are retained for the migrated campaign. All new reads and writes must query by `campaign_id`, never by ornament number alone.

Add a unique secondary index `campaign_ornament` over `[campaign_id, ornament_number]` and a secondary index `campaign_id`. Keep the token hash index globally unique in application logic and continue storing only its SHA-256 digest.

### `auth_sessions`

Participant sessions gain `campaign_id`; admin sessions remain global and have no campaign ID. Participant session subjects must identify the campaign-scoped ornament key, not just a number. Deleting a campaign revokes all participant sessions with that campaign ID.

## Migration and rollout

Migration is additive and idempotent:

1. Create `campaigns` table if absent.
2. Create one deterministic legacy campaign ID (for example `legacy-default`) if legacy ornaments exist; create a descriptive name such as `Существующая кампания`.
3. Add `campaign_id = legacy-default` to every existing ornament without changing its primary key, token hash, activation state, submission or timestamps.
4. Add `campaign_id` to participant sessions by resolving their legacy numeric subject against the legacy campaign. Sessions that cannot be resolved are revoked rather than granted access to an arbitrary campaign.
5. Create and wait for required indexes.
6. Verify every legacy ornament belongs to exactly one campaign and that the pre-migration counts of invitations and submitted greetings are unchanged.

Do not drop or recreate the database, tables, indexes, or persistent volume. The migration must be safe to run more than once. Application startup must fail closed if migration verification fails.

## API contract

All mutating admin endpoints require an authenticated admin session and valid CSRF token.

- `GET /api/admin/campaigns` — list campaigns with invitation/submission counts.
- `POST /api/admin/campaigns` — create a campaign and its initial invitations. Body: `{ "name": "...", "participant_count": 1 }`. Response returns campaign metadata and raw invitation tokens once.
- `GET /api/admin/campaigns/{campaign_id}/participants` — campaign-scoped participant list.
- `POST /api/admin/campaigns/{campaign_id}/invitations` — create additional invitations in unused slots; never overwrite existing records.
- `POST /api/admin/campaigns/{campaign_id}/invitations/{number}/reissue` — reissue only an unactivated, unused invitation.
- `POST /api/admin/campaigns/{campaign_id}/invitations/{number}/revoke` — revoke only an unactivated, unused invitation.
- `DELETE /api/admin/campaigns/{campaign_id}` — require a confirmation body containing the exact campaign name. Delete campaign-scoped ornaments and revoke campaign participant sessions before deleting the campaign record.

Existing unscoped admin endpoints are temporary compatibility routes only and must be removed after the UI migrates to the campaign-scoped API. Public APIs and SSE must be scoped to the participant session's campaign.

## Deletion semantics

Deletion is permanent. The UI must show the campaign name and counts of invitations/submissions, then require the administrator to type the exact campaign name. Backend repeats the exact-name check; UI confirmation alone is not a security boundary.

Deletion must:
1. Resolve and validate the campaign.
2. Revoke all participant sessions for that campaign.
3. Delete only ornaments with the campaign ID.
4. Delete the campaign record.
5. Return success only after all steps complete; partial failure must be surfaced and be safely retryable.

Because RethinkDB does not provide cross-table transactions, implement deletion as an idempotent ordered workflow. The campaign remains present until child data has been removed. A failed operation can be retried without touching another campaign.

Submitted greetings are deleted with their campaign only after the explicit confirmation. Deleting an individual invitation is allowed only when it has never been activated or used; submitted greetings are never individually deleted through invitation controls.

## Frontend behavior

- Admin landing screen loads a campaign list, including an empty state.
- Selecting a campaign scopes all participant/invitation requests and displays that campaign's details.
- Create campaign form asks for a name and initial invitation count (1–35).
- Add-invitations action only allocates unused numbers and never silently replaces links.
- Newly issued raw links are shown once with copy affordance and a warning to save them.
- Destructive campaign action displays counts and requires exact-name input.
- After deletion, selection moves to the campaign list; no stale campaign data remains in client state.

## Required tests

- Idempotent migration against empty, legacy-populated and already-migrated databases.
- Two campaigns can both have ornament number 1 without key collision.
- Every list, invitation activation, greeting submission, greeting snapshot and SSE event is isolated by campaign.
- Duplicate slot and duplicate token hashes are rejected.
- Cross-campaign IDs cannot read, mutate or delete another campaign's records.
- Admin role and CSRF checks cover every mutating route.
- Campaign deletion with wrong confirmation is rejected and leaves all data intact.
- Confirmed deletion removes only the selected campaign's invitations/greetings and revokes its participant sessions.
- Failed deletion can be retried safely.
- Existing campaign data and one-time invitation behavior remain intact after migration.

## Delivery sequence

1. **Foundation/migration:** schema, idempotent migration, store-level campaign model and migration tests.
2. **Backend API:** campaign CRUD, campaign-scoped invitation/participant/greeting flows, security tests.
3. **Admin UI:** campaign switcher, create/add invitations, invitation actions, confirmed permanent deletion.
4. **E2E and release:** Playwright, CI, production migration procedure, backup/rollback notes and VPS deployment.

Each stage is a separate PR. No PR is merged and no production migration is run without explicit user approval.
