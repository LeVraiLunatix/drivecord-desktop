/**
 * Types mirroring Drivecord's public API (`/api/v1`).
 * Keep in sync with `discloud/src/lib/storage/schema.ts` and
 * `discloud/src/lib/discord/types.ts`.
 */

export type ApiScope = "read" | "write";

export interface MeResponse {
  drive: string;
  driveId: string;
  scopes: ApiScope[];
}

/** One Discord message reference — a slice of a file. */
export interface ChunkRef {
  index: number;
  size: number;
  messageId: string;
  attachmentId: string;
  url: string;
  /** Unix ms when the signed CDN `url` expires (0 if unknown). */
  expiresAt: number;
}

export interface FileEntry {
  id: string;
  driveId: string;
  /** Parent folder id, or "" for the drive root. */
  parentId: string;
  filename: string;
  size: number;
  mimeType: string;
  chunkSize: number;
  chunks: ChunkRef[];
  /** Unix ms. */
  createdAt: number;
  /** Unix ms. */
  updatedAt: number;
  tags: string[];
  favorite: boolean;
  locked: boolean;
  trashed: boolean;
  trashedAt?: number;
  encIv?: string;
}

export interface FolderEntry {
  id: string;
  driveId: string;
  parentId: string;
  name: string;
  createdAt: number;
  updatedAt: number;
  trashed: boolean;
  trashedAt?: number;
  color?: string;
}

/** Response of `GET /api/v1/files` in sync mode (recursive / updatedSince / cursor). */
export interface FilesPage {
  files: FileEntry[];
  nextCursor: string | null;
}

// ── Local app configuration (mirrors `src-tauri/src/config.rs`) ───────────────

export interface AppConfig {
  /** Base origin of the Drivecord server, no trailing slash. */
  serverUrl: string;
  /** Absolute path of the local folder kept in sync. */
  syncDir: string;
  /** Seconds between down-sync polls. */
  pollIntervalSecs: number;
  /** `.gitignore`-style patterns excluded from sync. */
  excludes: string[];
}

export const DEFAULT_SERVER_URL = "https://drivecord.app";
