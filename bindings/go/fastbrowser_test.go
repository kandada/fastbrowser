package fastbrowser

import (
	"encoding/json"
	"testing"
)

func TestVersion(t *testing.T) {
	if Version() == "" {
		t.Fatal("version empty")
	}
}

func TestLifecycle(t *testing.T) {
	if _, err := Init(map[string]interface{}{"engine": "mock"}); err != nil {
		t.Fatal(err)
	}
	out, err := Open("https://example.com")
	if err != nil {
		t.Fatal(err)
	}
	var v struct {
		Title string `json:"title"`
	}
	if err := json.Unmarshal(out, &v); err != nil {
		t.Fatal(err)
	}
	if v.Title != "Example Page" {
		t.Fatalf("unexpected title: %s", v.Title)
	}
	tools, err := ToolList()
	if err != nil {
		t.Fatal(err)
	}
	var list []map[string]interface{}
	if err := json.Unmarshal(tools, &list); err != nil {
		t.Fatal(err)
	}
	if len(list) < 30 {
		t.Fatalf("expected >=30 tools, got %d", len(list))
	}
	if _, err := Shutdown(); err != nil {
		t.Fatal(err)
	}
}

func TestUnknownToolIsError(t *testing.T) {
	Init(map[string]interface{}{"engine": "mock"})
	defer Shutdown()
	if _, err := ToolCall("nope", nil); err == nil {
		t.Fatal("expected error")
	}
}
