# Flux — Desktop App Implementation Plan

Flux is a Tauri desktop app (Mac + Windows) that replaces the dist repo shell scripts
as the customer distribution method. "Flux" is a placeholder name — final name TBD,
stored as a single variable in `tauri.conf.json`.

## Repos
- **Main farm repo:** `kennydead/claude-agent-farm` — branch `feature/flux-app`
- **Dist repo (Tauri app):** `kennydead/claude-agent-farm-dist` — branch `feature/flux-app`
- Merge both to `main` simultaneously when v1 is ready to ship

---

## Phase 1: Repo & Tooling Setup
- [ ] Initialize Tauri + React project in dist repo (`feature/flux-app` branch)
- [ ] Verify Rust + Tauri CLI installed, confirm Mac build works locally
- [ ] Set up GitHub Actions in dist repo: Mac runner (dmg) + Windows runner (nsis)
- [ ] Confirm dist repo GitHub Releases as the artifact destination

## Phase 2: App Shell
- [ ] Tauri config: app name "Flux", window 1200×800, resizable, system tray enabled
- [ ] Inter font bundled, global CSS design token: `--accent: #4F8EF7`, `--accent-end: #7C5CFC`
- [ ] Placeholder app icon — stylized "F" in accent gradient (all required sizes)
- [ ] Basic React router: `setup` flow screens + `running` state (navigates to localhost:5174)

## Phase 3: First-Run Flow
- [ ] **License screen** — text input, calls license server, stores key to `logs/license_key.txt`
- [ ] **Docker check screen** — polls every 3s, shows download link if not found, auto-advances when Docker comes up
- [ ] **AI Provider screen** — Claude selected, OpenAI/Gemini greyed out as "coming soon"
- [ ] **Claude auth screen** — runs `docker run -i claude auth login`, captures URL from stdout, shows "Open in browser" button, accepts paste-back code in text field, confirms success

## Phase 4: Farm File Bundling & Startup
- [ ] Bundle `docker-compose.yml`, `host_bridge.py` as Tauri resources
- [ ] On first run: extract bundled files to `~/farm/` (or user-chosen directory)
- [ ] **Startup sequence** — animated vertical stepper:
  - Docker found ✓
  - License verified ✓
  - Pulling latest images → (longest step)
  - Starting services →
  - Ready ✓
- [ ] Start `host_bridge.py` as a subprocess when farm starts
- [ ] Navigate Tauri window to `http://localhost:5174` when dashboard is healthy

## Phase 5: System Tray & Lifecycle
- [ ] System tray icon (same "F" placeholder, replace with final icon later)
- [ ] Tray menu: Open Dashboard, Stop Farm, Update, Quit
- [ ] Close window → minimize to tray (farm keeps running)
- [ ] Quit → confirmation dialog → `docker compose down` → exit
- [ ] **Auto-start on login** — checkbox on first-run completion screen, default checked

## Phase 6: Updates
- [ ] Tauri updater pointing to dist repo GitHub Releases JSON endpoint
- [ ] "Update available — restart to install" prompt in tray/app
- [ ] Docker images pulled fresh on every farm start (already handled by start.sh logic)

## Phase 7: CI/CD
- [ ] Dist repo GitHub Actions: build Mac `.dmg` (universal: arm64 + x86_64) on `main` push
- [ ] Dist repo GitHub Actions: build Windows `.exe` (NSIS installer) on `main` push
- [ ] Publish both artifacts to GitHub Release with version tag
- [ ] Update main repo `release.yml` to trigger dist repo Tauri build after Docker images are pushed
- [ ] Tauri updater JSON endpoint auto-generated from releases

## Phase 8: Polish
- [ ] Stepper animation: pending (dim) → active (accent pulse glow) → done (checkmark fade-in)
- [ ] Error states: Docker not found, license invalid, auth failed, docker pull failed
- [ ] Smooth transition from startup stepper to dashboard (window content swap)
- [ ] Final app icon (placeholder until designer produces asset)

---

## v2 Backlog (do not scope into v1)
- Additional AI providers (OpenAI/Codex, Gemini)
- Light mode
- Reset / advanced controls in tray
- Crash recovery / watchdog
- Code signing (Mac Developer ID + Windows EV cert)
- Private Docker registry gated by license (real IP protection)

---

## Design Tokens
```css
--accent:        #4F8EF7;
--accent-end:    #7C5CFC;
--bg:            #0D0D0F;
--surface:       #161618;
--surface-raised: #1E1E21;
--text:          #F0F0F2;
--text-muted:    #6B6B7A;
--success:       #34D399;
--error:         #F87171;
```
Font: **Inter** (bundled)
Window: **1200×800**, resizable

## Current Status
> Starting Phase 1 — repo and tooling setup
