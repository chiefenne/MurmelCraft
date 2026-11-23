# English strings of the MurmelCraft window (UI spec: every visible string
# is a key). Keys follow the area of the UI; tool names are tool-<id>.

app-name = MurmelCraft
app-no-file = No mesh open

toolbar-open = Open
toolbar-open-tip = Open a mesh (Ctrl + O)
toolbar-group-view = View
toolbar-fit = Fit
toolbar-fit-tip = Fit the mesh in the view (F)
toolbar-home = Home
toolbar-home-tip = Home view, iso and fitted (Home)
toolbar-projection-tip = Projection (5 cycles the three modes)
toolbar-group-display = Display
toolbar-color-by = Colour by
toolbar-color-by-tip = What gives the boundary faces their colour
toolbar-tree = Model tree
toolbar-tree-tip = Show or hide the model tree (T)
toolbar-properties = Properties
toolbar-properties-tip = Show or hide the properties (P)

color-by-solid = Solid colour
color-by-patch = Patch
color-by-role = Patch role
color-by-cell_type = Cell type

kind-polyhedron = Polyhedron
kind-prism = Prism
kind-hexahedron = Hexahedron
kind-tetrahedron = Tetrahedron
kind-pyramid = Pyramid

role-wall = Wall
role-inlet = Inlet
role-outlet = Outlet
role-symmetry = Symmetry
role-interface = Interface
role-unspecified = Unspecified

tree-filter = Filter by name (Ctrl + F)
tree-empty = No mesh open.
tree-zones = Zones
tree-cell-types = Cell types
tree-patches = Patches
tree-patch-tip = Role: { $role }
tree-show = Show { $item }
tree-hide = Hide { $item }
tree-isolate = Isolate (I)
tree-show-all = Show all (Alt + H)
tree-fit = Fit
tree-copy-name = Copy name
tree-color = Colour
tree-reset-color = Reset colour

validity-valid = Valid
validity-no_evidence = No evidence
validity-failed = Mismatch
validity-tip-valid = Matches the mesh-evidence.json KoflCraft wrote beside it
validity-tip-no_evidence = No mesh-evidence.json beside the mesh
validity-tip-failed = Does not match the mesh-evidence.json beside it:

props-title = Properties
props-empty = Open a mesh to see its properties.
props-area-mesh = Mesh
props-area-zone = Zone
props-area-kind = Cell type
props-area-patch = Patch
props-copy = { $value } (click to copy)
value-yes = yes
value-no = no

prop-file = File
prop-format = Format
prop-koflcraft_version = KoflCraft version
prop-generator = Generator
prop-validity = Evidence
prop-validity_problems = Problems
prop-cells = Cells
prop-faces_interior = Interior faces
prop-faces_boundary = Boundary faces
prop-vertices = Vertices
prop-volume = Volume
prop-volume_error = Relative volume error
prop-inverted_faces = Inverted faces
prop-visible_cells = Visible cells
prop-visible_faces = Visible faces
prop-digest = SHA-256
prop-name = Name
prop-role = Role
prop-foehncraft_type = FoehnCraft type
prop-faces = Faces
prop-area = Area
prop-mean_normal = Mean normal
prop-normal_spread = Normal spread
prop-bounds_min = Bounds min
prop-bounds_max = Bounds max
prop-hidden = Hidden

projection-orthographic = Orthographic
projection-perspective = Perspective
projection-perspective_ortho_faces = Perspective with orthographic faces
projection-short-orthographic = Ortho
projection-short-perspective = Persp
projection-short-perspective_ortho_faces = Persp · ortho faces

settings-title = Settings
settings-tip = Theme and mouse navigation
settings-theme = Theme
theme-dark = Dark
theme-light = Light
theme-system = Follow the system
settings-navigation = Mouse navigation
preset-three_button = Three-button mouse
preset-apple = Apple mouse or trackpad
preset-fusion = Fusion style
preset-paraview = ParaView style
preset-custom = Custom
settings-zoom-to-cursor = Zoom to the cursor
settings-invert-zoom = Invert the zoom direction
settings-colors = Colours
settings-mesh-color = Mesh colour (solid)
settings-reset-colors = Reset to the built-in colours
settings-file = Saved in { $path }

viewport-empty = Open a KoflCraft mesh (foehncraft.vtu) with Open or Ctrl + O, or drop it here.
viewport-description = { $view }, { $cells } cells, { $faces } boundary faces, colour by { $colour }
viewport-view-iso = Iso view
viewport-view-custom = Custom view
menu-fit = Fit
menu-home = Home view
menu-front = Front
menu-top = Top
menu-right = Right

hint-orbit = { $input } to orbit
hint-pan = { $input } to pan
hint-zoom = { $input } to zoom
input-primary = Drag
input-secondary = Right drag
input-middle = Middle drag
input-scroll = Wheel
input-pinch = Pinch
mod-cmd = Cmd
mod-shift = Shift
mod-ctrl = Ctrl
mod-alt = Option

status-cells = Visible { $cells } cells · { $faces } boundary faces
status-frame = { $ms } ms
status-units = m
status-loading = Opening { $file } …
status-error = { $code }: { $message }
status-selected = Selected { $area } { $item }

tool-system-info = Version
tool-system-commands = Tool catalogue
tool-diag-codes = Diagnostic codes
tool-mesh-open = Open mesh
tool-mesh-close = Close mesh
tool-mesh-info = Mesh information
tool-view-get-camera = Camera
tool-view-set-camera = Set camera
tool-view-orbit = Orbit
tool-view-pan = Pan
tool-view-zoom = Zoom
tool-view-fit = Fit view
tool-view-standard = Standard view
tool-view-screenshot = Screenshot
tool-display-color-by = Colour by
tool-display-set-color = Item colour
tool-display-visibility = Show and hide
tool-select-item = Select
tool-select-clear = Clear selection
tool-inspect-item = Properties
tool-settings-get = Settings
tool-settings-set-theme = Theme
tool-settings-set-colors = Mesh colours
tool-settings-set-bindings = Mouse bindings

display-shaded-edges = Shaded with edges
