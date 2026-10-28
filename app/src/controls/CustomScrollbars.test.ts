import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import CustomScrollbars from "./CustomScrollbars.svelte";

function createScrollableTarget(): HTMLElement {
  const target = document.createElement("section");
  target.setAttribute("aria-label", "搜索结果");
  target.style.overflowY = "auto";
  Object.defineProperties(target, {
    clientHeight: { configurable: true, value: 200 },
    clientWidth: { configurable: true, value: 300 },
    scrollHeight: { configurable: true, value: 1000 },
    scrollTop: { configurable: true, value: 0, writable: true },
  });
  target.getBoundingClientRect = () => ({
    bottom: 210,
    height: 200,
    left: 10,
    right: 310,
    top: 10,
    width: 300,
    x: 10,
    y: 10,
    toJSON: () => undefined,
  });
  document.body.appendChild(target);
  return target;
}

describe("CustomScrollbars", () => {
  afterEach(() => {
    cleanup();
    document.querySelectorAll(".custom-scrollbar-track").forEach((node) => node.remove());
  });

  it("exposes scrollbar semantics and supports every keyboard scroll command", async () => {
    const target = createScrollableTarget();
    const view = render(CustomScrollbars);
    const scrollbar = await screen.findByRole("scrollbar", { name: "搜索结果滚动条" });

    expect(target.id).toMatch(/^emily-scroll-viewport-/);
    expect(scrollbar).toHaveAttribute("aria-controls", target.id);
    expect(scrollbar).toHaveAttribute("aria-orientation", "vertical");
    expect(scrollbar).toHaveAttribute("aria-valuemin", "0");
    expect(scrollbar).toHaveAttribute("aria-valuemax", "800");
    expect(scrollbar).toHaveAttribute("aria-valuenow", "0");

    await fireEvent.keyDown(scrollbar, { key: "ArrowDown" });
    expect(target.scrollTop).toBe(40);
    expect(scrollbar).toHaveAttribute("aria-valuenow", "40");

    await fireEvent.keyDown(scrollbar, { key: "PageDown" });
    expect(target.scrollTop).toBe(240);

    await fireEvent.keyDown(scrollbar, { key: "End" });
    expect(target.scrollTop).toBe(800);
    expect(scrollbar).toHaveAttribute("aria-valuenow", "800");

    await fireEvent.keyDown(scrollbar, { key: "PageUp" });
    expect(target.scrollTop).toBe(600);

    await fireEvent.keyDown(scrollbar, { key: "ArrowUp" });
    expect(target.scrollTop).toBe(560);

    await fireEvent.keyDown(scrollbar, { key: "Home" });
    expect(target.scrollTop).toBe(0);

    view.unmount();
    await waitFor(() => expect(screen.queryByRole("scrollbar")).not.toBeInTheDocument());
    expect(target).not.toHaveAttribute("id");
    target.remove();
  });
});
