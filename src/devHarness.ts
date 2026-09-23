// Env-gated dev automation — single entry from App.tsx after image-ready.
import { reportFrontendStatus } from "./ipc/commands";

export async function runDevHarness(imageVersion: number): Promise<void> {
  const { invoke } = await import("@tauri-apps/api/core");
  const scope = await invoke<string>("selftest_enabled");
  if (scope) {
    const { runSelfTest } = await import("./selftest");
    void runSelfTest(imageVersion, scope);
    return;
  }
  if (await invoke<boolean>("verify_slider_enabled").catch(() => false)) {
    const { verifySlider } = await import("./verifyslider");
    void verifySlider();
    return;
  }
  await reportFrontendStatus("dev-harness: idle").catch(() => {});
}
