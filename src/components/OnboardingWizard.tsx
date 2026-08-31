import { useState } from "react";
import {
  getConfig,
  pickSyncDir,
  setApiKey,
  setConfig,
  verifyKey,
} from "../lib/api";
import type { MeResponse } from "../lib/types";
import { DEFAULT_SERVER_URL } from "../lib/types";
import { normaliseServerUrl } from "../lib/format";

type Props = { onDone: () => void };

export function OnboardingWizard({ onDone }: Props) {
  const [serverUrl, setServerUrl] = useState(DEFAULT_SERVER_URL);
  const [apiKey, setApiKeyInput] = useState("");
  const [me, setMe] = useState<MeResponse | null>(null);
  const [syncDir, setSyncDir] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const canVerify = apiKey.trim().startsWith("dvc_") && !busy;
  const canFinish = !!me && !!syncDir && !busy;

  async function handleVerify() {
    setBusy(true);
    setError(null);
    setMe(null);
    try {
      const origin = normaliseServerUrl(serverUrl);
      setServerUrl(origin);
      const result = await verifyKey(origin, apiKey.trim());
      setMe(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function handlePickDir() {
    const dir = await pickSyncDir();
    if (dir) setSyncDir(dir);
  }

  async function handleFinish() {
    setBusy(true);
    setError(null);
    try {
      await setApiKey(apiKey.trim());
      const existing = await getConfig();
      await setConfig({
        serverUrl: normaliseServerUrl(serverUrl),
        syncDir,
        pollIntervalSecs: existing?.pollIntervalSecs ?? 60,
        excludes: existing?.excludes ?? ["**/.DS_Store", "**/Thumbs.db", "**/~$*"],
      });
      onDone();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  const writable = me?.scopes.includes("write");

  return (
    <div className="mx-auto flex h-full max-w-lg flex-col justify-center gap-6 px-8">
      <header>
        <h1 className="text-xl font-semibold">Connexion à Drivecord</h1>
        <p className="mt-1 text-sm text-text-dim">
          Colle une clé API générée dans{" "}
          <span className="text-text">Réglages → API pour développeurs</span> sur
          ton drive.
        </p>
      </header>

      <label className="flex flex-col gap-1.5 text-sm">
        <span className="text-text-dim">Serveur</span>
        <input
          className="rounded-lg border border-border bg-surface px-3 py-2 outline-none focus:border-accent"
          value={serverUrl}
          onChange={(e) => setServerUrl(e.target.value)}
          spellCheck={false}
          placeholder={DEFAULT_SERVER_URL}
        />
      </label>

      <label className="flex flex-col gap-1.5 text-sm">
        <span className="text-text-dim">Clé API</span>
        <div className="flex gap-2">
          <input
            className="flex-1 rounded-lg border border-border bg-surface px-3 py-2 font-mono text-xs outline-none focus:border-accent"
            value={apiKey}
            onChange={(e) => setApiKeyInput(e.target.value)}
            spellCheck={false}
            placeholder="dvc_…"
            type="password"
          />
          <button
            className="rounded-lg bg-accent px-4 py-2 font-medium text-white enabled:hover:bg-accent-hover disabled:opacity-40"
            onClick={handleVerify}
            disabled={!canVerify}
          >
            {busy && !me ? "…" : "Vérifier"}
          </button>
        </div>
      </label>

      {me && (
        <div className="rounded-lg border border-border bg-surface-2 px-3 py-2.5 text-sm">
          <div>
            Drive : <span className="font-medium">{me.drive}</span>
          </div>
          <div className="text-text-dim">
            Permissions : {me.scopes.join(" + ") || "aucune"}
          </div>
          {!writable && (
            <div className="mt-1 text-warn">
              Cette clé est en lecture seule — la synchro montante ne fonctionnera
              pas.
            </div>
          )}
        </div>
      )}

      {me && (
        <label className="flex flex-col gap-1.5 text-sm">
          <span className="text-text-dim">Dossier local à synchroniser</span>
          <div className="flex gap-2">
            <input
              className="flex-1 rounded-lg border border-border bg-surface px-3 py-2 text-xs outline-none"
              value={syncDir}
              readOnly
              placeholder="Aucun dossier choisi"
            />
            <button
              className="rounded-lg border border-border px-4 py-2 hover:bg-surface-2"
              onClick={handlePickDir}
            >
              Parcourir…
            </button>
          </div>
        </label>
      )}

      <div className="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2.5 text-xs text-warn">
        Les fichiers synchronisés via cette application ne sont{" "}
        <strong>pas chiffrés de bout en bout</strong>, contrairement à l'upload
        depuis le site : une clé API n'a pas accès à ta clé de chiffrement
        personnelle.
      </div>

      {error && <div className="text-sm text-danger">{error}</div>}

      <button
        className="rounded-lg bg-accent px-4 py-2.5 font-medium text-white enabled:hover:bg-accent-hover disabled:opacity-40"
        onClick={handleFinish}
        disabled={!canFinish}
      >
        Démarrer la synchronisation
      </button>
    </div>
  );
}
