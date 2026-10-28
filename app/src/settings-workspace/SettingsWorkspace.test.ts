import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import Fixture from "./SettingsWorkspace.fixture.svelte";

describe("SettingsWorkspace", () => {
  afterEach(() => cleanup());

  it("exposes the approved nine settings categories and save status", () => {
    render(Fixture);

    expect(screen.getByRole("navigation", { name: "设置分类" }).children).toHaveLength(9);
    expect(screen.getByText("已保存")).toHaveAttribute("aria-live", "polite");
    expect(screen.getByRole("button", { name: "常规与外观" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("navigation", { name: "设置子项" })).toBeVisible();
    expect(screen.getByRole("button", { name: "外观" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("renders only the selected nested settings panel", async () => {
    render(Fixture);

    expect(screen.getByLabelText("主题").closest("section")).toBeVisible();
    expect(screen.getByLabelText("关闭行为").closest("section")).not.toBeVisible();
    expect(screen.getByLabelText("消息提示停留时间").closest("section")).not.toBeVisible();

    await fireEvent.click(screen.getByRole("button", { name: "桌面行为" }));
    expect(screen.getByLabelText("关闭行为").closest("section")).toBeVisible();
    expect(screen.getByLabelText("主题").closest("section")).not.toBeVisible();
  });

  it("searches all categories, switches to the match, and highlights it", async () => {
    render(Fixture);

    await fireEvent.input(screen.getByRole("searchbox", { name: "搜索设置" }), {
      target: { value: "消息提示停留时间" },
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "通知" })).toHaveAttribute(
        "aria-current",
        "page",
      ),
    );
    expect(screen.getByRole("button", { name: "消息行为" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByLabelText("消息提示停留时间").closest("section")).toBeVisible();
    expect(screen.getByLabelText("消息提示停留时间").closest("section")).toHaveClass(
      "settings-search-hit",
    );
  });
});
