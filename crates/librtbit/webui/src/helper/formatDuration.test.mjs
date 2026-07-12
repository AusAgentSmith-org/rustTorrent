// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";

import { formatDuration } from "./formatDuration.js";

test("formatDuration keeps sub-minute seconds", () => {
  assert.equal(formatDuration(15), "15s");
});

test("formatDuration includes minute remainder seconds", () => {
  assert.equal(formatDuration(65), "1m5s");
});

test("formatDuration supports hours", () => {
  assert.equal(formatDuration(3661), "1h1m1s");
});
