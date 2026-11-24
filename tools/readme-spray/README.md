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
