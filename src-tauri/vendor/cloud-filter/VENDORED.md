# cloud-filter 0.0.6 — copie corrigée

Copie de [cloud-filter](https://github.com/ho-229/cloud-filter-rs) 0.0.6 (MIT,
voir `LICENSE`), branchée via `[patch.crates-io]` dans `src-tauri/Cargo.toml`.

Seul changement : `src/filter/proxy.rs` et `src/command/commands.rs`.

- Les callbacks CfAPI (`fetch_data`, `validate_data`, `fetch_placeholders`,
  `dehydrate`, `delete`, `rename`) faisaient `.unwrap()` sur le `CfExecute`
  qui signale un échec à Windows. Ce `CfExecute` peut lui-même échouer (requête
  annulée entre-temps…) → panique dans un `extern "system"` → **l'app entière
  plantait** (STATUS_STACK_BUFFER_OVERRUN). Les erreurs sont désormais ignorées.
- `fetch_data` signalait l'échec avec une plage de longueur 0, que Windows
  refuse (`ERROR_CLOUD_FILE_INVALID_REQUEST`, 0x8007017C) : l'échec est
  maintenant signalé sur la plage demandée (`Write::fail_range`).
