// Build browser assets into dist/, served by the backend at /static/*.
import { cp, mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";

const root = import.meta.dirname;
const dist = path.join(root, "dist");

// Hacky function to compile icons into a single SVG sprite (dist/icons.svg).
async function buildIcons() {
  const dir = path.join(root, "assets/icons");
  const files = (await readdir(dir)).filter((f) => f.endsWith(".svg")).sort();
  const out = [];

  for (const file of files) {
    const name = path.basename(file, ".svg");
    const svg = (await readFile(path.join(dir, file), "utf8"))
      .replace(/<\?xml[\s\S]*?\?>/g, "")
      .replace(/<!--[\s\S]*?-->/g, "")
      .trim();

    const match = svg.match(/^<svg\b([^>]*)>([\s\S]*)<\/svg>$/);
    if (!/^[a-z][a-z0-9-]*$/.test(name) || !match ||
      !/\bviewBox\s*=/.test(match[1]) ||
      /\bid\s*=|<style\b|<!DOCTYPE/i.test(svg)) {
      throw new Error(`${file}: expected a simple SVG with viewBox, no IDs or style elements`);
    }

    const attrs = match[1].replace(/\s+(?:xmlns|width|height)\s*=\s*("[^"]*"|'[^']*')/g, "");
    out.push(`<symbol id="${name}"${attrs}>${match[2].trim()}</symbol>`);
  }

  await writeFile(path.join(dist, "icons.svg"),
    `<svg xmlns="http://www.w3.org/2000/svg">\n${out.join("\n")}\n</svg>\n`);
  console.log(`compiled ${out.length} icons -> dist/icons.svg`);
}

await rm(dist, { recursive: true, force: true });
await mkdir(dist, { recursive: true });

const result = await Bun.build({
  entrypoints: [
    path.join(root, "assets/js/main.js"),
    path.join(root, "assets/css/style.css"),
  ],
  outdir: dist,
  naming: "bundle.[ext]",
  format: "esm",
  target: "browser",
  external: ["/static/*"],
  minify: true,
});

if (!result.success) {
  throw new AggregateError(result.logs, "site build failed");
}

await cp(path.join(root, "assets/static"), dist, { recursive: true });
await buildIcons();
// Manually vendor oat.
await cp(path.join(root, "node_modules/@knadh/oat/oat.min.css"), path.join(dist, "oat.min.css"));
await cp(path.join(root, "node_modules/@knadh/oat/oat.min.js"), path.join(dist, "oat.min.js"));
console.log(`built ${result.outputs.length} bundles and copied static assets -> dist/`);
