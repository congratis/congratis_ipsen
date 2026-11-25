import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const distDir = join(dirname(fileURLToPath(import.meta.url)), "..", "dist");

async function fingerprint(relativePath) {
    const contents = await readFile(join(distDir, relativePath));
    return createHash("sha256").update(contents).digest("hex").slice(0, 16);
}

const [cssVersion, jsVersion] = await Promise.all([
    fingerprint("style.css"),
    fingerprint("js/index.js"),
]);

const htmlPath = join(distDir, "index.html");
let html = await readFile(htmlPath, "utf8");

for (const [placeholder, version] of [
    ["__CSS_VERSION__", cssVersion],
    ["__JS_VERSION__", jsVersion],
]) {
    if (!html.includes(placeholder)) {
        throw new Error(`Missing asset version placeholder: ${placeholder}`);
    }
    html = html.replaceAll(placeholder, version);
}

if (html.includes("__CSS_VERSION__") || html.includes("__JS_VERSION__")) {
    throw new Error("An asset version placeholder was not replaced");
}

await writeFile(htmlPath, html);
console.log(`Finalized frontend asset URLs (css=${cssVersion}, js=${jsVersion})`);
