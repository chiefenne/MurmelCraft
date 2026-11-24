//! The registry of diagnostic codes. A code names one meaning forever and
//! is never reused. Mesh and render codes come from their crates.

use std::sync::OnceLock;

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Code {
    pub code: &'static str,
    pub severity: Severity,
    pub meaning: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
}

const OWN: &[Code] = &[
    Code {
        code: "TOOL001",
        severity: Severity::Error,
        meaning: "No tool has this id.",
        fix: Some("Run `murmel commands` for the ids."),
    },
    Code {
        code: "TOOL002",
        severity: Severity::Error,
        meaning: "The input does not match the tool's input schema.",
        fix: Some("Compare the input with the schema in `murmel commands`."),
    },
    Code {
        code: "VIEW001",
        severity: Severity::Error,
        meaning: "The tool needs an open mesh and none is open.",
        fix: Some("Call mesh.open first, or pass `mesh` to view.screenshot."),
    },
    Code {
        code: "VIEW002",
        severity: Severity::Error,
        meaning: "No item of the open mesh has this id.",
        fix: Some(
            "Use `mesh`, `zone:<name>`, `kind:<cell type>` or `patch:<name>`; mesh.info lists the names.",
        ),
    },
    Code {
        code: "VIEW003",
        severity: Severity::Error,
        meaning: "No saved view of the open mesh has this name.",
        fix: Some("views.list names the saved views."),
    },
    Code {
        code: "VIEW004",
        severity: Severity::Error,
        meaning: "The view name is empty, longer than 64 characters, holds control characters, or another saved view has it.",
        fix: Some("Choose another name, or delete the other view first."),
    },
    Code {
        code: "CLIP001",
        severity: Severity::Error,
        meaning: "No clip plane has this number.",
        fix: Some("Clip planes count from 1 (clip:1); the clips list of the result names them."),
    },
    Code {
        code: "CLIP002",
        severity: Severity::Error,
        meaning: "The display has the most clip planes it holds (8).",
        fix: Some("Remove a clip plane with clip.clear, or change one with clip.set."),
    },
    Code {
        code: "SLICE001",
        severity: Severity::Error,
        meaning: "No slice has this number.",
        fix: Some("Slices count from 1 (slice:1); the slices list of the result names them."),
    },
    Code {
        code: "SLICE002",
        severity: Severity::Error,
        meaning: "The display has the most slices it holds (8; a series counts once).",
        fix: Some(
            "Remove a slice with slice.clear, change one with slice.set, or make a series (count).",
        ),
    },
    Code {
        code: "SLICE003",
        severity: Severity::Error,
        meaning: "A series has a count outside 1 to 64 or a spacing that is not positive.",
        fix: Some(
            "Give count 1 to 64, and a positive spacing or none (the planes then spread through the mesh).",
        ),
    },
    Code {
        code: "ISO001",
        severity: Severity::Error,
        meaning: "No iso-surface has this number.",
        fix: Some("Iso-surfaces count from 1 (iso:1); the isos list of the result names them."),
    },
    Code {
        code: "ISO002",
        severity: Severity::Error,
        meaning: "The display has the most iso-surfaces it holds (8).",
        fix: Some("Remove one with iso.clear, or change one with iso.set."),
    },
    Code {
        code: "ARROW001",
        severity: Severity::Error,
        meaning: "Arrows need a vector field (one with x, y and z components), and the field named or the open mesh has none.",
        fix: Some(
            "Name a vector field (field.list shows each field's components), or open a result with velocity.",
        ),
    },
    Code {
        code: "REGION001",
        severity: Severity::Error,
        meaning: "No region of interest has this number.",
        fix: Some("Regions count from 1 (region:1); region.list names them."),
    },
    Code {
        code: "REGION002",
        severity: Severity::Error,
        meaning: "The region's shape is broken (a size, radius or thickness that is not positive, a zero normal, too many rings) or names a zone, patch or cell the mesh does not have.",
        fix: Some(
            "mesh.info lists the zones and patches; cells count from 0 to the cell count minus one; sizes must be positive.",
        ),
    },
    Code {
        code: "REGION003",
        severity: Severity::Error,
        meaning: "The display has the most regions it holds (16).",
        fix: Some("Remove a region with region.clear, or change one with region.set."),
    },
    Code {
        code: "ARROW002",
        severity: Severity::Error,
        meaning: "The arrows' density is outside 4 to 200, or their scale is not above 0 and up to 100.",
        fix: Some(
            "Give density 4 to 200 (arrows across the mesh's largest extent) and a scale above 0, up to 100.",
        ),
    },
    Code {
        code: "PARCEL001",
        severity: Severity::Error,
        meaning: "No parcels are open.",
        fix: Some(
            "Open a parcel file (FoehnCraft's spray/parcels_<step>.vtkhdf) with parcels.open.",
        ),
    },
    Code {
        code: "PARCEL002",
        severity: Severity::Error,
        meaning: "The parcel file has no field of this name (to colour by, to size by diameter or to filter by diameter or injector).",
        fix: Some(
            "The message lists the fields of the file; parcels.open's result lists them too.",
        ),
    },
    Code {
        code: "PARCEL003",
        severity: Severity::Error,
        meaning: "A parcel setting is out of range: a scale not above 0 or above 100, a diameter range not 0 <= min <= max, an unknown colour map or a fixed range not min < max.",
        fix: Some(
            "Give a scale above 0 up to 100, diameters in metres with min <= max, a map of field.style's list or auto, and min < max.",
        ),
    },
    Code {
        code: "SERIES001",
        severity: Severity::Error,
        meaning: "The time series cannot be read: the path does not exist, the manifest is not foehncraft.series 1 or names a step twice or a step without files, or the folder holds no numbered files.",
        fix: Some(
            "Give FoehnCraft's series.json or its output folder (fields_<step>.vtkhdf, spray/parcels_<step>.vtkhdf).",
        ),
    },
    Code {
        code: "SERIES002",
        severity: Severity::Error,
        meaning: "No time series is open.",
        fix: Some("Open a series.json or an output folder with series.open."),
    },
    Code {
        code: "SERIES003",
        severity: Severity::Error,
        meaning: "The series has no such step, index or time, or the rate is outside 0.1 to 60 steps a second.",
        fix: Some("series.open's result gives the count, the first and last step and the times."),
    },
    Code {
        code: "STREAM001",
        severity: Severity::Error,
        meaning: "No streamline set has this number, or the display has the most it holds (8).",
        fix: Some(
            "Sets count from 1 (stream:1); the streams list of the result names them. Remove one with stream.clear to add another.",
        ),
    },
    Code {
        code: "STREAM002",
        severity: Severity::Error,
        meaning: "A streamline setting is out of range (count outside 1 to 1000, a length or radius not above 0, a line of one point, a zero normal), names a patch the mesh does not have, or belongs to another seed shape than the set's.",
        fix: Some(
            "Give count 1 to 1000 and positive lengths; mesh.info lists the patches; name the shape with seed to change it (line: from, to; sphere: centre, radius; patch: patch; plane: origin, normal).",
        ),
    },
    Code {
        code: "STREAM003",
        severity: Severity::Error,
        meaning: "Streamlines follow a vector field (one with x, y and z components), and the field named or the open mesh has none.",
        fix: Some(
            "Name a vector field (field.list shows each field's components), or open a result with velocity.",
        ),
    },
    Code {
        code: "MCP001",
        severity: Severity::Error,
        meaning: "The MCP server refused a path outside its read or write roots.",
        fix: Some(
            "Use a path below a --read-root (files read) or a --write-root (files written), or start the server with more roots.",
        ),
    },
    Code {
        code: "MCP002",
        severity: Severity::Error,
        meaning: "The MCP server could not resolve a path (it does not exist, or its folder does not).",
        fix: Some("Check the path; relative paths resolve from the first --write-root."),
    },
    Code {
        code: "LIVE001",
        severity: Severity::Error,
        meaning: "No MurmelCraft window listens for agents (none is open, agents are switched off in its Settings, or the platform has no live bridge yet).",
        fix: Some(
            "Open MurmelCraft and switch on Settings > Agents > Let agents drive this window, or start the MCP server without --live for a headless session.",
        ),
    },
    Code {
        code: "LIVE002",
        severity: Severity::Error,
        meaning: "The window did not answer the call: the person stopped the agents, the window closed, or it took longer than two minutes.",
        fix: Some("Check the window; the next call connects again."),
    },
    Code {
        code: "LIVE003",
        severity: Severity::Error,
        meaning: "The window could not listen for agents: another window listens already, or the socket folder is not private.",
        fix: Some(
            "Close the other window, or make the socket folder private (chmod 700); MURMEL_LIVE_SOCKET names another socket.",
        ),
    },
    Code {
        code: "SET001",
        severity: Severity::Error,
        meaning: "A settings file could not be written, or no config folder is known.",
        fix: Some("Set MURMEL_CONFIG_DIR to a writable folder."),
    },
    Code {
        code: "SET002",
        severity: Severity::Error,
        meaning: "The navigation bindings bind one input to two actions.",
        fix: Some("Give each gesture, button and modifier combination one action."),
    },
    Code {
        code: "SET003",
        severity: Severity::Error,
        meaning: "A value cannot be a default: its kind has no such value, it does not fit, or it belongs to one mesh or field.",
        fix: Some("settings.get lists the values each kind can keep as a default (defaultable)."),
    },
    Code {
        code: "EXPORT001",
        severity: Severity::Error,
        meaning: "An export setting is out of range (scale, line width, dpi).",
        fix: Some(
            "Use a scale of 0.25 to 8, a line width of 1 to 32 pixels and a dpi of 1 to 10000; a movie 1 to 60 frames a second and at most 36000 frames, an MP4 without transparency.",
        ),
    },
    Code {
        code: "EXPORT002",
        severity: Severity::Error,
        meaning: "An MP4 needs ffmpeg, and none was found (MURMEL_FFMPEG, PATH, /opt/homebrew/bin, /usr/local/bin).",
        fix: Some(
            "Install ffmpeg (macOS: brew install ffmpeg), set MURMEL_FFMPEG to its path, or write a GIF or PNG frames.",
        ),
    },
    Code {
        code: "EXPORT003",
        severity: Severity::Error,
        meaning: "ffmpeg could not be started or failed while writing the MP4; the message ends with what it said.",
        fix: Some("Check the path and the free space; try another ffmpeg with MURMEL_FFMPEG."),
    },
    Code {
        code: "EXPORT004",
        severity: Severity::Error,
        meaning: "A movie file or a frame could not be written.",
        fix: Some("Check that the folder exists and is writable, and the free space."),
    },
    Code {
        code: "EXPORT005",
        severity: Severity::Error,
        meaning: "The movie has nothing to move: no clip plane to sweep, fewer than two saved views, or a view name that is not saved.",
        fix: Some("Add a clip plane (clip.set) or save views (views.save); views.list names them."),
    },
    Code {
        code: "EXPORT006",
        severity: Severity::Error,
        meaning: "The movie was cancelled; a half-written MP4 or GIF is removed, PNG frames written stay.",
        fix: None,
    },
    Code {
        code: "EXPORT007",
        severity: Severity::Error,
        meaning: "The window is making a movie already; it makes one at a time.",
        fix: Some("Wait for the movie, or cancel it in the Export movie window."),
    },
    Code {
        code: "MEAS001",
        severity: Severity::Error,
        meaning: "Nothing of the mesh lies under the screen point, so there is no point to measure from.",
        fix: Some(
            "Point at a face of the mesh, or give typed coordinates (point), a node, a face or a cell.",
        ),
    },
    Code {
        code: "MEAS002",
        severity: Severity::Error,
        meaning: "The open mesh has no node, face or cell with this number.",
        fix: Some("mesh.info gives the counts; numbers start at 0."),
    },
    Code {
        code: "MEAS003",
        severity: Severity::Error,
        meaning: "The name of a measurement or probe is empty, longer than 64 characters or holds control characters.",
        fix: None,
    },
    Code {
        code: "SESS001",
        severity: Severity::Error,
        meaning: "A session file could not be read, or it is not a MurmelCraft session (format murmelcraft.session 1.x) with valid values.",
        fix: Some("Check the path; save the session again with session.save."),
    },
    Code {
        code: "SESS002",
        severity: Severity::Error,
        meaning: "A session file could not be written.",
        fix: Some("Choose a writable folder."),
    },
    Code {
        code: "SESS003",
        severity: Severity::Warning,
        meaning: "The session's mesh changed since the session was saved (its SHA-256 differs).",
        fix: Some("Check the view; save the session again to accept the new mesh."),
    },
    Code {
        code: "SESS004",
        severity: Severity::Warning,
        meaning: "Items the session names are not in the mesh; they were skipped.",
        fix: Some("Save the session again to drop them."),
    },
    Code {
        code: "FIELD001",
        severity: Severity::Error,
        meaning: "The open mesh has no fields: it was read without results.",
        fix: Some("Open a result: FoehnCraft's fields.vtkhdf, or a .vtu with cell or point data."),
    },
    Code {
        code: "FIELD002",
        severity: Severity::Error,
        meaning: "The open mesh has no field of this name, or no field is chosen.",
        fix: Some("field.list names the fields; pass `field`."),
    },
    Code {
        code: "FIELD003",
        severity: Severity::Error,
        meaning: "A component (x, y, z) was asked of a scalar field.",
        fix: Some("Leave `component` out, or give `magnitude`, for a scalar."),
    },
    Code {
        code: "FIELD004",
        severity: Severity::Error,
        meaning: "A field style value is out of range: bands outside 2 to 64, contour lines outside 1 to 64, an unknown colour map, or a fixed range whose min is not below its max.",
        fix: Some("field.list names the colour maps; give finite min < max."),
    },
    Code {
        code: "FIELD005",
        severity: Severity::Error,
        meaning: "A derived field's input is missing: no field of the name given, or none found by its name (velocity: velocity, U or u; pressure: pressure, p or static_pressure).",
        fix: Some(
            "Name the input field (velocity, pressure); field.catalogue lists the candidates.",
        ),
    },
    Code {
        code: "FIELD006",
        severity: Severity::Error,
        meaning: "A derived field's input has the wrong shape or place: the velocity needs three components, the pressure one, the vortex quantities a velocity in the cells or at the nodes, the total pressure a velocity where the pressure is.",
        fix: Some("field.catalogue lists the fields that fit each input."),
    },
    Code {
        code: "FIELD007",
        severity: Severity::Error,
        meaning: "A reference value is out of range: the density and the velocity must be above 0, every value finite and at most 1e12 in size.",
        fix: Some("Give reference density, pressure and velocity in range."),
    },
    Code {
        code: "FIELD008",
        severity: Severity::Error,
        meaning: "The result's file already has a field of the derived quantity's name; it is not computed again.",
        fix: Some("Colour by the file's field with display.color_by."),
    },
    Code {
        code: "FIELD009",
        severity: Severity::Error,
        meaning: "No derived or calculated field of this name: none was made with field.derive or field.calculate, or the field is the file's own.",
        fix: Some("field.catalogue says which fields are derived and calculated."),
    },
    Code {
        code: "FIELD010",
        severity: Severity::Error,
        meaning: "The derived or calculated field is in use: an iso-surface, the arrows, a streamline set or another derived or calculated field is made from it (or it would change between scalar and vector under them).",
        fix: Some(
            "Remove or change the items and fields named (iso.clear, arrows.clear, stream.clear, field.calculate remove), then remove or change the field.",
        ),
    },
    Code {
        code: "FIELD011",
        severity: Severity::Error,
        meaning: "The formula cannot be read: a character it does not know, a bracket not closed, something missing or too much; the message says the column. Or it is empty, longer than 2000 characters or nested more than 64 deep.",
        fix: Some("Mend the formula at the column given; field.calculate describes the grammar."),
    },
    Code {
        code: "FIELD012",
        severity: Severity::Error,
        meaning: "A name in the formula is no field of the result, no built-in name (x, y, z, pos, pi) and no function.",
        fix: Some(
            "The message lists the fields; quote a field name with spaces or dots (\"p rgh\"). field.catalogue lists the functions.",
        ),
    },
    Code {
        code: "FIELD013",
        severity: Severity::Error,
        meaning: "The formula's types or places do not fit: a vector where a scalar goes (or the other way), a function given the wrong number of arguments, a field with neither 1 nor 3 components, or fields with no place in common (one only on the boundary faces, one only in the cells).",
        fix: Some(
            "Take a component (U.x) or the magnitude (mag(U)), or use dot or cross; the message says which argument.",
        ),
    },
    Code {
        code: "FIELD014",
        severity: Severity::Error,
        meaning: "A formula's field cannot have this name (it must start with a letter or _, then letters, digits and _, at most 64 long, and not be a derived quantity's field), or its unit is longer than 64 characters.",
        fix: Some("Choose another name, such as speed or p_rel."),
    },
    Code {
        code: "FIELD015",
        severity: Severity::Error,
        meaning: "The formula reads its own field, or a field made from it, so it would go round in a circle.",
        fix: Some(
            "Read the fields it is made from instead, or remove the field made from it first.",
        ),
    },
];

/// Every code: the tools' own, then the mesh and render codes.
pub fn codes() -> &'static [Code] {
    static ALL: OnceLock<Vec<Code>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut v = OWN.to_vec();
        for (code, meaning, fix) in murmel_mesh::CODES.iter().chain(murmel_render::CODES) {
            let severity = if *code == "MESH011" {
                Severity::Warning
            } else {
                Severity::Error
            };
            v.push(Code {
                code,
                severity,
                meaning,
                fix: Some(fix),
            });
        }
        v
    })
}

pub fn code(id: &str) -> Option<&'static Code> {
    codes().iter().find(|c| c.code == id)
}
