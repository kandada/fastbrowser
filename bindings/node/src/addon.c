/*
 * fastbrowser N-API addon — thin translation of the C ABI to JS.
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 */
#include <node_api.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "fastbrowser.h"

static napi_value fb_err(napi_env env, const char *msg) {
  napi_throw_error(env, "FB_ERR", msg);
  return NULL;
}

/* 调用返回 JSON 字符串的 C 函数，自动释放并转为 JS string。 */
static napi_value fb_call_json(napi_env env, char *(*fn)(void)) {
  char *r = fn();
  if (r == NULL) return fb_err(env, "fastbrowser returned NULL");
  napi_value out;
  napi_status st = napi_create_string_utf8(env, r, NAPI_AUTO_LENGTH, &out);
  fastbrowser_free_string(r);
  if (st != napi_ok) return fb_err(env, "napi_create_string_utf8 failed");
  return out;
}

static napi_value fb_version(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_version);
}

static napi_value fb_init(napi_env env, napi_callback_info info) {
  size_t argc = 1;
  napi_value argv[1];
  napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
  char cfg[4096] = "{}";
  if (argc >= 1 && argv[0] != NULL) {
    size_t len = 0;
    napi_get_value_string_utf8(env, argv[0], NULL, 0, &len);
    if (len < sizeof(cfg)) {
      napi_get_value_string_utf8(env, argv[0], cfg, sizeof(cfg), &len);
    }
  }
  char *r = fastbrowser_init(cfg);
  if (r == NULL) return fb_err(env, "fastbrowser returned NULL");
  napi_value out;
  napi_status st = napi_create_string_utf8(env, r, NAPI_AUTO_LENGTH, &out);
  fastbrowser_free_string(r);
  if (st != napi_ok) return fb_err(env, "napi_create_string_utf8 failed");
  return out;
}

static napi_value fb_str_arg(napi_env env, napi_callback_info info, char *buf, size_t bufsz,
                             char *(*fn)(const char *)) {
  size_t argc = 1;
  napi_value argv[1];
  napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
  if (argc < 1 || argv[0] == NULL) return fb_err(env, "missing argument");
  size_t len = 0;
  napi_get_value_string_utf8(env, argv[0], NULL, 0, &len);
  if (len >= bufsz) len = bufsz - 1;
  napi_get_value_string_utf8(env, argv[0], buf, bufsz, &len);
  char *r = fn(buf);
  if (r == NULL) return fb_err(env, "fastbrowser returned NULL");
  napi_value out;
  napi_status st = napi_create_string_utf8(env, r, NAPI_AUTO_LENGTH, &out);
  fastbrowser_free_string(r);
  if (st != napi_ok) return fb_err(env, "napi_create_string_utf8 failed");
  return out;
}

static napi_value fb_open(napi_env env, napi_callback_info info) {
  char url[8192];
  return fb_str_arg(env, info, url, sizeof(url), fastbrowser_open);
}

static napi_value fb_navigate(napi_env env, napi_callback_info info) {
  char url[8192];
  return fb_str_arg(env, info, url, sizeof(url), fastbrowser_navigate);
}

static napi_value fb_tool_call(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2];
  napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
  if (argc < 1 || argv[0] == NULL) return fb_err(env, "missing tool name");
  char name[128];
  char params[16384] = "{}";
  size_t len = 0;
  napi_get_value_string_utf8(env, argv[0], NULL, 0, &len);
  if (len >= sizeof(name)) len = sizeof(name) - 1;
  napi_get_value_string_utf8(env, argv[0], name, sizeof(name), &len);
  if (argc >= 2 && argv[1] != NULL) {
    napi_get_value_string_utf8(env, argv[1], NULL, 0, &len);
    if (len < sizeof(params)) {
      napi_get_value_string_utf8(env, argv[1], params, sizeof(params), &len);
    }
  }
  char *r = fastbrowser_tool_call(name, params);
  if (r == NULL) return fb_err(env, "fastbrowser returned NULL");
  napi_value out;
  napi_status st = napi_create_string_utf8(env, r, NAPI_AUTO_LENGTH, &out);
  fastbrowser_free_string(r);
  if (st != napi_ok) return fb_err(env, "napi_create_string_utf8 failed");
  return out;
}

static napi_value fb_tool_list(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_tool_list);
}
static napi_value fb_snapshot(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_snapshot);
}
static napi_value fb_screenshot(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_screenshot);
}
static napi_value fb_get_info(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_get_info);
}
static napi_value fb_status(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_status);
}
static napi_value fb_shutdown(napi_env env, napi_callback_info info) {
  return fb_call_json(env, fastbrowser_shutdown);
}

static napi_value fb_set_viewport(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2];
  napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
  uint32_t w = 0, h = 0;
  if (argc >= 1) napi_get_value_uint32(env, argv[0], &w);
  if (argc >= 2) napi_get_value_uint32(env, argv[1], &h);
  char *r = fastbrowser_set_viewport(w, h);
  if (r == NULL) return fb_err(env, "fastbrowser returned NULL");
  napi_value out;
  napi_status st = napi_create_string_utf8(env, r, NAPI_AUTO_LENGTH, &out);
  fastbrowser_free_string(r);
  if (st != napi_ok) return fb_err(env, "napi_create_string_utf8 failed");
  return out;
}

static napi_value fb_set_permission(napi_env env, napi_callback_info info) {
  size_t argc = 2;
  napi_value argv[2];
  napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
  char res[256] = "";
  if (argc >= 1 && argv[0] != NULL) {
    size_t len = 0;
    napi_get_value_string_utf8(env, argv[0], NULL, 0, &len);
    if (len >= sizeof(res)) len = sizeof(res) - 1;
    napi_get_value_string_utf8(env, argv[0], res, sizeof(res), &len);
  }
  bool allowed = true;
  if (argc >= 2) napi_get_value_bool(env, argv[1], &allowed);
  fastbrowser_set_permission(res, allowed ? 1 : 0);
  napi_value undef;
  napi_get_undefined(env, &undef);
  return undef;
}

NAPI_MODULE_INIT() {
  napi_property_descriptor props[] = {
    {"version", NULL, fb_version, NULL, NULL, NULL, napi_default, NULL},
    {"init", NULL, fb_init, NULL, NULL, NULL, napi_default, NULL},
    {"open", NULL, fb_open, NULL, NULL, NULL, napi_default, NULL},
    {"navigate", NULL, fb_navigate, NULL, NULL, NULL, napi_default, NULL},
    {"toolList", NULL, fb_tool_list, NULL, NULL, NULL, napi_default, NULL},
    {"toolCall", NULL, fb_tool_call, NULL, NULL, NULL, napi_default, NULL},
    {"snapshot", NULL, fb_snapshot, NULL, NULL, NULL, napi_default, NULL},
    {"screenshot", NULL, fb_screenshot, NULL, NULL, NULL, napi_default, NULL},
    {"getInfo", NULL, fb_get_info, NULL, NULL, NULL, napi_default, NULL},
    {"status", NULL, fb_status, NULL, NULL, NULL, napi_default, NULL},
    {"shutdown", NULL, fb_shutdown, NULL, NULL, NULL, napi_default, NULL},
    {"setViewport", NULL, fb_set_viewport, NULL, NULL, NULL, napi_default, NULL},
    {"setPermission", NULL, fb_set_permission, NULL, NULL, NULL, napi_default, NULL},
  };
  napi_status st = napi_define_properties(env, exports,
                                          sizeof(props) / sizeof(props[0]), props);
  if (st != napi_ok) {
    napi_throw_error(env, "FB_ERR", "napi_define_properties failed");
    return NULL;
  }
  return exports;
}
