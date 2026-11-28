- **Injector devices.** FoehnCraft's `spray/devices.vtu` (its injector and
  nozzle devices, D343 to D345) draws with the mesh and under the parcels:
  the placed tip shaded like the mesh (a 35° crease angle keeps its rims
  sharp), a line along each hole and the outline of each injector's cone.
  `series.open` takes the devices that `series.json` lists under
  `static.devices` (or the folder's `spray/devices.vtu`) and keeps them at
  every step of the player. Tools `devices.open`, `devices.set` (tip,
  hole axes, cones), `devices.clear`; a model-tree row, palette and menu
  entries, help page; sessions keep them (format 1.20).
