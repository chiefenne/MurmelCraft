### Added

- The Derived fields window computes FoehnCraft's turbomachinery, turbulence and wall quantities itself, with FoehnCraft's names and units: enstrophy and strain rate; relative and frame velocity, the radial, circumferential, axial and meridional velocity, the relative circumferential velocity, the flow angles α and β, the relative total pressure and rothalpy; ε, ω, the turbulence time and length scales, the viscosity ratio ν_t/ν and the six Boussinesq Reynolds stresses; and the exact wall distance.
- Turning zones for the turbomachinery quantities come from FoehnCraft's `case.resolved.toml` beside the result (`[[mrf]]`, `[[sliding]]`, `[output] frame`), else you type them in the window: cell zone, axis point, axis and rpm, several zones, and the frame every cell is seen in. A right-click keeps the speed or the axis as your default (`[defaults.rotation]`).
- The wall distance measures to the case's walls (`type = "wall"`), else to the patches whose role is wall; the window lets you tick others.
- The kinematic viscosity ν and C_μ come from the case too (`[fluid]`, `[turbulence.constants]`), else your default, else air at sea level and 0.09.
- The window shows one group at a time (Vortices, Pressure, Turbomachinery, Turbulence, Wall), the values its quantities take above the rows.
- `field.derive` takes `k`, `dissipation` (ω or ε), `nu_t`, `rotation` and `walls`; `field.catalogue` lists the rotation, the walls, the mesh's zones and patches. New refusals FIELD016 (turning zones) and FIELD017 (walls). Session files are format 1.16.
