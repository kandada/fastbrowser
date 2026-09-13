"use strict";
const assert = require("assert");
const { FastBrowser } = require("./index.js");

assert.ok(FastBrowser.version());

const b = new FastBrowser({ engine: "mock" });
const out = b.open("https://example.com");
assert.strictEqual(out.title, "Example Page");

const tools = b.toolList();
assert.ok(tools.length >= 30);

const snap = b.snapshot();
assert.ok(snap.interactive.length > 0);

const links = b.toolCall("extract_links");
assert.ok(links.links.length >= 1);

const shot = b.screenshot();
assert.strictEqual(shot.format, "rgba");

const info = b.info();
assert.strictEqual(info.engine, "mock");

let threw = false;
try {
  b.toolCall("does_not_exist");
} catch (e) {
  threw = true;
  assert.strictEqual(e.kind, "invalid_argument");
}
assert.ok(threw);

b.shutdown();
console.log("node binding tests passed");
