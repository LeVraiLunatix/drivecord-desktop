/**
 * Thin typed wrappers over the Rust commands (`src-tauri/src/commands.rs`).
 * The actual HTTP to Drivecord happens in Rust (`reqwest`); the webview never
 * holds the API key or talks to the network directly.
 */
import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, MeResponse } from "./types";

/** Verify a server URL + API key against `GET /api/v1/me`. Does not persist anything. */
export function verifyKey(serverUrl: string, apiKey: string): Promise<MeResponse> {
  return invoke<MeResponse>("verify_key", { serverUrl, apiKey });
}

/** Persist the API key in the OS keychain (Windows Credential Manager). */
export function setApiKey(apiKey: string): Promise<void> {
  return invoke("set_api_key", { apiKey });
}

export function hasApiKey(): Promise<boolean> {
  return invoke<boolean>("has_api_key");
}

export function clearApiKey(): Promise<void> {
  return invoke("clear_api_key");
}

/** Load the saved config, or `null` on first run. */
export function getConfig(): Promise<AppConfig | null> {
  return invoke<AppConfig | null>("get_config");
}

export function setConfig(config: AppConfig): Promise<void> {
  return invoke("set_config", { config });
}

/** Open the native folder picker. Resolves to the chosen path, or `null` if cancelled. */
export function pickSyncDir(): Promise<string | null> {
  return invoke<string | null>("pick_sync_dir");
}
