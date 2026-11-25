+++
covers = ["field.style*", "field.list", "field.catalogue", "ui.field_bar"]
related = ["color-by", "derived"]
chapter = "results"
picture = "field-style.png"
shot = "shots/field-style.json"
+++
How a field is coloured: flat per cell, smooth, or banded, with contour lines on or off. The colour map and its range (whole field, shown cells, symmetric about zero, fixed, or over all time steps) are chosen here.

<!-- more -->

## Field colouring

- **Flat** shows the value of each cell. **Smooth** interpolates to the nodes from least-squares cell gradients. **Banded** draws steps of the colour map.
- **Contour lines** draw iso-lines on the surface.
- **Range**: the whole field, the cells shown, symmetric about zero, fixed as it is now, or over all steps of a series.
- **Colour map**: Rainbow desaturated is the default, Inferno second, Rainbow uniform for ranges centred on zero. *Automatic* picks one by the range.
- **Reverse the colour map**, and **Field: default style** to start again.

Click the colour bar to change its settings.
