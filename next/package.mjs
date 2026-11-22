#!/usr/bin/env node
// Packages the `threads` builds made by Rapier's own generator (bindings/typescript, `-f threads`)
// as @<scope>/rapier{2,3}d-threads-next, ready to test and publish. Lives only on the `next` branch.
//
//   node next/package.mjs            (after `npm run build:wasm && npm run build:ts` in each build dir)
import { cpSync, existsSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "..");
const config = JSON.parse(readFileSync(join(here, "config.json"), "utf8"));
const run = process.env.GITHUB_RUN_NUMBER ?? "0";
const commit = process.env.GITHUB_SHA ?? "local";

// Rewrite extensionless relative imports so the package also works as plain browser ESM.
function addJsExtensions(dir) {
  for (const f of readdirSync(dir, { recursive: true }).map(String)) {
    const p = join(dir, f);
    if (!statSync(p).isFile() || !/\.(js|d\.ts)$/.test(p)) continue;
    const src = readFileSync(p, "utf8");
    const fixed = src.replace(/((?:from|import)\s*["'])(\.{1,2}\/[^"']+?)(["'])/g, (m, a, spec, c) => {
      if (/\.(js|mjs|json|wasm)$/.test(spec)) return m;
      const abs = resolve(dirname(p), spec);
      return existsSync(abs) && statSync(abs).isDirectory() ? `${a}${spec}/index.js${c}` : `${a}${spec}.js${c}`;
    });
    if (fixed !== src) writeFileSync(p, fixed);
  }
}

for (const dim of config.dims) {
  const built = join(repo, "bindings", "typescript", "builds", `rapier${dim}d-threads`, "pkg");
  if (!existsSync(join(built, `rapier_wasm${dim}d_bg.wasm`))) throw new Error(`missing build: ${built}`);
  // Folder keeps the -threads name so the shared browser tests load /dist/rapier{dim}d-threads/.
  const out = join(here, "dist", `rapier${dim}d-threads`);
  rmSync(out, { recursive: true, force: true });
  cpSync(built, out, { recursive: true });
  rmSync(join(out, "src"), { recursive: true, force: true });
  rmSync(join(out, ".gitignore"), { force: true });
  if (!existsSync(join(out, "snippets"))) throw new Error("wasm-bindgen-rayon snippet missing: was it built with -f threads?");
  addJsExtensions(out);
  cpSync(join(here, "overlay", "vite.js"), join(out, "vite.js"));
  cpSync(join(here, "overlay", "vite.d.ts"), join(out, "vite.d.ts"));
  cpSync(join(repo, "bindings", "typescript", "LICENSE"), join(out, "LICENSE"));

  const base = JSON.parse(readFileSync(join(out, "package.json"), "utf8")).version;
  const name = `${config.npmScope}/rapier${dim}d-${config.packageSuffix}`;
  const files = readdirSync(out).filter((f) => f !== "package.json");
  writeFileSync(join(out, "package.json"), JSON.stringify({
    name,
    version: `${base}-next.${run}`,
    description: `Experimental multi-threaded Rapier ${dim}D for the web (WASM threads + SIMD), built from ${config.repository} (next branch): upstream Rapier plus optimizations proposed upstream. Same API as @dimforge/rapier${dim}d-simd-compat.`,
    license: "Apache-2.0",
    type: "module",
    main: "rapier.js",
    module: "rapier.js",
    types: "rapier.d.ts",
    exports: { ".": { types: "./rapier.d.ts", default: "./rapier.js" }, "./vite": { types: "./vite.d.ts", default: "./vite.js" }, "./*": "./*" },
    files: [...files, "README.md"],
    sideEffects: ["./snippets/**", "./rapier_wasm*.js"],
    keywords: ["physics", "rapier", "wasm", "threads", "multithreaded", "simd", "experimental"],
    repository: { type: "git", url: `git+${config.repository}.git` },
    rapierNext: { commit },
  }, null, 2) + "\n");
  writeFileSync(join(out, "README.md"), `# ${name}

**Experimental, unofficial** multi-threaded build of [Rapier](https://rapier.rs) ${dim}D: upstream Rapier
plus optimizations that are proposed upstream but not merged yet. Not affiliated with Dimforge.

- Built from [\`${config.repository}\`](${config.repository}/tree/next) (branch \`next\`), commit \`${commit}\`.
- For production, use [\`${config.npmScope}/rapier${dim}d-threads\`](https://www.npmjs.com/package/${config.npmScope}/rapier${dim}d-threads)
  (official Rapier, unmodified) or the official [\`@dimforge/rapier${dim}d-simd-compat\`](https://www.npmjs.com/package/@dimforge/rapier${dim}d-simd-compat).

Same API and requirements as \`${config.npmScope}/rapier${dim}d-threads\`: a cross-origin-isolated page
(COOP/COEP) and running Rapier in a Web Worker. \`await RAPIER.init({ threads })\` starts the pool.
Vite: \`import rapierThreads from "${name}/vite"\` and add it to \`plugins\` and \`worker.plugins\`.

License: Apache-2.0, same as Rapier.
`);
  console.log(`packaged ${name}@${base}-next.${run} -> ${out}`);
}
