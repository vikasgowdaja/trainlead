import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import sharp from 'sharp';

const source = process.argv[2];
if (!source) throw new Error('Pass the path to the supplied TrainLead logo PNG');
const destination = new URL('../public/brand/', import.meta.url);
await mkdir(destination, { recursive: true });
const metadata = await sharp(source).metadata();
if (!metadata.width || !metadata.height) throw new Error('Cannot read logo dimensions');
const { width, height } = metadata;
const outputPath = name => fileURLToPath(new URL(name, destination));
await sharp(source).extract({
  left: Math.round(width * 0.09), top: Math.round(height * 0.22),
  width: Math.round(width * 0.82), height: Math.round(height * 0.62),
}).resize({ width: 1200 }).png().toFile(outputPath('wordmark.png'));
const symbol = await sharp(source).extract({
  left: Math.round(width * 0.095), top: Math.round(height * 0.22),
  width: Math.round(width * 0.28), height: Math.round(height * 0.62),
}).resize(800, 800, { fit: 'contain', background: '#FFFFFF' }).png().toBuffer();
const square = await sharp({ create: { width: 1024, height: 1024, channels: 4, background: '#FFFFFF' } })
  .composite([{ input: symbol, left: 112, top: 112 }]).png().toBuffer();
await sharp(square).toFile(outputPath('icon.png'));
await sharp(square).resize(64, 64).toFile(outputPath('favicon.png'));
await sharp(square).resize(180, 180).toFile(outputPath('apple-touch-icon.png'));
console.log('Generated TrainLead wordmark, square launcher source, and browser icons');