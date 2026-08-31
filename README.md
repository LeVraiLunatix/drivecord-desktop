# Drivecord Desktop

Client de synchronisation Windows pour [Drivecord](https://drivecord.app) —
façon kDrive / Dropbox : tourne dans la zone de notification, synchronise un
dossier local avec un drive Drivecord via son **API publique** (`/api/v1`,
authentifiée par clé `dvc_…`).

> ⚠️ Les fichiers synchronisés par cette app **ne sont pas chiffrés de bout en
> bout**, contrairement à l'upload depuis le site : une clé API n'a pas accès à
> la clé de chiffrement personnelle du compte. Ne l'utilise pas pour des
> fichiers sensibles.

## Stack

- **Tauri 2** (backend Rust) + **React 19 / TypeScript / Vite** + **Tailwind v4**
- Rust : `reqwest` (HTTP vers l'API Drivecord, backoff sur `429`),
  `keyring` (clé API dans le Credential Manager Windows),
  `tauri-plugin-store` (config non secrète), tray, autostart, single-instance
- Le webview ne détient jamais la clé API ni ne fait de réseau : tout passe par
  des commandes Rust.

## Prérequis de dev

| Outil | Note |
|-------|------|
| Node ≥ 20 | frontend |
| **Rust (stable)** via [rustup](https://rustup.rs) | **non installé sur cette machine — à faire avant `npm run app:dev`** |
| MSVC Build Tools (C++) | `rustup` le réclame au 1er build sur Windows |
| WebView2 Runtime | préinstallé sur Windows 11 |

## Commandes

```bash
npm install
npm run app:dev      # Tauri + Vite en dev (HMR)
npm run app:build    # build release (installeur NSIS)
npm run typecheck    # tsc --noEmit
```

Côté Rust :

```bash
cargo test  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

## Architecture

```
src/                      frontend React
  lib/types.ts            miroir des types de l'API Drivecord
  lib/api.ts              wrappers typés autour de invoke()
  components/             OnboardingWizard, MainWindow (stubs B8)
src-tauri/src/
  api/                    client HTTP (reqwest) + modèles + retry 429
  config.rs               keyring (clé API) + store (config.json)
  commands.rs             commandes exposées au frontend
  tray.rs                 icône + menu de la zone de notification
  lib.rs                  wiring des plugins
```

## Feuille de route

- **B0–B2** ✅ scaffold, config sécurisée, client API + tests
- **B3** cache SQLite (mapping local ⇄ distant)
- **B4** onboarding (fait, à polir) — vérif clé, choix dossier, bandeau E2EE
- **B5** sync montante : watcher `notify`, upload simple/chunks
- **B6** sync descendante : poll `updatedSince` + `cursor`, écriture atomique
- **B7** résolution de conflits (`nom (conflit).ext`)
- **B8** fenêtre principale : explorateur, file de transferts, réglages
- **B9** tray complet (pause, statut live)
- **B10** autostart + reprise de file au lancement
- **V2** menu contextuel shell (lien public), throttle bande passante, multi-drive

## API Drivecord consommée

`GET /me` · `GET /files` (+ `recursive`, `updatedSince`, `cursor`) ·
`POST /files` (multipart & JSON) · `POST /files/chunks` ·
`GET /files/:id/download` · `DELETE /files/:id` ·
`POST|DELETE /files/:id/public` ·
`GET|POST /folders` · `GET|DELETE /folders/:id`

Doc : <https://drivecord.app/docs/technique/api>
