import sharp from 'sharp';
import { readFileSync } from 'fs';

const svg = readFileSync('src-tauri/icons/icon.svg');
const out = 'assets/icon_svg_1024.png';

await sharp(svg)
  .resize(1024, 1024)
  .png()
  .toFile(out);

console.log('Saved', out);
