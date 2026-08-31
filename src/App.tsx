/**
 * Placeholder shell. Phase 1 (D1-b) replaces this with the embedded, client-only
 * Drivecord frontend pointed at the remote API; Phase 2 adds the hidden sync
 * webview. For now it just proves the shell + tray + autostart boot.
 */
export default function App() {
  return (
    <div className="grid h-full place-items-center px-8 text-center">
      <div>
        <h1 className="text-lg font-semibold">Drivecord Desktop</h1>
        <p className="mt-2 text-sm text-text-dim">
          Coquille en cours de construction — Phase&nbsp;0.
        </p>
      </div>
    </div>
  );
}
