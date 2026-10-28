import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

describe("应用更新 IPC", () => {
  beforeEach(() => {
    invoke.mockReset().mockResolvedValue({});
    Object.assign(globalThis, { window: { __TAURI_INTERNALS__: {} } });
  });

  it("状态、检查、下载和安装使用固定原生命令", async () => {
    const { getUpdateState, checkForUpdates, downloadUpdate, installUpdate } = await import(
      "../../app/src/app-api.js"
    );

    await getUpdateState();
    await checkForUpdates();
    await downloadUpdate();
    await installUpdate();

    expect(invoke).toHaveBeenNthCalledWith(1, "get_update_state");
    expect(invoke).toHaveBeenNthCalledWith(2, "check_for_updates");
    expect(invoke).toHaveBeenNthCalledWith(3, "download_update");
    expect(invoke).toHaveBeenNthCalledWith(4, "install_update");
  });
});
