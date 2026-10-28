import { mkdtemp, mkdir, rm, symlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { assertRegularFile, walkTree } from "../scripts/frontend-manifest-walk.mjs";

describe("frontend manifest tree policy", () => {
  it("rejects symbolic links even when their target stays inside the repository", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "emily-manifest-walk-"));
    const source = path.join(root, "source");
    const target = path.join(root, "target");
    await mkdir(source);
    await mkdir(target);
    await symlink(target, path.join(source, "linked"), process.platform === "win32" ? "junction" : "dir");

    try {
      await expect(walkTree(root, "source")).rejects.toThrow("禁止符号链接");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it("rejects a symbolic traversal root and fixed input", async () => {
    const root = await mkdtemp(path.join(tmpdir(), "emily-manifest-root-"));
    const target = path.join(root, "target");
    const linkedRoot = path.join(root, "linked-root");
    await mkdir(target);
    await symlink(target, linkedRoot, process.platform === "win32" ? "junction" : "dir");

    try {
      await expect(walkTree(root, "linked-root")).rejects.toThrow("禁止符号链接");
      await expect(assertRegularFile(root, "linked-root")).rejects.toThrow("禁止符号链接");
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
