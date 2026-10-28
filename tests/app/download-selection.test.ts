import { describe, expect, test } from "vitest";
import { buildDownloadSelection } from "../../app/src/download/selection.js";

describe("download selection", () => {
  test("builds a deduplicated checked-record request", () => {
    expect(
      buildDownloadSelection({ mode: "checked", checkedIds: [7, 3, 7, 3] }),
    ).toEqual({ kind: "checked", watch_record_ids: [7, 3] });
  });

  test("builds the complete filtered-result request independent of pagination", () => {
    expect(buildDownloadSelection({ mode: "filtered", query: " 猫 " })).toEqual({
      kind: "filtered",
      query: "猫",
    });
  });

  test("builds the all-history request", () => {
    expect(buildDownloadSelection({ mode: "all" })).toEqual({ kind: "all" });
  });
});
