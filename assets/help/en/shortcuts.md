+++
covers = ["window.shortcuts", "window.edit_shortcuts", "settings.set_keys", "settings.set_bindings*"]
related = ["palette", "settings"]
chapter = "getting-started"
+++
Every key and mouse binding can be changed. **Keyboard shortcuts** lists the keys, and **Edit keyboard and mouse shortcuts…** changes them, with a warning when a key is already taken.

<!-- more -->

## Shortcuts

The Shortcuts window has two tabs:

- **Keyboard**: click a command's key and press the new one. A key that another command already has is refused, unless you choose Replace.
- **Mouse**: orbit, pan, zoom and zoom to box on any button with any modifier. Presets: three-button (the default), Apple mouse, Fusion and ParaView.

Your keys are written to `[keys]` in settings.toml, and only the commands you changed are listed there. Mouse bindings go to navigation.toml. On macOS, Ctrl in the tables is the Command key and Alt the Option key. Every tip in the window shows your keys, not the built-in ones.
