import assert from 'node:assert/strict';
import { test } from 'node:test';

import { compare, decodePng, encodePng } from '../e2e/compare.mjs';

/** A white image with black pixels at `inked`, each [x, y]. */
const image = (width, height, inked, grey = 0) => {
  const rgba = new Uint8Array(width * height * 4).fill(255);
  for (const [x, y] of inked) rgba.fill(grey, (y * width + x) * 4, (y * width + x) * 4 + 3);
  return rgba;
};

test('a PNG written is read back the same', () => {
  const rgba = image(5, 3, [[1, 1], [4, 2]]);
  const back = decodePng(encodePng(rgba, 5, 3));
  assert.equal(back.width, 5);
  assert.equal(back.height, 3);
  assert.deepEqual(back.rgba, rgba);
});

test('ink one pixel off counts within one pixel; two off, within two only', () => {
  const line = Array.from({ length: 10 }, (_, x) => [x, 5]);
  const same = compare(image(10, 10, line), image(10, 10, line), 10, 10);
  assert.equal(same.png_ink_within_1px, 1);
  assert.equal(same.pixels_differing_by_more_than_64, 0);
  const one = compare(image(10, 10, line), image(10, 10, line.map(([x, y]) => [x, y + 1])), 10, 10);
  assert.equal(one.png_ink_within_1px, 1);
  const two = compare(image(10, 10, line), image(10, 10, line.map(([x, y]) => [x, y + 2])), 10, 10);
  assert.equal(two.png_ink_within_1px, 0);
  assert.equal(two.png_ink_within_2px, 1);
});

test('a grey hairline just either side of the ink threshold still matches', () => {
  const line = Array.from({ length: 10 }, (_, x) => [x, 5]);
  const result = compare(image(10, 10, line, 191), image(10, 10, line, 192), 10, 10);
  assert.equal(result.png_ink_pixels, 10);
  assert.equal(result.png_ink_within_1px, 1);
});
