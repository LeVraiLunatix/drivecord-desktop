# Drivecord Desktop

Client de synchronisation Windows pour [Drivecord](https://drivecord.app) —
façon kDrive : une fenêtre qui **est** l'app web Drivecord (même login, même
interface), plus une **synchro de dossier local** en arrière-plan.

> ⚠️ La synchro chiffre les fichiers **côté client**, comme les uploads du site
> (AES-256-GCM, clé de drive) : elle tourne dans le contexte JS du web, pas dans
> une API. Le natif ne fait que l'accès disque.

## Architecture cible

- **Coquille Tauri 2** : la fenêtre principale charge le front Drivecord
  (embarqué, client-only, API distante — voir `D1-b`). Tray + autostart +
  instance unique.
- **Webview de sync cachée** (Phase 2) : même origine → partage la session
  NextAuth + l'IndexedDB, reste vivante fenêtre fermée. Charge `/desktop-sync`.
- **Pont natif Rust** (Phase 3) : `pick_folder`, `watch` (+ événements),
  `read_file`, `write_file_atomic`, `list_dir`, `stat`, `mkdir`, `remove`,
  `move`. Exposé à la seule webview de sync.
- **Côté `discloud`** (repo web) : `isDesktopApp()`, page `/desktop-sync`,
  section réglages « Dossier synchronisé », moteur `src/lib/sync/` réutilisant
  `src/lib/crypto` + `src/lib/discord` + `src/lib/storage` + les routes
  `/api/drive/[id]/*` existantes. Aucune nouvelle route.

## Prérequis de dev

- Node ≥ 20
- Rust stable (`rustup`) + charge « Développement Desktop en C++ » de VS 2022
  (MSVC + Windows SDK). WebView2 : préinstallé Windows 11.

## Commandes

```bash
npm install
npm run app:dev      # Tauri + Vite (HMR)
npm run app:build    # build release → installeur NSIS
npm run typecheck
cargo test  --manifest-path src-tauri/Cargo.toml
```

## Feuille de route (Plan B v2)

| Phase | Repo | Contenu |
|---|---|---|
| 0 | desktop | ✅ Nettoyage du scaffold (retrait clé API / UI custom / client API Rust) |
| 1 | desktop + web | Coquille : front Drivecord embarqué client-only, API → `drivecord.app` |
| 2 | desktop | Webview de sync cachée, persistante en tray-only |
| 3 | desktop | Pont natif fichiers (`notify`, commandes disque) |
| 4 | web | `isDesktopApp()`, page `/desktop-sync`, réglages « Dossier synchronisé » |
| 5 | web | Sync montante : watcher → chiffrer + chunker + upload Discord + `POST .../files` |
| 6 | web | Sync descendante : poll `items` → `GET` + déchiffrer + écriture atomique |
| 7 | web | Résolution de conflits (`nom (conflit AAAA-MM-JJ).ext`) |
| 8 | desktop | Tray complet (pause, statut live) + autostart `--minimized` + réconciliation au boot |
| 9 | desktop | Google OAuth en webview (navigateur système + deep link) si bloqué |
| 10 | desktop | `cargo tauri build` → NSIS (CI sur tag `v*`) |
