import { describe, expect, it } from "vitest";
import {
  formatCollectorPhase,
  formatLikeCount,
  formatWatchProgress,
} from "../../app/src/formatters.js";

describe("formatLikeCount", () => {
  it("按准确数字、W、万、K 和千格式化点赞量", () => {
    expect(formatLikeCount(12345, "exact")).toBe("12,345");
    expect(formatLikeCount(10000, "w")).toBe("1W");
    expect(formatLikeCount(12500, "w")).toBe("1.3W");
    expect(formatLikeCount(10000, "wan")).toBe("1万");
    expect(formatLikeCount(12500, "wan")).toBe("1.3万");
    expect(formatLikeCount(1000, "k")).toBe("1K");
    expect(formatLikeCount(1250, "k")).toBe("1.3K");
    expect(formatLikeCount(1000, "qian")).toBe("1千");
    expect(formatLikeCount(1250, "qian")).toBe("1.3千");
  });

  it("小于单位阈值时保留精确数字", () => {
    expect(formatLikeCount(9999, "w")).toBe("9,999");
    expect(formatLikeCount(9999, "wan")).toBe("9,999");
    expect(formatLikeCount(999, "k")).toBe("999");
    expect(formatLikeCount(999, "qian")).toBe("999");
    expect(formatLikeCount(undefined, "wan")).toBeUndefined();
    expect(formatLikeCount(null, "wan")).toBeUndefined();
  });
});

describe("formatWatchProgress", () => {
  it("将 0 到 1 的数值进度转换为已看百分比", () => {
    expect(formatWatchProgress(0)).toBe("已看 0%");
    expect(formatWatchProgress(0.425)).toBe("已看 43%");
    expect(formatWatchProgress(1)).toBe("已看 100%");
    expect(formatWatchProgress(undefined)).toBeUndefined();
    expect(formatWatchProgress(null)).toBeUndefined();
  });
});

describe("formatCollectorPhase", () => {
  it("覆盖采集器实际发布的全部运行阶段", () => {
    expect(formatCollectorPhase("locating")).toBe("定位历史列表");
    expect(formatCollectorPhase("recovering_login")).toBe("正在恢复登录");
    expect(formatCollectorPhase("retrying_after_login")).toBe("登录已恢复，正在继续同步");
    expect(formatCollectorPhase("waiting-growth")).toBe("等待列表增长");
    expect(formatCollectorPhase("validating")).toBe("校验页面状态");
    expect(formatCollectorPhase("finalizing")).toBe("复核最终窗口");
  });
});
