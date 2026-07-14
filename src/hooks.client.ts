import { mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { handleMockCommand } from "@e2e/fixtures/commands";

// Browser E2E runs Vite without the Tauri shell. Register mock IPC before any
// route component invokes a command (see docs/testing/e2e.md).
if (import.meta.env.VITE_E2E_MOCK) {
  mockIPC((cmd, args) => handleMockCommand(cmd, args ?? {}), {
    shouldMockEvents: true,
  });
  mockConvertFileSrc("windows");
}
