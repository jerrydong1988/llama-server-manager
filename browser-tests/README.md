# Browser regression backend

Run `npx playwright install chromium` once, then `npm run test:browser` to execute the browser regression suite. Playwright starts the frontend on `http://127.0.0.1:4173` with a deterministic Tauri IPC backend. `npm run dev:browser-test` remains available for interactive inspection.

The mock is injected only by `vite.browser-test.config.ts` while serving. The production entry and `vite.config.ts` do not import it, the test config rejects build commands, and `npm run build` scans `dist` for mock markers.

The page exposes `window.__TAURI_BROWSER_TEST__` only in this environment. Browser automation inspects `calls`, `unhandled`, `saveCount`, `lastGenerated`, and the persisted fixture state without modifying application production code. The release check runs the Chromium suite, and CI installs the matching Playwright browser before entering that gate.
