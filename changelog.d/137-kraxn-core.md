- **Kraxn, the installer (core and text mode).** `kraxn` installs
  MurmelCraft, KoflCraft and FoehnCraft for you or for every user, from the
  GitHub releases or a folder of downloads, checks every file against
  `SHA256SUMS.txt`, keeps several versions side by side with a default,
  puts the programs on the PATH and removes them again. The text mode asks
  in a terminal, `kraxn apply plan.toml` repeats an install, and only the
  steps that write into system folders ask for administrator rights
  ([docs/kraxn.md](docs/kraxn.md)).
