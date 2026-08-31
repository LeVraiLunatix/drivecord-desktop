import { useCallback, useEffect, useState } from "react";
import { getConfig, hasApiKey } from "./lib/api";
import type { AppConfig } from "./lib/types";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { MainWindow } from "./components/MainWindow";

type Boot =
  | { state: "loading" }
  | { state: "onboarding" }
  | { state: "ready"; config: AppConfig };

export default function App() {
  const [boot, setBoot] = useState<Boot>({ state: "loading" });

  const refresh = useCallback(async () => {
    const [config, keyed] = await Promise.all([getConfig(), hasApiKey()]);
    if (config && keyed) setBoot({ state: "ready", config });
    else setBoot({ state: "onboarding" });
  }, []);

  useEffect(() => {
    refresh().catch(() => setBoot({ state: "onboarding" }));
  }, [refresh]);

  if (boot.state === "loading") {
    return (
      <div className="grid h-full place-items-center text-sm text-text-dim">
        Chargement…
      </div>
    );
  }

  if (boot.state === "onboarding") {
    return <OnboardingWizard onDone={refresh} />;
  }

  return <MainWindow config={boot.config} onReconfigure={refresh} />;
}
