#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import pako from "pako";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(__dirname, "..");
const outputPath = path.join(repositoryRoot, "packages", "extension-core", "resources", "branding", "icon.png");
const checkOnly = process.argv.includes("--check");
const size = 256;
const pixels = Buffer.alloc(size * size * 4);

for (let y = 0; y < size; y += 1) {
  for (let x = 0; x < size; x += 1) {
    const offset = (y * size + x) * 4;
    const t = (x + y) / (size * 2);
    const radial = Math.min(1, Math.hypot(x - 80, y - 70) / 240);
    const base = mixColor([8, 48, 52], [20, 64, 104], t * 0.75 + radial * 0.25);
    pixels[offset] = base[0];
    pixels[offset + 1] = base[1];
    pixels[offset + 2] = base[2];
    pixels[offset + 3] = 255;
  }
}

drawOrbit(128, 130, 78, 42, -0.42, [112, 220, 197], 0.54, 5);
drawOrbit(128, 130, 86, 48, 0.48, [238, 183, 82], 0.5, 4);
drawBlockS();
drawDot(82, 75, 8, [238, 183, 82], 0.95);
drawDot(176, 176, 8, [112, 220, 197], 0.95);
drawDot(132, 128, 5, [255, 255, 255], 0.5);

const icon = encodePng(size, size, pixels);
const output = path.relative(repositoryRoot, outputPath);

if (checkOnly) {
  if (!fs.existsSync(outputPath)) {
    console.error(JSON.stringify({ status: "failed", reason: "missing generated icon", output }, null, 2));
    process.exit(1);
  }
  const current = fs.readFileSync(outputPath);
  if (!current.equals(icon)) {
    console.error(JSON.stringify({ status: "failed", reason: "generated icon is out of sync", output }, null, 2));
    process.exit(1);
  }
  console.log(JSON.stringify({ status: "passed", output, size }, null, 2));
  process.exit(0);
}

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, icon);
console.log(JSON.stringify({ status: "generated", output, size }, null, 2));

function drawBlockS() {
  const cream = [244, 245, 231];
  drawRoundedRect(70, 60, 122, 28, 14, cream, 0.98);
  drawRoundedRect(62, 70, 30, 62, 15, cream, 0.98);
  drawRoundedRect(74, 114, 110, 28, 14, cream, 0.98);
  drawRoundedRect(164, 124, 30, 62, 15, cream, 0.98);
  drawRoundedRect(64, 168, 122, 28, 14, cream, 0.98);
}

function drawOrbit(cx, cy, rx, ry, angle, color, alpha, thickness) {
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);
  for (let y = 0; y < size; y += 1) {
    for (let x = 0; x < size; x += 1) {
      const dx = x - cx;
      const dy = y - cy;
      const px = dx * cos + dy * sin;
      const py = -dx * sin + dy * cos;
      const normalized = Math.hypot(px / rx, py / ry);
      const distance = Math.abs(normalized - 1) * Math.max(rx, ry);
      if (distance <= thickness) {
        blendPixel(x, y, color, alpha * (1 - distance / (thickness + 1)));
      }
    }
  }
}

function drawRoundedRect(x, y, width, height, radius, color, alpha) {
  for (let py = Math.floor(y); py < y + height; py += 1) {
    for (let px = Math.floor(x); px < x + width; px += 1) {
      const closestX = Math.max(x + radius, Math.min(px, x + width - radius));
      const closestY = Math.max(y + radius, Math.min(py, y + height - radius));
      const dx = px - closestX;
      const dy = py - closestY;
      if (dx * dx + dy * dy <= radius * radius || (px >= x + radius && px <= x + width - radius) || (py >= y + radius && py <= y + height - radius)) {
        blendPixel(px, py, color, alpha);
      }
    }
  }
}

function drawDot(cx, cy, radius, color, alpha) {
  for (let y = cy - radius; y <= cy + radius; y += 1) {
    for (let x = cx - radius; x <= cx + radius; x += 1) {
      const distance = Math.hypot(x - cx, y - cy);
      if (distance <= radius) {
        blendPixel(x, y, color, alpha * (1 - Math.max(0, distance - radius + 2) / 2));
      }
    }
  }
}

function blendPixel(x, y, color, alpha) {
  if (x < 0 || x >= size || y < 0 || y >= size || alpha <= 0) {
    return;
  }
  const offset = (Math.floor(y) * size + Math.floor(x)) * 4;
  pixels[offset] = Math.round(pixels[offset] * (1 - alpha) + color[0] * alpha);
  pixels[offset + 1] = Math.round(pixels[offset + 1] * (1 - alpha) + color[1] * alpha);
  pixels[offset + 2] = Math.round(pixels[offset + 2] * (1 - alpha) + color[2] * alpha);
}

function mixColor(a, b, t) {
  return [
    Math.round(a[0] * (1 - t) + b[0] * t),
    Math.round(a[1] * (1 - t) + b[1] * t),
    Math.round(a[2] * (1 - t) + b[2] * t),
  ];
}

function encodePng(width, height, rgba) {
  const stride = width * 4;
  const raw = Buffer.alloc((stride + 1) * height);
  for (let y = 0; y < height; y += 1) {
    raw[y * (stride + 1)] = 0;
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride);
  }
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr(width, height)),
    chunk("IDAT", Buffer.from(pako.deflate(raw, { level: 9 }))),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}

function ihdr(width, height) {
  const buffer = Buffer.alloc(13);
  buffer.writeUInt32BE(width, 0);
  buffer.writeUInt32BE(height, 4);
  buffer[8] = 8;
  buffer[9] = 6;
  buffer[10] = 0;
  buffer[11] = 0;
  buffer[12] = 0;
  return buffer;
}

function chunk(type, data) {
  const typeBuffer = Buffer.from(type, "ascii");
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(Buffer.concat([typeBuffer, data])), 0);
  return Buffer.concat([length, typeBuffer, data, crc]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}
