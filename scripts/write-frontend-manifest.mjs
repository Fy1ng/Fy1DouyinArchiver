import { createHash } from "node:crypto";
import { readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { assertRegularFile, walkTree } from "./frontend-manifest-walk.mjs";

const root = process.cwd();
const dist = path.join(root, "dist");
const fixedInputs = ["app/index.html", "app/vite.config.ts", "tsconfig.json", "package.json", "package-lock.json"];
await Promise.all(fixedInputs.map((file) => assertRegularFile(root, file)));
const lock = JSON.parse(await readFile(path.join(root, "package-lock.json"), "utf8"));
const agent = process.env.npm_config_user_agent || "";
const npm = agent.match(/\bnpm\/([^\s]+)/)?.[1];
if (!npm) throw new Error("无法从 npm_config_user_agent 确认 npm 版本");
const inputs = [...fixedInputs, ...(await walkTree(root, "app/src")).filter((file) => /\.(svelte|ts|css)$/.test(file))].sort();
const outputs = (await walkTree(root, "dist")).filter((file) => file !== "dist/build-manifest.json").map((file) => file.slice("dist/".length)).sort();
for (const output of outputs) {
  const allowed = output === "index.html" || /^assets\/.+\.(?:js|css|png|jpe?g|webp|avif|svg|ico|woff2?)$/.test(output);
  if (!allowed) throw new Error(`禁止打包的 dist 文件: ${output}`);
}
const manifest = {
  schemaVersion: 1,
  buildCommand: "npm run app:build",
  tools: {
    node: process.versions.node,
    npm,
    vite: lock.packages["node_modules/vite"].version,
    svelte: lock.packages["node_modules/svelte"].version,
    typescript: lock.packages["node_modules/typescript"].version,
    sveltePlugin: lock.packages["node_modules/@sveltejs/vite-plugin-svelte"].version,
  },
  inputs: await Promise.all(inputs.map(async (file) => ({ path: file, sha256: await hash(path.join(root, file)) }))),
  outputs: await Promise.all(outputs.map(async (file) => { const bytes = await readFile(path.join(dist, file)); return { path: file, bytes: bytes.length, sha256: digest(bytes) }; })),
};
const target = path.join(dist, "build-manifest.json");
const temporary = `${target}.${process.pid}.tmp`;
await writeFile(temporary, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
await rename(temporary, target);

async function hash(file) { return digest(await readFile(file)); }
function digest(bytes) { return createHash("sha256").update(bytes).digest("hex"); }
