import { lstat, readdir } from "node:fs/promises";
import path from "node:path";

export async function walkTree(root, relative) {
  const directory = path.join(root, relative);
  await assertDirectory(directory, relative);
  const found = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const child = path.posix.join(relative.replaceAll("\\", "/"), entry.name);
    const absolute = path.join(root, child);
    const info = await lstat(absolute);
    if (info.isSymbolicLink()) {
      throw new Error(`禁止符号链接: ${child}`);
    }
    if (info.isDirectory()) {
      found.push(...await walkTree(root, child));
    } else if (info.isFile()) {
      found.push(child);
    } else {
      throw new Error(`禁止非常规文件: ${child}`);
    }
  }
  return found;
}

export async function assertRegularFile(root, relative) {
  const absolute = path.join(root, relative);
  const info = await checkedMetadata(absolute, relative);
  if (!info.isFile()) {
    throw new Error(`禁止非常规文件: ${relative}`);
  }
}

async function assertDirectory(absolute, relative) {
  const info = await checkedMetadata(absolute, relative);
  if (!info.isDirectory()) {
    throw new Error(`禁止非常规目录: ${relative}`);
  }
}

async function checkedMetadata(absolute, relative) {
  const info = await lstat(absolute);
  if (info.isSymbolicLink()) {
    throw new Error(`禁止符号链接: ${relative}`);
  }
  return info;
}
