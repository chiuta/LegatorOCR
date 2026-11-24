import { createServer } from "node:http";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
// Timer of web/capture.html in Chromium with fake cameras: good frames are shot, dark frames are not
// and lengthen the interval, a blank interval means 15 s.  Needs the `playwright` package.
// usage: node web/test-capture-timer.mjs good.y4m dark.y4m   (CHROME=/path/to/chrome to use a system browser)
const require = createRequire(process.cwd() + "/");
const { chromium } = require("playwright");
const [GOOD, DARK] = process.argv.slice(2);
const html = readFileSync(new URL("./capture.html", import.meta.url));
const server = createServer((q, r) => { r.writeHead(200, { "content-type": "text/html" }); r.end(html); }).listen(0);
const port = server.address().port;
async function run(y4m, label, secs) {
  const b = await chromium.launch({ executablePath: process.env.CHROME || undefined, args: ["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream", `--use-file-for-fake-video-capture=${y4m}`] });
  const ctx = await b.newContext({ permissions: ["camera"] });
  const p = await ctx.newPage(); const errs = [];
  p.on("pageerror", (e) => errs.push(e.message));
  await p.goto(`http://localhost:${port}/`);
  const info = await p.evaluate(async (secs) => {
    window.count = 0; window.shoot = async () => { window.count++; };
    const v = document.getElementById("cam"); v.srcObject = await navigator.mediaDevices.getUserMedia({ video: true });
    await new Promise((r) => (v.onloadeddata = r)); await v.play();
    document.getElementById("auto").disabled = false; document.getElementById("secs").value = secs;
    document.getElementById("auto").click(); return [v.videoWidth, v.videoHeight];
  }, secs);
  await p.waitForTimeout((Number(secs) || 3) * 1000 * 3 + 1500);
  const out = await p.evaluate(() => ({ count: window.count, secs: document.getElementById("secs").value, timer: document.getElementById("timer").textContent }));
  const want = { "good frames": (o) => o.count >= 2, "dark frames": (o) => o.count === 0 && Number(o.secs) > 3, "blank field": (o) => o.count === 0 && /\d+ s/.test(o.timer) && Number(o.timer.match(/(\d+) s/)[1]) > 3, "text field": (o) => o.count === 0 }[label];
  if (errs.length || !want(out)) { console.log("FAIL", label, JSON.stringify(out), errs); process.exitCode = 1; }
  console.log(label, "video", info.join("x"), JSON.stringify(out), "errors:", errs.length ? errs : "none");
  await b.close();
}
await run(GOOD, "good frames", 3);
await run(DARK, "dark frames", 3);
await run(GOOD, "blank field", ""); await run(GOOD, "text field", "abc");
server.close();
