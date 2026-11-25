### Added

- Every keyboard shortcut and mouse binding can be changed in the new Shortcuts window (Settings → Edit shortcuts…, the F1 sheet, or the palette: "Edit keyboard and mouse shortcuts"). On the Keyboard tab, find a command, click Add key and press the keys. On the Mouse tab, pick a preset or change each binding's action, button, gesture and the keys held.
- A key or mouse input that another command already has shows a warning that names it, with Replace (move it here) and Cancel. Nothing changes until you choose.
- Your keys are saved under `[keys]` in settings.toml, holding only the commands you changed, so new built-in keys of later versions still arrive. You can edit them there by hand: `"view.fit" = ["F", "Shift+F"]`. A clash or a mistyped key in the file is reported at start. Mouse bindings stay in navigation.toml.
- The same is the tool `settings.set_keys` for `murmel call` and MCP. It refuses a taken key (SET004) unless `replace` is true.
- Tooltips, menus and tree rows name the keys you set, not the built-in ones.

### Fixed

- `murmel call` and the MCP server kept the "offer to convert large meshes" setting when they rewrote settings.toml.
