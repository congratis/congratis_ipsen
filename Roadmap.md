# Congratis — Development Roadmap

**Current implementation branch:** `main`  
**Frontend stack:** Rust, Dominator, WebAssembly, Rollup  
**Backend stack:** Rust, Axum, RethinkDB 2.4.4, Docker Compose, Nginx  
**Deployment target:** Ubuntu 26.04 LTS (Resolute)  
**Production domain:** `congratis.online`

This roadmap records product scope and verified implementation status. Green CI proves automated repository checks; it does not by itself prove that every live user workflow has been exercised.

## Product scope

Congratis is an interactive New Year greeting experience. An organizer creates a campaign, invites participants through individual one-time links, and participants submit one greeting each. The campaign page presents participants as ornaments on a Christmas tree. Visitors can watch submitted greetings as a festive presentation.

## Verified implementation status

| Area | Status | Evidence / remaining work |
|---|---|---|
| Rust/Dominator frontend and Rollup build | Implemented | WASM compile, Rollup build and Playwright in CI |
| Campaign size 1–35 | Implemented | Backend validation, campaign API, dynamic frontend and E2E sizes 1, 2, 20, 35 |
| Dynamic tree and ornament states | Implemented | API-backed states, responsive layout and E2E checks |
| Invitation and greeting submission | Implemented | One-time exchange, backend duplicate protection, validation and network error display |
| Draft recovery | Implemented | Browser localStorage, scoped by participant, cleared only after successful submission |
| Greeting viewer | Implemented | Dialog semantics, long-message layout and deterministic navigation |
| Candy-cane progress and SSE | Implemented | API snapshot, reconnect reconciliation and live updates |
| Presentation and Jingle Bells | Implemented | Controls, silent fallback, reduced-motion handling and attribution |
| Administration UI | Implemented | Login, campaign creation, participant list, reissue and confirmed revoke |
| Dialog keyboard accessibility | Implemented | PR #14 merged: initial focus, Tab/Shift+Tab trap, Escape dismissal and focus restoration; Playwright coverage |
| Production CI validation | Passed | Full Docker stack, Nginx bootstrap/TLS checks and real backend HTTP smoke test |
| Production DNS and TLS | Verified | Public Cloudflare DNS resolves apex A and www CNAME to VPS; external HTTPS requests return HTTP 200 and serve the GlobalSign certificate |
| Live VPS infrastructure | Verified | Ubuntu 26.04 VPS; app, RethinkDB and Nginx running; Nginx/RethinkDB healthy; Compose health and Nginx config checks pass |
| Live application workflows | Pending | Still need operator-led admin login and safe verification of the active campaign, participant invitation and greeting flows |
| SSE in production | Pending | Verify live updates and reconnect behavior against the active campaign without injecting test data |
| Production backup and restore | Pending | Confirm backup procedure, secure storage and a restore rehearsal against an isolated instance |

## 1. Frontend audit and stabilization

- Audit Dominator, Rust/WASM lifecycle, state management, API client, routing, styles and Rollup build.
- Verify loading, empty, success and error states.
- Review session handling, invitation exchange, CSRF protection and duplicate submission.
- Verify SSE lifecycle, reconnection, stale-data handling and cleanup.
- Preserve Rust + Dominator + WASM + Rollup. Do not introduce Trunk.

**Acceptance:** existing functionality is documented, automated tests pass, and changes do not destabilize the build.

## 2. Configurable campaign size: 1–35 participants

- Organizer selects the participant count when creating a campaign.
- Backend and frontend validate 1–35 inclusive.
- Generate exactly the configured number of invitations and ornament slots.
- Never silently discard participant data; campaign creation refuses to overwrite a non-empty campaign.

**Acceptance:** campaigns with 1, 2, 20 and 35 participants work end-to-end; invalid counts are rejected.

## 3. Christmas tree and participant interaction

- Illustrated tree with one interactive ornament per participant slot.
- Distinguish available, invited/in-progress and submitted states.
- Support desktop/mobile layouts and reduced-motion preferences.

**Acceptance:** every configured participant is represented exactly once and ornament state matches persisted backend state.

## 4. Invitation and greeting submission flow

- Exchange invitation links for participant sessions.
- Compose and submit one greeting; enforce uniqueness on the backend.
- Preserve drafts across reloads and recoverable network failures.
- Show expired/invalid invitation, session, validation and network errors.
- Update tree and progress through API/SSE.

**Acceptance:** invitation-to-submission flow and duplicate/expired cases are covered by browser tests.

## 5. Greeting viewer

- Dedicated greeting dialog with participant identity and full message.
- Long messages and narrow mobile screens remain readable.
- Deterministic previous/next navigation and semantic dialog labels.

**Acceptance:** greetings remain readable and navigable on desktop and mobile.

## 6. Candy-cane progress bar

- Festive red-and-white progress indicator.
- Calculate from submitted greetings / configured participant count.
- Update through SSE and reconcile from API after reconnect.
- Handle empty, complete and all supported campaign sizes.

**Acceptance:** progress matches persisted backend state without a page reload.

## 7. New Year presentation mode and Jingle Bells

- Dedicated presentation launched by Play, with user-initiated licensed/public-domain recording.
- Sequential greeting display and ornament highlighting.
- Play/pause, previous/next, mute, volume, replay and exit controls.
- Graceful silent fallback, reduced-motion support and source/license attribution.

**Acceptance:** audio failure never blocks viewing greetings; presentation handles empty, one and many greetings.

## 8. Administration UI

- Admin authentication and session handling.
- Create campaign and one-time invitations.
- View participant/invitation statuses.
- Reissue or revoke eligible invitations; confirm destructive actions.
- Copy one-time links and receive live updates.

**Acceptance:** administration requires no direct database access; admin actions are authorized and tested.

## 9. Visual polish, accessibility and performance

- Responsive layout and consistent visual system.
- Keyboard-operable controls, visible focus and semantic labels.
- Escape dismissal, focus trap and focus restoration for modal dialogs.
- Sufficient contrast, reduced-motion support and explicit audio controls.
- Optimize images/audio and WASM loading.

**Acceptance:** core flows are usable on mobile and desktop, including keyboard-only interaction.

## 10. Quality assurance and production readiness

- Unit/integration tests for campaign size, participant allocation, progress and presentation sequencing.
- Playwright coverage for invitations, submission, duplicate prevention, admin workflows, SSE reconnect and presentation controls.
- Test empty, partial and complete campaigns, network interruption, expired sessions and audio denial.
- Verify Rollup build, Docker image, Nginx routing/static caching and full production Compose smoke test.
- Keep required CI green before merging.
- Verify production HTTPS, health, administrator access, campaign data, invitation flow, greeting submission, SSE and persistence.
- Back up production data and rehearse restore against an isolated instance.

**Acceptance:** CI and production infrastructure checks pass; authenticated workflows and backup/restore are verified without risking production campaign data.

## Next execution sequence

1. Inspect the live campaign and participant state through read-only/admin UI access; do not expose raw invitation tokens.
2. Verify administrator login and inspect campaign status without changing production data.
3. Verify participant invitation and greeting flows using an isolated disposable environment, not the live campaign.
4. Verify production SSE updates/reconnect without injecting test greetings.
5. Establish a production RethinkDB backup and securely store the backup and required configuration.
6. Rehearse restore against an isolated temporary RethinkDB instance.
7. Update this Roadmap again only after each acceptance check has evidence.

## Architecture principles

- Backend data is authoritative for participant count, invitation status, greetings and progress.
- Frontend renders state and dispatches authorized API actions.
- SSE is a notification channel; reconnects reconcile against authoritative API state.
- Enforce one greeting per invitation/participant on the backend.
- Keep presentation mode separate from ordinary tree interaction.
- Keep Rust + Dominator + WebAssembly + Rollup as the frontend build pipeline.
