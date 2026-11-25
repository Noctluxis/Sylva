// SPDX-License-Identifier: MPL-2.0
// Build-time only. Usage: node scripts/generate-app-icon.cjs <path-to-sharp>
// Sharp/librsvg is not linked or shipped with Sylva.
const fs = require('node:fs/promises');
const path = require('node:path');
const sharp = require(process.argv[2] || 'sharp');
(async () => {
  const folder = path.join(__dirname, '..', 'assets', 'branding');
  const svg = await fs.readFile(path.join(folder, 'sylva.svg'));
  const sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
  const frames = await Promise.all(sizes.map(n => sharp(svg).resize(n, n).png().toBuffer()));
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((n, i) => {
    const p = 6 + i * 16;
    header[p] = header[p + 1] = n % 256;
    header.writeUInt16LE(1, p + 4);
    header.writeUInt16LE(32, p + 6);
    header.writeUInt32LE(frames[i].length, p + 8);
    header.writeUInt32LE(offset, p + 12);
    offset += frames[i].length;
  });
  await fs.writeFile(path.join(folder, 'sylva.ico'), Buffer.concat([header, ...frames]));
  await fs.writeFile(path.join(folder, 'sylva.png'), frames.at(-1));
  console.log(`Sylva icon: ${sizes.length} sizes, ${offset} bytes`);
})().catch(error => { console.error(error); process.exitCode = 1; });
