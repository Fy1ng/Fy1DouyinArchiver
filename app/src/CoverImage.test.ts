import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";
import CoverImage from "./CoverImage.svelte";

describe("封面本地优先与网络回退", () => {
  afterEach(() => cleanup());

  it("本地缓存失败后回退远端，远端也失败时显示占位", async () => {
    render(CoverImage, {
      cachedCoverUrl: "http://cover-cache.localhost/7",
      remoteCoverUrl: "https://example.com/cover.jpg",
      preferCached: true,
      alt: "测试封面",
    });

    const local = screen.getByRole("img", { name: "测试封面" });
    expect(local).toHaveAttribute("src", "http://cover-cache.localhost/7");

    await fireEvent.error(local);
    const remote = screen.getByRole("img", { name: "测试封面" });
    expect(remote).toHaveAttribute("src", "https://example.com/cover.jpg");

    await fireEvent.error(remote);
    expect(screen.queryByRole("img", { name: "测试封面" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("无封面")).toBeVisible();
  });

  it("联网模式不读取已有本地缓存", () => {
    render(CoverImage, {
      cachedCoverUrl: "http://cover-cache.localhost/7",
      remoteCoverUrl: "https://example.com/cover.jpg",
      preferCached: false,
      alt: "联网封面",
    });

    expect(screen.getByRole("img", { name: "联网封面" })).toHaveAttribute(
      "src",
      "https://example.com/cover.jpg",
    );
  });
});
