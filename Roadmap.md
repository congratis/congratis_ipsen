# Congratis — Development Roadmap

**Current implementation branch:** `feat/dominator-spa`  
**Frontend stack:** Rust, Dominator, WebAssembly, Rollup  
**Backend stack:** Rust, Axum, RethinkDB 2.4.4, Docker Compose, Nginx  
**Deployment target:** Ubuntu 26.04 LTS (Resolute)

This roadmap describes the planned product and implementation sequence. It is a plan, not a statement that every item is already implemented.

## Product scope

Congratis is an interactive New Year greeting experience. An organizer creates a campaign, invites participants through individual links, and participants submit one greeting each. The campaign page presents the participants as ornaments on a Christmas tree. Visitors can watch the submitted greetings as a festive presentation.

## 1. Frontend audit and stabilization

- Audit the existing Dominator application, Rust/WASM lifecycle, state management, API client, routing, styles, and Rollup build.
- Verify loading, empty, success, and error states for every existing screen.
- Review session handling, invitation exchange, CSRF protection, and duplicate-submission behavior from the frontend.
- Verify SSE connection lifecycle, reconnection, stale data handling, and cleanup.
- Establish reusable UI primitives and a consistent responsive layout.
- Preserve the existing Rust + Dominator + WASM + Rollup toolchain. Do not introduce Trunk.

**Acceptance:** current functionality is documented, existing tests pass, and the next changes can be made without destabilizing the working build.

## 2. Configurable campaign size: 1–35 participants

Replace the fixed 20-participant assumption with a campaign-level participant count configurable from **1 through 35 inclusive**.

- Let the organizer choose the number of participants when creating/configuring a campaign.
- Validate the range on both backend and frontend; reject values outside 1–35.
- Persist the configured count in campaign data and expose it through the API.
- Generate exactly the configured number of participant invitations/ornament slots.
- Render the tree dynamically and distribute ornaments into a balanced composition for any supported count.
- Ensure invitation administration, progress calculations, presentation ordering, and tests use the campaign count rather than a hard-coded constant.
- Define behavior when the organizer changes the count after invitations or greetings already exist; never silently discard participant data. Any reduction that would remove occupied slots must be rejected or require an explicit safe resolution.

**Acceptance:** campaigns with 1, 2, 20, and 35 participants work end-to-end; invalid counts are rejected; no UI or backend path assumes 20 participants.

## 3. Christmas tree and participant interaction

- Build the illustrated tree as the main campaign view.
- Render one interactive ornament per participant slot.
- Distinguish available, invited/in-progress, and submitted states visually and accessibly.
- Associate each ornament with its invitation/participant identity and greeting.
- Support desktop and mobile layouts without overlap or unusably small targets.
- Animate selection and state transitions with reduced-motion preferences respected.

**Acceptance:** every configured participant is represented exactly once, and ornament state matches persisted backend state.

## 4. Invitation and greeting submission flow

- Open an invitation link and exchange it for a participant session.
- Allow the participant to select their assigned ornament and compose a greeting.
- Submit the greeting exactly once; prevent duplicate submissions server-side as well as in the UI.
- Preserve an in-progress draft across recoverable network failures where practical.
- Provide clear expired/invalid invitation, session, validation, and network error states.
- After submission, update the ornament and campaign progress through the normal API/SSE flow.

**Acceptance:** the complete invitation-to-submission flow is covered by end-to-end tests, including duplicate and expired-session cases.

## 5. Greeting viewer

- Show a submitted greeting in a dedicated card/modal or presentation view.
- Display the participant identity and full greeting text with readable typography.
- Support long messages and narrow mobile screens.
- Provide clear navigation between greetings where appropriate.
- Keep presentation data ordered deterministically.

**Acceptance:** greetings remain readable and navigable on desktop and mobile, including long text.

## 6. Candy-cane progress bar

Add a festive progress indicator in the footer area of the Christmas-tree page, positioned as an overlay on the background illustration.

- Style the progress track as a red-and-white striped Christmas candy cane, not a generic rectangular progress bar.
- Calculate progress from authoritative campaign data: submitted greetings divided by configured participant count.
- Animate the candy-cane fill/advance whenever a new greeting is successfully submitted.
- Update the displayed count and progress in real time using the existing SSE mechanism, with API refresh/reconciliation after reconnect.
- Handle zero submissions, full completion, and all supported campaign sizes (1–35).
- Keep the indicator legible and correctly positioned on mobile and desktop.

**Acceptance:** progress always equals the persisted submitted-greeting count / campaign participant count and updates live without a page reload.

## 7. New Year presentation mode and Jingle Bells

Add a dedicated presentation mode launched by a **Play** button placed beside the candy-cane progress indicator.

### Playback experience

- Clicking Play enters a focused, festive presentation mode.
- Start playback of a suitable licensed/public-domain recording of **“Jingle Bells”** as a user-initiated action.
- Present submitted greetings one by one, with the corresponding ornament highlighted and a transition into the greeting.
- Use polished transitions and festive visual effects while keeping the message itself readable.
- After the final greeting, show a closing/finale scene and offer replay or return to the tree.

### Controls

- Play and pause/resume.
- Previous and next greeting.
- Music mute/unmute and volume control.
- Replay from the beginning.
- Exit presentation and return to the campaign tree.

### Audio and browser constraints

- Do not rely on autoplay: browsers may block audio until a user gesture. Start audio in the Play click flow and handle playback rejection gracefully.
- The composition “Jingle Bells” is public domain in many jurisdictions, but a particular sound recording may still be copyrighted. Use only a recording whose license permits this use, and record its source/license in the repository.
- Provide a graceful silent mode if audio is unavailable or blocked.
- Respect reduced-motion preferences and avoid making audio start unexpectedly outside the explicit Play action.

**Acceptance:** Play starts the presentation and attempts audio playback; all controls work; audio failure does not prevent viewing greetings; the presentation handles zero, one, and many submitted greetings.

## 8. Administration UI

- Admin authentication and session handling.
- Campaign overview with configured participant count and submission progress.
- Participant/invitation list with statuses.
- Create/reissue/revoke invitations as supported by backend policy.
- Copy invitation links.
- Inspect submitted greetings.
- Receive live updates through SSE.
- Clearly distinguish destructive actions and confirm them where necessary.

**Acceptance:** administrators can manage the campaign without direct database access, and admin actions are authorized and tested.

## 9. Visual polish, accessibility, and performance

- Establish typography, colors, spacing, reusable components, and illustration/animation conventions.
- Refine tree, ornaments, candy cane, presentation transitions, and finale.
- Ensure responsive behavior across common desktop, tablet, and mobile sizes.
- Add keyboard navigation, visible focus, semantic labels, and sufficient contrast.
- Respect reduced-motion settings; provide audio controls and never force sound.
- Optimize image/audio assets and WASM bundle loading; avoid unnecessary rerenders.

**Acceptance:** the experience is usable on mobile and desktop, accessible for core flows, and performs within agreed bundle/loading budgets.

## 10. Quality assurance and production readiness

- Add/maintain unit and integration tests for campaign size validation, participant allocation, progress calculation, and presentation sequencing.
- Add Playwright end-to-end coverage for invitation, submission, duplicate prevention, admin workflows, SSE updates/reconnect, and presentation controls.
- Test campaign sizes 1, 2, 20, and 35.
- Test empty, partial, and complete campaigns; network interruption; expired sessions; and audio playback denial.
- Verify Rollup production build, Docker image, Nginx routing/static asset caching, and the full production Compose smoke test.
- Keep CI checks required and green before merging/deploying.

**Acceptance:** required CI and production smoke tests pass, and the documented deployment procedure works on Ubuntu 26.04 LTS (Resolute).

## Suggested implementation order

1. Complete the frontend audit and stabilize the current baseline.
2. Implement campaign participant count (1–35) across backend, API, and frontend.
3. Build the dynamic tree and ornament states.
4. Complete invitation and greeting submission UX.
5. Build the greeting viewer.
6. Add candy-cane progress and live SSE updates.
7. Implement the presentation mode, controls, and licensed Jingle Bells audio.
8. Complete the administration UI.
9. Polish accessibility, responsive design, animations, and performance.
10. Expand E2E coverage and verify the production deployment.

## Architecture principles

- Backend data is authoritative for participant count, invitation status, submitted greetings, and progress.
- Frontend components render state and dispatch actions; they do not invent campaign state or bypass API authorization.
- SSE is a notification/update channel; reconnects must reconcile with authoritative API state.
- Enforce one greeting per invitation/participant on the backend, not only by disabling UI controls.
- Keep presentation mode separate from ordinary tree interaction, reusing the same campaign and greeting data.
- Keep the existing Rust + Dominator + WebAssembly + Rollup build pipeline.
