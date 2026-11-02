// Loads the compiled module and checks it end to end: usage `node web/test-wasm.mjs path/to/legatorocr_wasm.wasm`
import { readFileSync } from "node:fs";

const wasmPath = process.argv[2];
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), {});
const x = instance.exports;
const png = readFileSync(new URL("./sample.png", import.meta.url));

function put(bytes) {
  const p = x.lgr_alloc(bytes.length);
  new Uint8Array(x.memory.buffer, p, bytes.length).set(bytes);
  return p;
}
let p = put(png);
const sharp = x.lgr_sharpness(p, png.length);
const pages = x.lgr_process(p, png.length, 2, 300);
x.lgr_free(p, png.length);
if (pages < 1) throw new Error(`lgr_process returned ${pages}`);
const out = new Uint8Array(x.memory.buffer, x.lgr_out_ptr(), x.lgr_out_len()).slice();
const sig = [0x89, 0x50, 0x4e, 0x47].every((b, i) => out[i] === b);
if (!sig) throw new Error("output is not a PNG");
const w = new DataView(out.buffer).getUint32(16), h = new DataView(out.buffer).getUint32(20);
console.log(`sharpness*1000=${sharp} pages=${pages} output PNG ${w}x${h} (${out.length} bytes)`);
if (sharp <= 0 || w < 100 || h < 100) throw new Error("implausible result");
p = put(new TextEncoder().encode("not an image"));
if (x.lgr_process(p, 12, 2, 300) !== -1) throw new Error("garbage must give -1");
console.log("ok");
