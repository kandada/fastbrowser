"use strict";

/**
 * fastbrowser — Node.js binding (N-API over the C ABI).
 * Usage: const fb = require('./index.js'); fb.init({engine:'mock'});
 */

let native = null;
try {
  native = require('./build/Release/fastbrowser_native.node');
} catch (e) {
  native = require('./build/Debug/fastbrowser_native.node');
}

function parse(json) {
  const data = JSON.parse(json);
  if (data && data.error) {
    const err = new Error(data.error.message);
    err.kind = data.error.kind;
    throw err;
  }
  return data;
}

class FastBrowser {
  constructor(config = {}) {
    native.init(JSON.stringify(config));
  }

  static version() {
    return native.version();
  }

  open(url) {
    return parse(native.open(url));
  }
  navigate(url) {
    return parse(native.navigate(url));
  }
  toolList() {
    return parse(native.toolList());
  }
  toolCall(name, params = {}) {
    return parse(native.toolCall(name, JSON.stringify(params)));
  }
  snapshot() {
    return parse(native.snapshot());
  }
  screenshot() {
    return parse(native.screenshot());
  }
  info() {
    return parse(native.getInfo());
  }
  status() {
    return parse(native.status());
  }
  setViewport(w, h) {
    return parse(native.setViewport(w, h));
  }
  shutdown() {
    return parse(native.shutdown());
  }
}

module.exports = { FastBrowser };
