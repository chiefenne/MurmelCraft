+++
covers = ["window.derived", "window.calculator", "field.derive*", "field.calculate", "settings.keep_formula", "settings.forget_formula"]
related = ["field-style", "iso"]
chapter = "results"
+++
Compute new fields from the results: vorticity, Q criterion, λ₂, total pressure, pressure coefficient, turbomachinery velocities and more. The field calculator takes your own formulas.

<!-- more -->

## Derived fields and the calculator

**Derived fields…** lists the quantities that can be computed from the fields of the result, with the reference values they need (density, velocity, pressure, rotation). Gradients come from least squares over each cell's neighbours.

**Field calculator…** evaluates a formula over the cells: `mag(U)`, `p / (0.5 * rho * mag(U)^2)`, `mag(curl(U))`. Formulas you keep are saved in settings.toml and offered again.
