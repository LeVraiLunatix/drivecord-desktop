import { useState } from "react";
import { clearApiKey } from "../lib/api";
import type { AppConfig } from "../lib/types";

type Props = { config: AppConfig; onReconfigure: () => void };

type Tab = "files" | "transfers" | "settings";

export function MainWindow({ config, onReconfigure }: Props) {
  const [tab, setTab] = useState<Tab>("files");

  async function handleUnlink() {
    await clearApiKey();
    onReconfigure();
  }

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center gap-4 border-b border-border px-4 py-2.5">
        <span className="font-semibold">Drivecord</span>
        <nav className="flex gap-1 text-sm">
          <TabButton active={tab === "files"} onClick={() => setTab("files")}>
            Fichiers
          </TabButton>
          <TabButton
            active={tab === "transfers"}
            onClick={() => setTab("transfers")}
          >
            Transferts
          </TabButton>
          <TabButton
            active={tab === "settings"}
            onClick={() => setTab("settings")}
          >
            Réglages
          </TabButton>
        </nav>
        <span className="ml-auto text-xs text-text-dim">
          {config.serverUrl.replace(/^https?:\/\//, "")}
        </span>
      </header>

      <main className="min-h-0 flex-1 overflow-auto p-4 text-sm">
        {tab === "files" && (
          <Placeholder title="Explorateur de fichiers">
            L'arborescence du drive (via <code>parentId</code>) s'affichera ici —
            étape B8.
          </Placeholder>
        )}
        {tab === "transfers" && (
          <Placeholder title="File de transferts">
            Les uploads / downloads en cours et l'historique — étape B8.
          </Placeholder>
        )}
        {tab === "settings" && (
          <div className="flex flex-col gap-4">
            <Row label="Dossier synchronisé" value={config.syncDir} />
            <Row
              label="Intervalle de poll"
              value={`${config.pollIntervalSecs} s`}
            />
            <Row
              label="Exclusions"
              value={config.excludes.join(", ") || "aucune"}
            />
            <button
              className="mt-2 w-fit rounded-lg border border-danger/50 px-3 py-1.5 text-danger hover:bg-danger/10"
              onClick={handleUnlink}
            >
              Déconnecter ce drive
            </button>
          </div>
        )}
      </main>
    </div>
  );
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={
        "rounded-md px-2.5 py-1 " +
        (active ? "bg-surface-2 text-text" : "text-text-dim hover:text-text")
      }
    >
      {children}
    </button>
  );
}

function Placeholder({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-dashed border-border p-6 text-text-dim">
      <div className="mb-1 font-medium text-text">{title}</div>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <span className="text-xs text-text-dim">{label}</span>
      <span className="break-all">{value}</span>
    </div>
  );
}
