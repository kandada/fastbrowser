{
  "targets": [
    {
      "target_name": "fastbrowser_native",
      "sources": ["src/addon.c"],
      "include_dirs": ["<(module_root_dir)/../../fastbrowser_c/include", "src"],
      "conditions": [
        ["OS=='mac'", { "libraries": ["-L<(module_root_dir)/../../target/release", "-lfastbrowser"] }],
        ["OS=='linux'", { "libraries": ["-L<(module_root_dir)/../../target/release", "-lfastbrowser"] }],
        ["OS=='win'", { "libraries": ["-L<(module_root_dir)/../../target/release", "-lfastbrowser"] }]
      ]
    }
  ]
}
