import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import DialogFixture from "./Dialog.fixture.svelte";

describe("Dialog", () => {
  afterEach(() => cleanup());

  it("isolates the background, traps focus, supports nested dialogs, and restores focus", async () => {
    render(DialogFixture);
    const trigger = screen.getByRole("button", { name: "打开主弹窗" });

    await fireEvent.click(trigger);
    const primary = screen.getByRole("dialog", { name: "主弹窗" });
    const primaryCancel = screen.getByRole("button", { name: "取消主弹窗" });
    const openNested = screen.getByRole("button", { name: "打开嵌套弹窗" });
    const background = screen.getByTestId("workspace-background");

    await waitFor(() => expect(primaryCancel).toHaveFocus());
    expect(background).toHaveAttribute("inert");
    expect(background).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByTestId("modal-host")).toContainElement(primary);

    await fireEvent.keyDown(primary, { key: "Tab", shiftKey: true });
    expect(openNested).toHaveFocus();
    await fireEvent.keyDown(primary, { key: "Tab" });
    expect(primaryCancel).toHaveFocus();

    await fireEvent.click(openNested);
    const nested = screen.getByRole("dialog", { name: "嵌套弹窗" });
    const nestedCancel = screen.getByRole("button", { name: "取消嵌套弹窗" });
    await waitFor(() => expect(nestedCancel).toHaveFocus());

    const primaryLayer = primary.closest<HTMLElement>("[data-dialog-layer]");
    expect(primaryLayer).toHaveAttribute("inert");
    expect(primaryLayer).toHaveAttribute("aria-hidden", "true");
    expect(background).toHaveAttribute("inert");

    await fireEvent.keyDown(nested, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "嵌套弹窗" })).not.toBeInTheDocument());
    expect(openNested).toHaveFocus();
    expect(primaryLayer).not.toHaveAttribute("inert");
    expect(background).toHaveAttribute("inert");

    await fireEvent.keyDown(primary, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "主弹窗" })).not.toBeInTheDocument());
    expect(trigger).toHaveFocus();
    expect(background).not.toHaveAttribute("inert");
    expect(background).not.toHaveAttribute("aria-hidden");
  });
});
