### Added

- Agents can drive the open window: `murmel mcp --live --root DIR` runs the same MCP server with the same roots check, but each call runs in your open MurmelCraft window. You watch the agent work, its steps are undo steps (Cmd + Z or Ctrl + Z), and screenshots come back at the viewport's size. See docs/mcp.md, "Drive the open window".
- While an agent is connected, the status bar says "Agent connected" with a Stop button. Settings > Agents has the switch "Let agents drive this window" (on by default, saved in settings.toml) and shows the socket the window listens on. The window listens on a socket only your user account can open (macOS and Linux).
