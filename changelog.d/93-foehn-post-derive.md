### Added

- The Derived fields window lists FoehnCraft's own fields under Wall and Turbulence: y⁺, wall shear stress, the wall treatment's u_τ, y* and blending, the turbulence features, the wall normal and the curvature correction. One the result lacks is computed by the `foehn` installed on your machine (`foehn post derive`, in the background, with Cancel and Esc) and read in as the next step of the same result, so the view and your derived fields stay. The row is greyed with the reason when foehn, the restart file or the case is not found.
- `field.foehn_derive` does the same for scripts and agents (over MCP it needs `output` or `file`). New codes FIELD018 to FIELD020.
