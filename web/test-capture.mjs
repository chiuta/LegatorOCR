// End-to-end check of web/capture.html in Chromium with a fake camera that shows web/sample.png.
// usage: node web/test-capture.mjs path/to/legatorocr_wasm.wasm path/to/camera.y4m
// needs the `playwright` package (npm i playwright) and its Chromium (npx playwright install chromium)
import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";

const require = createRequire(process.cwd() + "/");
const { chromium } = require("playwright");
const [wasmPath, y4m] = process.argv.slice(2);
const html = readFileSync(new URL("./capture.html", import.meta.url));
const wasm = readFileSync(wasmPath);
const server = createServer((req, res) => {
  if (req.url === "/legatorocr_wasm.wasm") { res.writeHead(200, { "content-type": "application/wasm" }); res.end(wasm); }
  else { res.writeHead(200, { "content-type": "text/html" }); res.end(html); }
}).listen(0);
const port = server.address().port;

const browser = await chromium.launch({
  args: ["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream", `--use-file-for-fake-video-capture=${y4m}`],
});
const ctx = await browser.newContext({ permissions: ["camera"] });
const page = await ctx.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
await page.goto(`http://localhost:${port}/`);
await page.click("#start");
await page.waitForFunction(() => /Clar|Neclar/.test(document.getElementById("verdict").textContent), null, { timeout: 30000 });
console.log("verdict:", await page.textContent("#verdict"));
await page.click("#shoot");
await page.waitForFunction(() => document.querySelectorAll("#shots img").length === 1, null, { timeout: 60000 });
console.log("status:", await page.textContent("#status"));
const size = await page.evaluate(() => { const i = document.querySelector("#shots img"); return [i.naturalWidth, i.naturalHeight]; });
console.log("cleaned page image:", size.join("x"));
await browser.close(); server.close();
if (errors.length) throw new Error("page errors: " + errors.join("; "));
if (size[0] < 100 || size[1] < 100) throw new Error("implausible page size");
console.log("ok");
