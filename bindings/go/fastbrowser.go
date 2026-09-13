// Package fastbrowser 提供 fastbrowser 内核的 Go 绑定（cgo 调用 C ABI）。
//
// 构建：
//   cargo build --release
//   CGO_LDFLAGS="-L ../../target/release -lfastbrowser" go build ./...
package fastbrowser

/*
#cgo LDFLAGS: -L${SRCDIR}/../../target/release -lfastbrowser
#include <stdlib.h>
#include "fastbrowser.h"
*/
import "C"

import (
	"encoding/json"
	"errors"
	"unsafe"
)

// Error 表示内核返回的错误。
type Error struct {
	Kind    string `json:"kind"`
	Message string `json:"message"`
}

func (e *Error) Error() string { return e.Message }

func call(fn func() *C.char) (json.RawMessage, error) {
	ptr := fn()
	if ptr == nil {
		return nil, errors.New("fastbrowser returned NULL")
	}
	defer C.fastbrowser_free_string(ptr)
	data := C.GoString(ptr)
	var v struct {
		Error *Error `json:"error"`
	}
	if err := json.Unmarshal([]byte(data), &v); err != nil {
		return nil, err
	}
	if v.Error != nil {
		return nil, v.Error
	}
	return json.RawMessage(data), nil
}

func callStr(fn func(*C.char) *C.char, s string) (json.RawMessage, error) {
	cs := C.CString(s)
	defer C.free(unsafe.Pointer(cs))
	return call(func() *C.char { return fn(cs) })
}

// Version 返回内核版本。
func Version() string {
	ptr := C.fastbrowser_version()
	if ptr == nil {
		return ""
	}
	defer C.fastbrowser_free_string(ptr)
	return C.GoString(ptr)
}

// Init 初始化内核。
func Init(config map[string]interface{}) (json.RawMessage, error) {
	b, _ := json.Marshal(config)
	cs := C.CString(string(b))
	defer C.free(unsafe.Pointer(cs))
	return call(func() *C.char { return C.fastbrowser_init(cs) })
}

// Open 打开 URL。
func Open(url string) (json.RawMessage, error) {
	return callStr(C.fastbrowser_open, url)
}

// Navigate 导航。
func Navigate(url string) (json.RawMessage, error) {
	return callStr(C.fastbrowser_navigate, url)
}

// ToolList 返回工具清单。
func ToolList() (json.RawMessage, error) { return call(C.fastbrowser_tool_list) }

// ToolCall 调用工具。
func ToolCall(name string, params map[string]interface{}) (json.RawMessage, error) {
	b, _ := json.Marshal(params)
	cn := C.CString(name)
	cp := C.CString(string(b))
	defer C.free(unsafe.Pointer(cn))
	defer C.free(unsafe.Pointer(cp))
	return call(func() *C.char { return C.fastbrowser_tool_call(cn, cp) })
}

// Snapshot 返回页面快照。
func Snapshot() (json.RawMessage, error) { return call(C.fastbrowser_snapshot) }

// Screenshot 返回截图。
func Screenshot() (json.RawMessage, error) { return call(C.fastbrowser_screenshot) }

// GetInfo 返回内核信息。
func GetInfo() (json.RawMessage, error) { return call(C.fastbrowser_get_info) }

// Status 返回状态。
func Status() (json.RawMessage, error) { return call(C.fastbrowser_status) }

// SetViewport 设置视口。
func SetViewport(w, h uint32) (json.RawMessage, error) {
	return call(func() *C.char { return C.fastbrowser_set_viewport(C.uint32_t(w), C.uint32_t(h)) })
}

// Shutdown 关闭内核。
func Shutdown() (json.RawMessage, error) { return call(C.fastbrowser_shutdown) }
