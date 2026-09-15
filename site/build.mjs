// Build browser assets into dist/, served by the backend at /static/*.
import { cp, mkdir, rm } from "node:fs/promises";
import path from "node:path";

const root = import.meta.dirname;
const dist = path.join(root, "dist");

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
// Manually vendor oat.
await cp(path.join(root, "node_modules/@knadh/oat/oat.min.css"), path.join(dist, "oat.min.css"));
await cp(path.join(root, "node_modules/@knadh/oat/oat.min.js"), path.join(dist, "oat.min.js"));
console.log(`built ${result.outputs.length} bundles and copied static assets -> dist/`);
