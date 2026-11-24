# The spray pictures of the README

`assets/readme/spray-parcels.png` and `assets/readme/spray-series.gif`
are FoehnCraft's `examples/cases/flow/spray_duct.toml` (FoehnCraft main
cc83898) with `[spray] write_every = 5` and `[output] fields_every = 10`.
In the run folder:

    foehn run spray_duct.toml
    murmel run --script /path/to/MurmelCraft/tools/readme-spray/spray-parcels.json
    murmel run --script /path/to/MurmelCraft/tools/readme-spray/spray-series.json

The picture is step 300; the movie is every step of the series (the
folder's numbered files), one frame each at 10 frames a second. The gas
is a slice at z coloured by |velocity|, the parcels are coloured by their
own |velocity| (inferno), seen from the top.

`assets/readme/spray-charts.png` is FoehnCraft's gallery case
`examples/cases/flow/spray_datums.toml` (FoehnCraft pull request #265,
`8c1efc5`), run to an output folder, drawn by the window test
`readme_chart_panel` (the whole window with wgpu, lavapipe on Linux):

    foehn flow examples/cases/flow/spray_datums.toml --out spray_datums
    MURMEL_README_RUN=$PWD/spray_datums MURMEL_README_OUT=$PWD/spray-charts.png \
      cargo test -p murmel-app readme_chart_panel -- --ignored

The test opens `fields.vtkhdf`, the series at step 150 (which opens the
charts of the folder), a slice at z coloured by |velocity|, the parcels
coloured by diameter, the top view and the mass-flow chart, and hides
the side panels. The same charts without a window:

    murmel run --script tools/readme-spray/spray-charts.json

in the run folder writes `mass_flow.png`, `penetration.png`,
`diameters.png`, `histogram_outlet.png` and `statistics` in the result.
