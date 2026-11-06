// Builds docs/Ghid-LegatorOCR.pdf from guide.html (the text) and img/ (the screenshots), with the
// version of this build written into the text.   Usage: node docs/guide-src/build.js
// Needs the `playwright` package and a Chromium (npx playwright install chromium).
const fs = require('fs'), path = require('path');
const { chromium } = require('playwright');
(async () => {
  const root = path.resolve(__dirname, '..', '..');
  const version = /^version\s*=\s*"([^"]+)"/m.exec(fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8'))[1];
  const html = fs.readFileSync(path.join(__dirname, 'guide.html'), 'utf8').split('{{VERSION}}').join(version);
  const tmp = path.join(__dirname, '.build.html');
  fs.writeFileSync(tmp, html);
  const b = await chromium.launch();
  const p = await b.newPage();
  await p.goto('file://' + tmp);
  await p.waitForLoadState('load');
  await p.pdf({ path: path.join(root, 'docs', 'Ghid-LegatorOCR.pdf'), format: 'A4', printBackground: true,
    displayHeaderFooter: true, headerTemplate: '<span></span>',
    footerTemplate: '<div style="font-size:8px;width:100%;text-align:center"><span class="pageNumber"></span> / <span class="totalPages"></span></div>',
    margin: { top: '16mm', bottom: '16mm', left: '15mm', right: '15mm' } });
  await b.close();
  fs.unlinkSync(tmp);
  console.log('wrote docs/Ghid-LegatorOCR.pdf for', version);
})();
