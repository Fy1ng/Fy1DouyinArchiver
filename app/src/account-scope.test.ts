import { describe, expect, it } from "vitest";
import { resolvePortableAccountScope } from "./account-scope";

const accounts = [
  { id: 7, accountKey: "sec:alpha" },
  { id: 9, accountKey: "uid:beta" },
];

describe("portable account scope resolution", () => {
  it("keeps all explicit and resolves selected keys in stable id order", () => {
    expect(resolvePortableAccountScope({ kind: "all" }, accounts)).toEqual({ kind: "all" });
    expect(
      resolvePortableAccountScope(
        { kind: "selected", accountKeys: ["uid:beta", "sec:alpha"] },
        accounts,
      ),
    ).toEqual({ kind: "selected", accountIds: [7, 9] });
  });

  it("rejects empty, duplicate, or stale selections without widening to all", () => {
    for (const accountKeys of [[], ["sec:alpha", "sec:alpha"], ["sec:missing"]]) {
      expect(() =>
        resolvePortableAccountScope({ kind: "selected", accountKeys }, accounts),
      ).toThrow("account_scope_invalid");
    }
  });
});
