# W1b frontend verification

2026-09-23. Browser fixture only: Aside, Vite localhost:1433, mocked Tauri invoke, synthetic localStorage. No native SQLite or real user storage used.

- Import preview shows namespace, indexes, titles, counts, valid native IDs, unmigrated count and invalid group exclusion.
- Apply initially disabled until explicit selection. Selecting index 0 sent only that group.
- First apply deliberately failed. Selection remained checked; second apply reused exactly the same idempotency key and succeeded.
- Original synthetic localStorage JSON remained byte-for-byte unchanged, including unmigrated note body and invalid group.
- Existing memory search, explicit radio selection and required reason produced expected workflow_link_add payload.
- Inspected browser-fixture.png viewport: readable labels, controls and status. It uses component CSS only, not full app theme. Full-page screenshot stitching was discarded.
- npm run build passed (existing GraphCanvas chunk-size warning).
- Four Vitest helper tests passed: corrupt storage, invalid group/unmigrated separation, selected groups only, request-key reuse across changed selections.

Not verified by this fixture: actual Core mutation, source lookup, restored-link persistence, native rendering. Backend tests and integration smoke cover their own boundaries separately.

Follow-up link UX check: enriched response displays current accessible title, provider/external ID and last collection time. Collapsed details made zero link requests; opening sent limit21/offset0, next sent offset20 and displayed remaining 2 of 22 fixture links with next disabled. Inline source preview sent context_get and displayed stale warning; no URL scheme was opened. Inspected links-fixture.png. SourceRef identity matches removed source links on the current page; all removed rows retain explicit restore control.

Final review: fixture now imports bare react and react-dom/client through Vite. A fresh direct Aside navigation rendered both components without evaluated render fallback. Source preview requires the selected SourceRef identity and accessible status together; unrelated one-hop context cannot substitute for a revoked root. Search and preview request max_age_seconds=86400. npm run build passed after these changes.
