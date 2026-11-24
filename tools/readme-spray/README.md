# The spray picture of the README

`assets/readme/spray-parcels.png` is FoehnCraft's `examples/cases/flow/spray_duct.toml`
(FoehnCraft main cc83898) with `[spray] write_every = 5` and
`[output] fields_every = 10`, at step 300. In the run folder:

    foehn run spray_duct.toml
    murmel run --script /path/to/MurmelCraft/tools/readme-spray/spray-parcels.json

The gas is a slice at z coloured by |velocity|, the parcels are coloured
by their own |velocity| (inferno), seen from the top.
