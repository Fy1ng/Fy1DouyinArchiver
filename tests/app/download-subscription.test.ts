import { describe, expect, test, vi } from "vitest";
import { installDownloadRefreshSubscription } from "../../app/src/download/subscription.js";

describe("download refresh subscription", () => {
  test("unregisters a listener that resolves after disposal", async () => {
    let resolveRegistration!: (unlisten: () => void) => void;
    const unlisten = vi.fn();
    const refresh = vi.fn();
    const registration = new Promise<() => void>((resolve) => {
      resolveRegistration = resolve;
    });

    const dispose = installDownloadRefreshSubscription(() => registration, refresh);
    dispose();
    resolveRegistration(unlisten);
    await registration;
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledOnce();
    expect(refresh).not.toHaveBeenCalled();
  });

  test("ignores refresh callbacks after disposal", async () => {
    let callback!: () => void;
    const refresh = vi.fn();
    const dispose = installDownloadRefreshSubscription(async (next) => {
      callback = next;
      return vi.fn<() => void>();
    }, refresh);
    await Promise.resolve();

    callback();
    expect(refresh).toHaveBeenCalledOnce();
    dispose();
    callback();
    expect(refresh).toHaveBeenCalledOnce();
  });

  test("contains listener registration rejection without an unhandled promise", async () => {
    const unhandled = vi.fn();
    process.once("unhandledRejection", unhandled);
    const dispose = installDownloadRefreshSubscription(
      async () => Promise.reject(new Error("listener registration failed")),
      vi.fn(),
    );

    await new Promise((resolve) => setTimeout(resolve, 0));
    dispose();
    process.removeListener("unhandledRejection", unhandled);
    expect(unhandled).not.toHaveBeenCalled();
  });
});
