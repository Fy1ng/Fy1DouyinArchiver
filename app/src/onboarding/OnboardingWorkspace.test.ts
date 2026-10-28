import "@testing-library/jest-dom/vitest";
import { cleanup, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import OnboardingWorkspace from "./OnboardingWorkspace.svelte";

describe("OnboardingWorkspace", () => {
  afterEach(() => cleanup());

  it("renders a full-window brand flow with four persisted steps", () => {
    render(OnboardingWorkspace, {
      step: 1,
      appIconUrl: "/app-icon.png",
    });

    expect(screen.getByRole("dialog", { name: "首次设置" })).toHaveClass(
      "onboarding-workspace",
    );
    expect(screen.getByRole("img", { name: "Fy1DouyinArchiver" })).toHaveAttribute(
      "src",
      "/app-icon.png",
    );
    const progress = screen.getByRole("list", { name: "设置进度" });
    expect(progress.children).toHaveLength(4);
    expect(within(progress).getByText("同步设置").closest("li")).toHaveAttribute(
      "aria-current",
      "step",
    );
  });
});
