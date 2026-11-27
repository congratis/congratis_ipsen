# Congratis — Development Roadmap

**Current implementation branch:** `main`  
**Frontend stack:** Rust, Dominator, WebAssembly, Rollup  
**Backend stack:** Rust, Axum, RethinkDB 2.4.4, Docker Compose, Nginx  
**Deployment target:** Ubuntu 26.04 LTS (Resolute)  
**Production domain:** `congratis.online`

This roadmap records product scope and verified implementation status. Green CI proves automated repository checks; it does not by itself prove that the live VPS deployment is complete.

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
| Administration UI | Implemented | Login, named campaign creation, campaign list, participant management, invitation add/reissue/revoke and exact-name deletion |
| Multi-campaign isolation | Implemented | Campaign-scoped invitations, greetings, participant sessions and SSE; RethinkDB integration test verifies cross-campaign isolation and deletion |
| Dialog keyboard accessibility | Implemented | Native modal focus trap, initial focus, Escape/close, focus restoration and draft preservation covered by Playwright |
| Production CI validation | Passed on current main baseline | Full Docker stack, Nginx bootstrap/TLS checks and real backend HTTP smoke test |
| Live VPS deployment | Not yet verified | Deliberately not executed during repository development; requires separate authorization for production operations |

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
- Create named campaigns and one-time invitations.
- View campaigns and campaign-scoped participant/invitation statuses.
- Add invitations to an existing campaign.
- Reissue or revoke eligible invitations; confirm destructive actions.
- Permanently delete a campaign only after exact-name confirmation; never delete legacy-default.
- Copy one-time links and receive live updates.

**Acceptance:** administration requires no direct database access; admin actions are authorized, CSRF-protected and tested.

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
- Keep required CI green before merging/deploying.
- Deploy to the Ubuntu 26.04 LTS VPS, verify HTTPS and live API flows, and confirm persistent data and backups.

**Acceptance:** CI and production smoke tests pass, and the documented deployment procedure works on the live VPS.

## Next execution sequence

1. Review the multi-campaign implementation and its integration coverage on main.
2. Keep deployment documentation aligned with the current Compose, Nginx and TLS configuration.
3. Treat live VPS inspection, deployment and production smoke tests as a separate operational task requiring explicit authorization.
4. Do not modify production services, secrets, volumes or persistent RethinkDB data as part of repository-only development.
5. Mark live deployment complete only after separately authorized HTTPS, health, administrator login, campaign creation, invitation flow, greeting submission, SSE and persistence checks succeed.

## Separate technical-debt task

- [ ] Investigate and fix the `wasm-opt: Fatal: error parsing wasm` warning emitted during the WASM/Rollup build. The build currently continues without WASM optimization; track this independently from browser-test failures and do not treat it as their root cause.

## Architecture principles

- Backend data is authoritative for participant count, invitation status, greetings and progress.
- Frontend renders state and dispatches authorized API actions.
- SSE is a notification channel; reconnects reconcile against authoritative API state.
- Enforce one greeting per invitation/participant on the backend.
- Keep presentation mode separate from ordinary tree interaction.
- Keep Rust + Dominator + WebAssembly + Rollup as the frontend build pipeline.