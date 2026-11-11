// Holds the viewer's canvas to the PNG `agentdrafter render` draws of the
// same sheet at the same view, as ticket #11 held the PNG to AutoCAD's
// plot: for every inked pixel of each image, how far the nearest inked
// pixel of the other is. Also the share of pixels whose colour differs by
// more than a quarter of the range in any channel.

import { crc32, deflateSync, inflateSync } from 'node:zlib';

/** A PNG of RGBA pixels, to look at what the viewer drew. */
export function encodePng(rgba, width, height) {
  const chunk = (type, body) => {
    const head = Buffer.alloc(8);
    head.writeUInt32BE(body.length, 0);
    head.write(type, 4, 'latin1');
    const tail = Buffer.alloc(4);
    tail.writeUInt32BE(crc32(Buffer.concat([head.subarray(4), body])) >>> 0, 0);
    return Buffer.concat([head, body, tail]);
  };
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width, 0);
  header.writeUInt32BE(height, 4);
  header.set([8, 6, 0, 0, 0], 8);
  const raw = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (width * 4 + 1)] = 0;
    raw.set(rgba.subarray(y * width * 4, (y + 1) * width * 4), y * (width * 4 + 1) + 1);
  }
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', header),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

/** The pixels of a PNG `agentdrafter render` wrote: 8-bit RGB or RGBA, not interlaced. */
export function decodePng(bytes) {
  const signature = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
  if (!signature.every((b, i) => bytes[i] === b)) throw new Error('not a PNG');
  let at = 8;
  let width = 0;
  let height = 0;
  let channels = 0;
  const data = [];
  while (at < bytes.length) {
    const length = bytes.readUInt32BE(at);
    const type = bytes.toString('latin1', at + 4, at + 8);
    const body = bytes.subarray(at + 8, at + 8 + length);
    if (type === 'IHDR') {
      width = body.readUInt32BE(0);
      height = body.readUInt32BE(4);
      const [depth, color, , , interlace] = body.subarray(8, 13);
      if (depth !== 8 || interlace !== 0 || (color !== 2 && color !== 6)) {
        throw new Error(`a PNG of depth ${depth}, colour type ${color}, interlace ${interlace} is not read here`);
      }
      channels = color === 2 ? 3 : 4;
    } else if (type === 'IDAT') {
      data.push(body);
    } else if (type === 'IEND') {
      break;
    }
    at += 12 + length;
  }
  const raw = inflateSync(Buffer.concat(data));
  const stride = width * channels;
  const out = new Uint8Array(width * height * 4);
  let previous = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const row = new Uint8Array(stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? row[x - channels] : 0;
      const b = previous[x];
      const c = x >= channels ? previous[x - channels] : 0;
      let v = line[x];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a);
        const pb = Math.abs(p - b);
        const pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      row[x] = v & 0xff;
    }
    for (let x = 0; x < width; x++) {
      out[(y * width + x) * 4] = row[x * channels];
      out[(y * width + x) * 4 + 1] = row[x * channels + 1];
      out[(y * width + x) * 4 + 2] = row[x * channels + 2];
      out[(y * width + x) * 4 + 3] = 255;
    }
    previous = row;
  }
  return { width, height, rgba: out };
}

/** A pixel is ink when its darkest channel is below this: a quarter of the way from white. */
export const INK = 192;

/**
 * A pixel of the other image matches ink when its darkest channel is below
 * this, an eighth of the way from white, so that a grey hairline one image
 * draws at 191 and the other at 192 still matches.
 */
export const MATCH = 224;

const inked = (rgba, width, height, below = INK) => {
  const ink = new Uint8Array(width * height);
  for (let i = 0; i < width * height; i++) {
    const darkest = Math.min(rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
    ink[i] = darkest < below ? 1 : 0;
  }
  return ink;
};

/** The offsets within `radius` pixels of a pixel, its own among them. */
const within = (radius) => {
  const offsets = [];
  const r = Math.ceil(radius);
  for (let dy = -r; dy <= r; dy++) {
    for (let dx = -r; dx <= r; dx++) {
      if (dx * dx + dy * dy <= radius * radius) offsets.push([dx, dy]);
    }
  }
  return offsets;
};

/** The share of `from`'s ink with ink of `to` within `radius` pixels. */
const near = (from, to, width, height, radius) => {
  const offsets = within(radius);
  let ink = 0;
  let found = 0;
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      if (!from[y * width + x]) continue;
      ink++;
      for (const [dx, dy] of offsets) {
        const u = x + dx;
        const v = y + dy;
        if (u >= 0 && v >= 0 && u < width && v < height && to[v * width + u]) {
          found++;
          break;
        }
      }
    }
  }
  return { ink, share: ink ? found / ink : 1 };
};

/**
 * How closely two images of the same size agree: `a` the PNG's pixels and
 * `b` the viewer's, each RGBA.
 */
export function compare(a, b, width, height) {
  const inkA = inked(a, width, height);
  const inkB = inked(b, width, height);
  const matchA = inked(a, width, height, MATCH);
  const matchB = inked(b, width, height, MATCH);
  const png1 = near(inkA, matchB, width, height, 1);
  const png2 = near(inkA, matchB, width, height, 2);
  const view1 = near(inkB, matchA, width, height, 1);
  const view2 = near(inkB, matchA, width, height, 2);
  let differing = 0;
  let total = 0;
  for (let i = 0; i < width * height; i++) {
    let most = 0;
    for (let c = 0; c < 3; c++) {
      const d = Math.abs(a[i * 4 + c] - b[i * 4 + c]);
      total += d;
      most = Math.max(most, d);
    }
    if (most > 64) differing++;
  }
  return {
    width,
    height,
    png_ink_pixels: png1.ink,
    viewer_ink_pixels: view1.ink,
    png_ink_within_1px: png1.share,
    png_ink_within_2px: png2.share,
    viewer_ink_within_1px: view1.share,
    viewer_ink_within_2px: view2.share,
    pixels_differing_by_more_than_64: differing / (width * height),
    mean_channel_difference: total / (width * height * 3),
  };
}
