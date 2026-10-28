import type { LikeCountFormat } from "./types.js";

const integerFormatter = new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 });

export function formatLikeCount(
  value: number | null | undefined,
  format: LikeCountFormat,
): string | undefined {
  if (value == null) return undefined;
  if (format === "w" && value >= 10_000) return `${formatUnit(value / 10_000)}W`;
  if (format === "wan" && value >= 10_000) return `${formatUnit(value / 10_000)}万`;
  if (format === "k" && value >= 1_000) return `${formatUnit(value / 1_000)}K`;
  if (format === "qian" && value >= 1_000) return `${formatUnit(value / 1_000)}千`;
  return integerFormatter.format(value);
}

export function formatWatchProgress(value: number | null | undefined): string | undefined {
  if (value == null) return undefined;
  return `已看 ${Math.round(value * 100)}%`;
}

export function formatCollectorPhase(phase?: string): string {
  const labels: Record<string, string> = {
    launching: "启动浏览器",
    "login-check": "验证登录",
    recovering_login: "正在恢复登录",
    retrying_after_login: "登录已恢复，正在继续同步",
    locating: "定位历史列表",
    scrolling: "加载更多记录",
    "waiting-growth": "等待列表增长",
    validating: "校验页面状态",
    finalizing: "复核最终窗口",
    completed: "采集完成",
    cancelled: "正在取消",
    failed: "采集失败",
  };
  return phase ? (labels[phase] ?? phase) : "";
}

function formatUnit(value: number): string {
  return value.toFixed(1).replace(/\.0$/, "");
}
