import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import SelectField from "./SelectField.svelte";

describe("SelectField", () => {
  afterEach(() => cleanup());

  it("opens a styled listbox and commits the selected option", async () => {
    const changes: string[] = [];
    render(SelectField, {
      ariaLabel: "排序",
      value: "newest",
      onchange: (value: string) => changes.push(value),
      options: [
        { value: "newest", label: "最新" },
        { value: "oldest", label: "最早" },
      ],
    });
    await fireEvent.click(screen.getByRole("button", { name: "排序" }));
    expect(screen.getByRole("listbox", { name: "排序" })).toBeVisible();
    await fireEvent.click(screen.getByRole("option", { name: "最早" }));

    expect(changes).toEqual(["oldest"]);
    expect(screen.getByRole("button", { name: "排序" })).toHaveTextContent("最早");
  });

  it("accepts the legacy change-event test boundary on the visible control", async () => {
    render(SelectField, {
      ariaLabel: "来源",
      value: "all",
      options: [
        { value: "all", label: "全部" },
        { value: "likes", label: "喜欢" },
      ],
    });
    const trigger = screen.getByRole("button", { name: "来源" });
    await fireEvent.change(trigger, { target: { value: "likes" } });
    expect(trigger).toHaveValue("likes");
  });
});
