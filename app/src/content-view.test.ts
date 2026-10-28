import { describe, expect, it } from "vitest";
import {
  createContentViewStates,
  resetContentViewStatesForAccount,
  updateContentViewState,
} from "./content-view";

describe("多来源内容页状态", () => {
  it("为观看历史、喜欢和收藏创建互不共享的默认状态", () => {
    const states = createContentViewStates();

    expect(states.history).toEqual({
      query: "",
      page: 1,
      viewMode: "list",
      includeRemoteRemoved: false,
      sort: "watchedNewest",
    });
    expect(states.likes.sort).toBe("pageOrder");
    expect(states.favorites.sort).toBe("pageOrder");
    expect(states.likes).not.toBe(states.history);
    expect(states.favorites).not.toBe(states.likes);
  });

  it("只更新指定来源并保留其他来源状态", () => {
    const states = createContentViewStates();
    const updated = updateContentViewState(states, "likes", {
      query: "作者",
      page: 3,
      viewMode: "grid",
      includeRemoteRemoved: true,
      sort: "pageOrder",
    });

    expect(updated.likes).toEqual({
      query: "作者",
      page: 3,
      viewMode: "grid",
      includeRemoteRemoved: true,
      sort: "pageOrder",
    });
    expect(updated.history).toEqual(states.history);
    expect(updated.favorites).toEqual(states.favorites);
  });

  it("切换账号时仅重置页码并保留用户的来源视图偏好", () => {
    const states = updateContentViewState(createContentViewStates(), "favorites", {
      query: "图文",
      page: 4,
      viewMode: "grid",
      includeRemoteRemoved: true,
    });

    const reset = resetContentViewStatesForAccount(states);

    expect(reset.favorites).toEqual({
      query: "图文",
      page: 1,
      viewMode: "grid",
      includeRemoteRemoved: true,
      sort: "pageOrder",
    });
  });
});
