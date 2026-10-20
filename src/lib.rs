//! WASM glue for step-loupe (the step-io STEP viewer): `load_step(source, sink)` reads a
//! STEP file piece by piece from a JS source, walks the scene, and hands the result to a
//! JS sink as it is made: a JSON bundle (report, header, tree) in pieces, and the
//! rendered geometry in binary runs. The HTML/three.js front-end renders both.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use step_io::scene::{
    ApprovalDate, Edge, Face, Feature, FeatureGeometry, GeometricSet, Person, ProductDef, Rgb,
    SetElement, Solid, SurfaceModel, Transform, Vertex,
};
use step_io::EntityKey;
use wasm_bindgen::prelude::*;

// ---------- 4x4 transforms (row-major, p' = M·p, column vectors) ----------

type Mat = [[f64; 4]; 4];
const IDENTITY: Mat = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn mat_mul(a: &Mat, b: &Mat) -> Mat {
    let mut m = [[0.0f64; 4]; 4];
    for (i, row) in m.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

/// Transform a point by a row-major 4x4 (implicit w = 1).
fn apply(m: &Mat, p: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2] + m[0][3],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2] + m[1][3],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2] + m[2][3],
    ]
}

// ---------- JSON shapes ----------

#[derive(Serialize)]
struct Header {
    file_name: String,
    time_stamp: String,
    author: Vec<String>,
    organization: Vec<String>,
    preprocessor: String,
    originating_system: String,
    authorisation: String,
    description: String,
    /// Identified schema, clean label (e.g. `AP242 ed2 (IS)`).
    schema: String,
    /// Raw `FILE_SCHEMA` token(s), verbatim.
    raw_schema: Vec<String>,
}

#[derive(Serialize)]
struct Report {
    n_in: usize,
    validated: usize,
    n_synth: usize,
    dropped: Vec<(u64, String)>,
    norm: Vec<(u64, String)>,
    parse_warnings: Vec<String>,
}

/// One tree node, written out as soon as it is made: children come before their
/// parent, and the page places each node by its `id`. `children` refer to nodes
/// by id (`>= 0`) and to groups by `-1 - index` into the bundle's `groups`, so a
/// node under several parents (an edge shared by two faces) is listed once.
#[derive(Serialize)]
struct Node {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<u32>,
    kind: &'static str,
    label: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    meta: Vec<[String; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<i64>,
}

// ---------- output ----------

#[wasm_bindgen]
extern "C" {
    /// Where the file comes from: `readInto(buf)` fills the front of `buf` with
    /// the next bytes and returns how many, 0 at the end. It throws a string
    /// when reading fails.
    pub type Source;
    #[wasm_bindgen(method, catch, js_name = readInto)]
    fn read_into(this: &Source, buf: &mut [u8]) -> Result<usize, JsValue>;

    /// Where the result goes, as it is made. `text(bytes)` takes the next piece
    /// of the JSON bundle, valid UTF-8 by itself; `geometry(rows, d, tri)` the
    /// next run of rendered geometry (see `Geometry`). Both get views into wasm
    /// memory, valid only during the call.
    pub type Sink;
    #[wasm_bindgen(method)]
    fn text(this: &Sink, bytes: &[u8]);
    #[wasm_bindgen(method)]
    fn geometry(this: &Sink, rows: &[u32], d: &[f32], tri: &[u32]);
}

/// Bytes of JSON gathered before they go to the sink.
const TEXT_PIECE: usize = 64 << 10;

/// The JSON bundle, sent to the sink in pieces. A piece ends only after a whole
/// value, so no character is split between two pieces.
struct Json<'a> {
    sink: &'a Sink,
    buf: Vec<u8>,
}

impl<'a> Json<'a> {
    fn new(sink: &'a Sink) -> Self {
        Json {
            sink,
            buf: Vec::new(),
        }
    }

    fn raw(&mut self, s: &str) {
        self.buf.extend_from_slice(s.as_bytes());
    }

    fn value<T: Serialize + ?Sized>(&mut self, v: &T) {
        serde_json::to_writer(&mut self.buf, v).expect("the bundle's values serialize");
        if self.buf.len() >= TEXT_PIECE {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.buf.is_empty() {
            self.sink.text(&self.buf);
            self.buf.clear();
        }
    }
}

// Kinds of rendered geometry, in `Geometry` rows.
const LINE: u32 = 0;
/// A straight segment between an edge's vertices, standing in for a curve that
/// could not be sampled.
const FALLBACK: u32 = 1;
const POINT: u32 = 2;
const MESH: u32 = 3;

/// Floats and indices gathered before they go to the sink.
const GEOMETRY_RUN: usize = 1 << 18;

/// Rendered geometry, sent to the sink in runs, in node id order. Each item is
/// four numbers in `rows` (node id, kind, its floats in `d`, its indices in
/// `tri`) and its values: x,y,z points in `d` (a polyline, one point, or mesh
/// vertices) and mesh triangle indices in `tri`. An item is never split
/// between runs.
struct Geometry<'a> {
    sink: &'a Sink,
    rows: Vec<u32>,
    d: Vec<f32>,
    tri: Vec<u32>,
}

impl<'a> Geometry<'a> {
    fn new(sink: &'a Sink) -> Self {
        Geometry {
            sink,
            rows: Vec::new(),
            d: Vec::new(),
            tri: Vec::new(),
        }
    }

    fn push(
        &mut self,
        id: u32,
        kind: u32,
        d: impl IntoIterator<Item = f32>,
        tri: impl IntoIterator<Item = u32>,
    ) {
        let (d0, tri0) = (self.d.len(), self.tri.len());
        // A non-finite coordinate goes in as 0, so it cannot spoil the view's bounds.
        self.d
            .extend(d.into_iter().map(|x| if x.is_finite() { x } else { 0.0 }));
        self.tri.extend(tri);
        self.rows.extend([
            id,
            kind,
            (self.d.len() - d0) as u32,
            (self.tri.len() - tri0) as u32,
        ]);
        if self.d.len() + self.tri.len() >= GEOMETRY_RUN {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.rows.is_empty() {
            self.sink.geometry(&self.rows, &self.d, &self.tri);
            self.rows.clear();
            self.d.clear();
            self.tri.clear();
        }
    }
}

// ---------- entry point ----------

struct SourceReader<'a>(&'a Source);

impl std::io::Read for SourceReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0
            .read_into(buf)
            .map_err(|e| std::io::Error::other(e.as_string().unwrap_or_else(|| format!("{e:?}"))))
    }
}

#[wasm_bindgen]
pub fn load_step(source: &Source, sink: &Sink) {
    let (model, rep) = match step_io::read(SourceReader(source)) {
        Ok(v) => v,
        Err(e) => {
            let error = match e {
                step_io::Error::Io(e) => format!("could not read the file: {e}"),
                e => format!("parse failed: {e}"),
            };
            let mut json = Json::new(sink);
            json.value(&serde_json::json!({ "ok": false, "error": error }));
            json.flush();
            return;
        }
    };

    let header = build_header(model.header());

    let report = Report {
        n_in: rep.entities_read,
        validated: rep.entities_kept,
        n_synth: rep.entities_added,
        dropped: rep
            .dropped
            .iter()
            .map(|(id, r)| (*id, r.to_string()))
            .collect(),
        norm: rep
            .normalized
            .iter()
            .map(|(id, note)| (*id, (*note).to_string()))
            .collect(),
        parse_warnings: rep.parse_warnings.iter().map(ToString::to_string).collect(),
    };

    let scene = model.scene();
    let mut b = Builder::new(sink);
    b.json.raw(r#"{"ok":true,"header":"#);
    b.json.value(&header);
    b.json.raw(r#","report":"#);
    b.json.value(&report);
    // The nodes follow, each as it is made.
    b.json.raw(r#","nodes":["#);
    let mut roots = Vec::new();

    // Model tree: assembly hierarchy (root_definitions -> occurrences), with
    // each occurrence's placement transform baked into the emitted geometry.
    let mut visited = Vec::new();
    let mut emitted = HashSet::new();
    let mut model_nodes = Vec::new();
    for def in scene.root_definitions() {
        model_nodes.push(i64::from(b.emit_def(
            &def,
            &IDENTITY,
            true,
            None,
            &mut visited,
            &mut emitted,
        )));
    }
    // Canonical top-level categories are always emitted (empty ones render
    // disabled), so "unsupported" reads differently from "absent in this file".
    roots.push(b.group("Model", model_nodes));

    // Fallback: any solid, surface model or geometric set not reached through
    // the product structure is shown at the origin, so geometry-only files
    // never lose their geometry.
    let mut orphans = Vec::new();
    for solid in scene.all_solids() {
        if !emitted.contains(&solid.key()) {
            let mut imap = HashMap::new();
            orphans.push(i64::from(b.solid_node(&solid, &IDENTITY, &mut imap)));
        }
    }
    for sm in scene.all_surface_models() {
        if !emitted.contains(&sm.key()) {
            let mut imap = HashMap::new();
            orphans.push(i64::from(b.surface_model_node(&sm, &IDENTITY, &mut imap)));
        }
    }
    for set in scene.all_geometric_sets() {
        if !emitted.contains(&set.key()) {
            let mut imap = HashMap::new();
            orphans.push(i64::from(b.set_node(&set, &IDENTITY, &mut imap)));
        }
    }
    roots.push(b.group("Unplaced geometry", orphans));

    // Node id -> rendered geometry ids to highlight (features/PMI only), in id
    // order so the bundle is the same from run to run.
    let mut out_hl: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    // Features first: build the nodes and a key -> (node id, highlight ids) map
    // so PMI can cross-reference each by number and inherit its 3D highlight.
    // Description and referenced geometry go to the detail pane as meta rows.
    let mut feat_map: HashMap<EntityKey, (u32, Vec<u32>)> = HashMap::new();
    let mut features = Vec::new();
    for f in scene.all_features() {
        let node_id = b.next();
        let mut meta = Vec::new();
        if let Some(desc) = f.description() {
            if !desc.is_empty() {
                meta.push(kv("description", desc));
            }
        }
        let mut hl: Vec<u32> = Vec::new();
        for g in f.geometry() {
            meta.push(kv("ref", &fg_label(&g)));
            if let Some(ids) = fg_key(&g).and_then(|k| b.hl_ids.get(&k)) {
                hl.extend(ids);
            }
        }
        if !hl.is_empty() {
            out_hl.insert(node_id, hl.clone());
        }
        feat_map.insert(f.key(), (node_id, hl));
        features.push(b.leaf(node_id, "Feature", feature_label(&f), meta));
    }

    // PMI. Datums are built first so tolerances can cross-reference them by
    // number; in the group they display after dimensions and tolerances.
    let mut datum_map: HashMap<EntityKey, u32> = HashMap::new();
    let mut datum_nodes = Vec::new();
    for dt in scene.all_datums() {
        let id = b.next();
        let mut meta = vec![kv("letter", dt.letter())];
        if !dt.name().is_empty() {
            meta.push(kv("name", dt.name()));
        }
        // Highlight the datum's physical feature (its face/edge) on selection, by
        // reusing the datum feature's own node id + highlight ids.
        if let Some(feat) = dt.datum_feature() {
            if let Some((feat_node_id, hl)) = feat_map.get(&feat.key()) {
                meta.push(kv("based on", &format!("Feature #{feat_node_id}")));
                if !hl.is_empty() {
                    out_hl.insert(id, hl.clone());
                }
            }
        }
        datum_map.insert(dt.key(), id);
        datum_nodes.push(b.leaf(id, "Datum", format!("Datum {}", dt.letter()), meta));
    }

    let mut pmi = Vec::new();
    for d in scene.all_dimensions() {
        let id = b.next();
        let mut meta = Vec::new();
        let label = match d.value() {
            Some(v) => {
                meta.push(kv("value", &v.to_string()));
                format!("Dimension ({:?}): {v}", d.kind())
            }
            None => format!("Dimension ({:?})", d.kind()),
        };
        let mut hl: Vec<u32> = Vec::new();
        for f in d.features() {
            apply_feature_ref(&mut meta, &mut hl, &feat_map, &f);
        }
        if !hl.is_empty() {
            out_hl.insert(id, hl);
        }
        pmi.push(b.leaf(id, "Dimension", label, meta));
    }
    for t in scene.all_tolerances() {
        let id = b.next();
        let mut meta = Vec::new();
        let mut tail = String::new();
        if let Some(m) = t.magnitude() {
            meta.push(kv("magnitude", &m.to_string()));
            tail.push_str(&format!(": {m}"));
        }
        let ds: Vec<_> = t.datums().collect();
        if !ds.is_empty() {
            let letters: Vec<String> = ds.iter().map(|d| d.letter().to_string()).collect();
            tail.push_str(&format!(" | {}", letters.join(",")));
            // cross-reference each datum to its PMI node number
            for d in &ds {
                match datum_map.get(&d.key()) {
                    Some(nid) => meta.push(kv("datum", &format!("Datum #{nid}: {}", d.letter()))),
                    None => meta.push(kv("datum", d.letter())),
                }
            }
        }
        let mut hl: Vec<u32> = Vec::new();
        if let Some(f) = t.feature() {
            apply_feature_ref(&mut meta, &mut hl, &feat_map, &f);
        }
        if !hl.is_empty() {
            out_hl.insert(id, hl);
        }
        pmi.push(b.leaf(
            id,
            "Tolerance",
            format!("Tolerance ({:?}){tail}", t.kind()),
            meta,
        ));
    }
    pmi.extend(datum_nodes);
    roots.push(b.group("PMI", pmi));
    roots.push(b.group("Features", features));

    // Units (length/angle + modelling precision) — file-level info, shown in the
    // info panel next to the header, not as a selectable scene-tree group.
    let scene_units = scene.units();
    let mut units = Vec::new();
    if let Some(u) = &scene_units.length {
        units.push(kv("length", &format!("{} (×{} m)", u.name, u.to_si)));
    }
    if let Some(u) = &scene_units.angle {
        units.push(kv("angle", &format!("{} (×{} rad)", u.name, u.to_si)));
    }
    if let Some(p) = scene_units.precision {
        // Modelling uncertainty is tiny (e.g. 1e-7); scientific notation reads
        // better than a long decimal.
        units.push(kv("precision", &format!("{p:e}")));
    }

    // Display meshes.
    let mut meshes = Vec::new();
    for g in scene.all_mesh_groups() {
        for m in g.meshes() {
            let id = b.next();
            let tris = m.triangles();
            b.geom.push(
                id,
                MESH,
                m.points().into_iter().flat_map(|p| p.map(|c| c as f32)),
                tris.iter().flat_map(|t| t.map(|i| i as u32)),
            );
            meshes.push(b.leaf(id, "Mesh", format!("Mesh ({} tris)", tris.len()), vec![]));
        }
    }
    roots.push(b.group("Meshes", meshes));

    b.geom.flush();
    let json = &mut b.json;
    json.raw(r#"],"tree":"#);
    json.value(&roots);
    json.raw(r#","groups":"#);
    json.value(&b.groups);
    if !units.is_empty() {
        json.raw(r#","units":"#);
        json.value(&units);
    }
    if !out_hl.is_empty() {
        json.raw(r#","hl":"#);
        json.value(&out_hl);
    }
    json.raw(r#","warnings":"#);
    let warnings: Vec<String> = scene.warnings().iter().map(ToString::to_string).collect();
    json.value(&warnings);
    json.raw("}");
    json.flush();
}

// ---------- scene walk helpers ----------

struct Builder<'a> {
    next: u32,
    json: Json<'a>,
    /// Whether a node has been written out, by id.
    listed: Vec<bool>,
    geom: Geometry<'a>,
    /// Geometry EntityKey -> rendered leaf ids (edge lines, vertex points) under
    /// it, accumulated across every placed instance. Lets features/PMI resolve
    /// the faces/edges they reference back to highlightable 3D objects.
    hl_ids: HashMap<EntityKey, Vec<u32>>,
    groups: Vec<Node>,
}

impl<'a> Builder<'a> {
    fn new(sink: &'a Sink) -> Self {
        Builder {
            next: 0,
            json: Json::new(sink),
            listed: Vec::new(),
            geom: Geometry::new(sink),
            hl_ids: HashMap::new(),
            groups: Vec::new(),
        }
    }

    fn next(&mut self) -> u32 {
        let i = self.next;
        self.next += 1;
        i
    }

    /// Write the node `id` out and return it.
    fn put(
        &mut self,
        id: u32,
        kind: &'static str,
        label: String,
        meta: Vec<[String; 2]>,
        children: Vec<i64>,
    ) -> u32 {
        if !self.listed.is_empty() {
            self.json.raw(",");
        }
        let i = id as usize;
        if self.listed.len() <= i {
            self.listed.resize(i + 1, false);
        }
        self.listed[i] = true;
        self.json.value(&Node {
            id: Some(id),
            kind,
            label,
            meta,
            children,
        });
        id
    }

    fn leaf(&mut self, id: u32, kind: &'static str, label: String, meta: Vec<[String; 2]>) -> i64 {
        i64::from(self.put(id, kind, label, meta, vec![]))
    }

    fn group(&mut self, label: &str, children: Vec<i64>) -> i64 {
        self.groups.push(Node {
            id: None,
            kind: "Group",
            label: label.to_string(),
            meta: vec![],
            children,
        });
        -(self.groups.len() as i64)
    }

    fn built(&self, id: u32) -> bool {
        self.listed.get(id as usize).copied().unwrap_or(false)
    }

    /// Id deduped within one placed-instance map; distinct instances of a shared
    /// part get distinct ids (and distinct world-baked geometry).
    fn imap_id(&mut self, imap: &mut HashMap<EntityKey, u32>, key: EntityKey) -> u32 {
        if let Some(&i) = imap.get(&key) {
            return i;
        }
        let i = self.next();
        imap.insert(key, i);
        i
    }

    /// Walk one product-definition instance placed at `world`, recursing into its
    /// occurrences with accumulated transforms. `visited` guards against cycles
    /// on the current path (shared parts under different paths are fine).
    fn emit_def(
        &mut self,
        def: &ProductDef<'_>,
        world: &Mat,
        is_root: bool,
        placement: Option<Transform>,
        visited: &mut Vec<EntityKey>,
        emitted: &mut HashSet<EntityKey>,
    ) -> u32 {
        let key = def.key();
        let cyclic = visited.contains(&key);
        visited.push(key);

        let node_id = self.next();
        let mut children = Vec::new();
        if !cyclic {
            // this definition's own b-rep, baked to `world`
            let mut imap = HashMap::new();
            for solid in def.solids() {
                emitted.insert(solid.key());
                children.push(i64::from(self.solid_node(&solid, world, &mut imap)));
            }
            for sm in def.surface_models() {
                emitted.insert(sm.key());
                children.push(i64::from(self.surface_model_node(&sm, world, &mut imap)));
            }
            for set in def.geometric_sets() {
                emitted.insert(set.key());
                children.push(i64::from(self.set_node(&set, world, &mut imap)));
            }
            // placed children (occurrences carry the transform)
            for occ in def.occurrences() {
                if let Some(child) = occ.definition() {
                    let tf = occ.transform();
                    let local = tf.as_ref().and_then(|t| t.matrix()).unwrap_or(IDENTITY);
                    let child_world = mat_mul(world, &local);
                    children.push(i64::from(self.emit_def(
                        &child,
                        &child_world,
                        false,
                        tf,
                        visited,
                        emitted,
                    )));
                }
            }
        }
        visited.pop();

        let (label, mut meta) = def_label_meta(def);
        if let Some(d) = def.description() {
            if !d.is_empty() {
                meta.push(kv("description", d));
            }
        }
        // Placement frame this component sits at (the occurrence's target frame),
        // plus its absolute position in the scene.
        if let Some(tf) = &placement {
            meta.push(kv("origin", &fmt3(tf.to.origin)));
            meta.push(kv("axis", &fmt3(tf.to.axis)));
            meta.push(kv("ref dir", &fmt3(tf.to.ref_direction)));
            meta.push(kv(
                "world origin",
                &fmt3([world[0][3], world[1][3], world[2][3]]),
            ));
        }
        meta.extend(plm_meta(def));
        let kind = if is_root { "Part" } else { "Component" };
        self.put(node_id, kind, label, meta, children)
    }

    fn solid_node(
        &mut self,
        solid: &Solid<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> u32 {
        let sid = self.imap_id(imap, solid.key());
        let mut faces = Vec::new();
        let mut face_count = 0u32;
        let mut solid_edge_ids: Vec<u32> = Vec::new();
        for face in solid.faces() {
            face_count += 1;
            let (fid, edge_ids) = self.face_node(&face, world, imap);
            solid_edge_ids.extend(&edge_ids);
            faces.push(i64::from(fid));
        }
        // Internal voids (BREP_WITH_VOIDS): one group per cavity shell, its
        // faces surfaced like the outer shell's so the cavity is fully read.
        let voids: Vec<_> = solid.voids().collect();
        let void_count = voids.len();
        for (i, (cavity, _)) in voids.into_iter().enumerate() {
            let mut cavity_faces = Vec::new();
            for face in cavity.faces() {
                face_count += 1;
                let (fid, edge_ids) = self.face_node(&face, world, imap);
                solid_edge_ids.extend(&edge_ids);
                cavity_faces.push(i64::from(fid));
            }
            faces.push(self.group(&format!("Void {}", i + 1), cavity_faces));
        }
        self.hl_ids
            .entry(solid.key())
            .or_default()
            .extend(solid_edge_ids);
        let mut meta = Vec::new();
        if !solid.name().is_empty() {
            meta.push(kv("name", solid.name()));
        }
        if let Some(c) = solid.color() {
            meta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(t) = solid.transparency() {
            meta.push(kv("transparency", &t.to_string()));
        }
        if let Some(l) = solid.layer() {
            meta.push(kv("layer", l));
        }
        if !solid.is_visible() {
            meta.push(kv("visible", "false"));
        }
        meta.push(kv("faces", &face_count.to_string()));
        if void_count > 0 {
            meta.push(kv("voids", &void_count.to_string()));
        }
        self.put(sid, "Solid", "Solid".into(), meta, faces)
    }

    /// A surface model (sheet body): its faces, one group per shell when it
    /// has several.
    fn surface_model_node(
        &mut self,
        sm: &SurfaceModel<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> u32 {
        let sid = self.imap_id(imap, sm.key());
        let shells: Vec<_> = sm.shells().collect();
        let mut children = Vec::new();
        let mut face_count = 0u32;
        let mut open = 0u32;
        let mut edge_ids: Vec<u32> = Vec::new();
        for (i, (shell, _)) in shells.iter().enumerate() {
            if !shell.is_closed() {
                open += 1;
            }
            let mut faces = Vec::new();
            for face in shell.faces() {
                face_count += 1;
                let (fid, ids) = self.face_node(&face, world, imap);
                edge_ids.extend(&ids);
                faces.push(i64::from(fid));
            }
            if shells.len() == 1 {
                children = faces;
            } else {
                let kind = if shell.is_closed() { "closed" } else { "open" };
                children.push(self.group(&format!("Shell {} ({kind})", i + 1), faces));
            }
        }
        self.hl_ids.entry(sm.key()).or_default().extend(edge_ids);
        let mut meta = Vec::new();
        if !sm.name().is_empty() {
            meta.push(kv("name", sm.name()));
        }
        if let Some(c) = sm.color() {
            meta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(t) = sm.transparency() {
            meta.push(kv("transparency", &t.to_string()));
        }
        if let Some(l) = sm.layer() {
            meta.push(kv("layer", l));
        }
        if !sm.is_visible() {
            meta.push(kv("visible", "false"));
        }
        meta.push(kv("shells", &shells.len().to_string()));
        if open > 0 {
            meta.push(kv("open shells", &open.to_string()));
        }
        meta.push(kv("faces", &face_count.to_string()));
        self.put(sid, "Surface body", "Surface body".into(), meta, children)
    }

    /// A geometric set (wireframe or sketch): one leaf per element, curves
    /// drawn as lines and points as points.
    fn set_node(
        &mut self,
        set: &GeometricSet<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> u32 {
        let id = self.imap_id(imap, set.key());
        let mut children = Vec::new();
        let mut drawn: Vec<u32> = Vec::new();
        for element in set.elements() {
            match element {
                SetElement::Curve(curve) => {
                    let cid = self.imap_id(imap, curve.key());
                    if !self.built(cid) {
                        let cname = kind_name(&format!("{:?}", curve.kind()));
                        let mut meta = vec![kv("curve", &cname)];
                        match curve.to_nurbs() {
                            Some(nc) => {
                                let pts = tessellate_nurbs(
                                    &nc.control_points,
                                    &nc.weights,
                                    &nc.knots,
                                    nc.degree,
                                );
                                self.geom.push(cid, LINE, flatten_world(&pts, world), []);
                            }
                            None => meta.push(kv("drawn", "no (unbounded or unsupported)")),
                        }
                        if let Some(c) = curve.color() {
                            meta.push(kv("color", &rgb_hex(c)));
                        }
                        if let Some(l) = curve.layer() {
                            meta.push(kv("layer", l));
                        }
                        self.hl_ids.entry(curve.key()).or_default().push(cid);
                        self.put(cid, "Curve", format!("Curve ({cname})"), meta, vec![]);
                    }
                    drawn.push(cid);
                    children.push(i64::from(cid));
                }
                SetElement::Point(point) => {
                    let pid = self.imap_id(imap, point.key());
                    if !self.built(pid) {
                        self.geom
                            .push(pid, POINT, apply(world, point.xyz()).map(|c| c as f32), []);
                        let [x, y, z] = point.xyz();
                        let mut meta = vec![kv("coords", &format!("{x}, {y}, {z}"))];
                        if let Some(c) = point.color() {
                            meta.push(kv("color", &rgb_hex(c)));
                        }
                        self.hl_ids.entry(point.key()).or_default().push(pid);
                        self.put(pid, "Point", "Point".into(), meta, vec![]);
                    }
                    drawn.push(pid);
                    children.push(i64::from(pid));
                }
                SetElement::Other(key) => {
                    let oid = self.next();
                    let name = key.keyword().unwrap_or("complex entity");
                    children.push(self.leaf(oid, "Item", name.to_string(), vec![]));
                }
                _ => {}
            }
        }
        self.hl_ids.entry(set.key()).or_default().extend(drawn);
        let mut meta = Vec::new();
        if !set.name().is_empty() {
            meta.push(kv("name", set.name()));
        }
        if let Some(c) = set.color() {
            meta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(l) = set.layer() {
            meta.push(kv("layer", l));
        }
        if !set.is_visible() {
            meta.push(kv("visible", "false"));
        }
        meta.push(kv("elements", &children.len().to_string()));
        self.put(id, "Wireframe", "Wireframe".into(), meta, children)
    }

    /// One face's tree node id plus the ids of its edges (for highlight
    /// aggregation). Shared by a solid's outer shell and its void cavities.
    fn face_node(
        &mut self,
        face: &Face<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> (u32, Vec<u32>) {
        let fid = self.imap_id(imap, face.key());
        let sk = kind_name(&format!("{:?}", face.surface().kind()));
        let mut fmeta = vec![kv("surface", &sk)];
        if let Some(c) = face.color() {
            fmeta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(t) = face.transparency() {
            fmeta.push(kv("transparency", &t.to_string()));
        }
        if let Some(l) = face.layer() {
            fmeta.push(kv("layer", l));
        }
        if !face.is_visible() {
            fmeta.push(kv("visible", "false"));
        }
        fmeta.push(kv("orientation", &format!("{:?}", face.orientation())));
        let mut bounds = Vec::new();
        let mut face_edge_ids: Vec<u32> = Vec::new();
        for bound in face.bounds() {
            let bid = self.imap_id(imap, bound.key());
            let mut edges = Vec::new();
            for (edge, _fwd) in bound.oriented_edges() {
                let eid = self.edge_node(&edge, world, imap);
                face_edge_ids.push(eid);
                edges.push(i64::from(eid));
            }
            bounds.push(i64::from(self.put(
                bid,
                "Bound",
                "Bound".into(),
                vec![],
                edges,
            )));
        }
        // a feature referencing this face highlights the face's edges
        self.hl_ids
            .entry(face.key())
            .or_default()
            .extend(&face_edge_ids);
        let node = self.put(fid, "Face", format!("Face ({sk})"), fmeta, bounds);
        (node, face_edge_ids)
    }

    fn edge_node(
        &mut self,
        edge: &Edge<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> u32 {
        let eid = self.imap_id(imap, edge.key());
        if self.built(eid) {
            // already listed in this instance
            return eid;
        }
        self.hl_ids.entry(edge.key()).or_default().push(eid);
        let (d, fallback) = sample_edge(edge, world);
        self.geom
            .push(eid, if fallback { FALLBACK } else { LINE }, d, []);
        let curve = edge.curve();
        let cname = kind_name(&format!("{:?}", curve.kind()));
        let mut meta = vec![kv("curve", &cname)];
        // A line style on the edge itself, else on its curve.
        if let Some(c) = edge.color().or_else(|| curve.color()) {
            meta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(w) = edge.width().or_else(|| curve.width()) {
            meta.push(kv("width", &w.to_string()));
        }
        if let Some(f) = edge.line_font().or_else(|| curve.line_font()) {
            meta.push(kv("line font", f));
        }
        meta.push(kv("same sense", &edge.same_sense().to_string()));
        let mut children = Vec::new();
        for v in [edge.start(), edge.end()].into_iter().flatten() {
            children.push(i64::from(self.vertex_node(&v, world, imap)));
        }
        self.put(eid, "Edge", format!("Edge ({cname})"), meta, children)
    }

    fn vertex_node(
        &mut self,
        v: &Vertex<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> u32 {
        let vid = self.imap_id(imap, v.key());
        if self.built(vid) {
            // already listed in this instance
            return vid;
        }
        self.hl_ids.entry(v.key()).or_default().push(vid);
        let mut meta = vec![];
        if let Some(p) = v.point() {
            // A feature may reference the cartesian point directly; map it to the
            // vertex's rendered point so such references still highlight.
            self.hl_ids.entry(p.key()).or_default().push(vid);
            self.geom
                .push(vid, POINT, apply(world, p.xyz()).map(|c| c as f32), []);
            let [x, y, z] = p.xyz();
            meta.push(kv("coords", &format!("{x}, {y}, {z}")));
        }
        self.put(vid, "Vertex", "Vertex".into(), meta, vec![])
    }
}

/// PLM metadata (approvals / documents / contributors / security) for an
/// assembly node, as detail-pane meta rows. Empty when the definition has none.
fn plm_meta(def: &ProductDef<'_>) -> Vec<[String; 2]> {
    let mut meta = Vec::new();
    for a in def.approvals() {
        let mut s = a.status().to_string();
        if !a.level().is_empty() {
            s.push_str(&format!(" / {}", a.level()));
        }
        meta.push(kv("approval", &s));
        if let Some(d) = a.date() {
            let ds = fmt_approval_date(&d);
            if !ds.is_empty() {
                meta.push(kv("approved", &ds));
            }
        }
        for ap in a.approvers() {
            let who = ap
                .person()
                .map(|p| person_name(&p))
                .or_else(|| ap.organization().map(str::to_string))
                .unwrap_or_default();
            if !who.is_empty() {
                let role = ap.role();
                let label = if role.is_empty() {
                    "approver".to_string()
                } else {
                    format!("approver ({role})")
                };
                meta.push(kv(&label, &who));
            }
        }
    }
    for d in def.documents() {
        let mut s = d.id().to_string();
        if !d.name().is_empty() {
            s.push_str(&format!(" — {}", d.name()));
        }
        if !d.kind().is_empty() {
            s.push_str(&format!(" [{}]", d.kind()));
        }
        meta.push(kv("document", &s));
    }
    for c in def.contributors() {
        let name = person_name(&c.person());
        let role = c.role();
        meta.push(kv(
            if role.is_empty() { "contributor" } else { role },
            &name,
        ));
    }
    for sc in def.security_classifications() {
        meta.push(kv("security", &format!("{} ({})", sc.name(), sc.level())));
        if !sc.purpose().is_empty() {
            meta.push(kv("purpose", sc.purpose()));
        }
    }
    // Full version list, only when the product carries more than the one already
    // shown as the node's `version`.
    if let Some(p) = def.product() {
        let vers: Vec<String> = p
            .versions()
            .map(|v| v.id().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if vers.len() > 1 {
            meta.push(kv("versions", &vers.join(", ")));
        }
    }
    meta
}

/// A 3-vector formatted compactly (4 decimals, trailing zeros trimmed).
fn fmt3(v: [f64; 3]) -> String {
    let f = |x: f64| {
        let s = format!("{x:.4}");
        let t = s.trim_end_matches('0').trim_end_matches('.');
        if t.is_empty() || t == "-" {
            "0".to_string()
        } else {
            t.to_string()
        }
    };
    format!("{}, {}, {}", f(v[0]), f(v[1]), f(v[2]))
}

/// An `ApprovalDate` as `YYYY-MM-DD` (dropping fields the file omits); empty when
/// even the year is absent.
fn fmt_approval_date(d: &ApprovalDate) -> String {
    match (d.year, d.month, d.day) {
        (Some(y), Some(m), Some(day)) => format!("{y:04}-{m:02}-{day:02}"),
        (Some(y), Some(m), None) => format!("{y:04}-{m:02}"),
        (Some(y), _, _) => y.to_string(),
        _ => String::new(),
    }
}

fn person_name(p: &Person<'_>) -> String {
    let n = format!(
        "{} {}",
        p.first_name().unwrap_or(""),
        p.last_name().unwrap_or("")
    )
    .trim()
    .to_string();
    if n.is_empty() {
        p.id().to_string()
    } else {
        n
    }
}

fn feature_label(f: &Feature<'_>) -> String {
    let name = f.name();
    if name.is_empty() {
        format!("{:?}", f.kind())
    } else {
        format!("{name} ({:?})", f.kind())
    }
}

/// The `EntityKey` of the geometry a `FeatureGeometry` designates, for looking
/// it up in `Builder::hl_ids`; `None` for a kind this viewer does not know.
fn fg_key(g: &FeatureGeometry<'_>) -> Option<EntityKey> {
    Some(match g {
        FeatureGeometry::Face(h) => h.key(),
        FeatureGeometry::Edge(h) => h.key(),
        FeatureGeometry::Point(h) => h.key(),
        FeatureGeometry::Solid(h) => h.key(),
        FeatureGeometry::Curve(h) => h.key(),
        FeatureGeometry::Mesh(h) => h.key(),
        FeatureGeometry::GeometricSet(h) => h.key(),
        FeatureGeometry::Other(k) => *k,
        _ => return None,
    })
}

/// Record a PMI item's "applies to" feature: cross-reference by node number when
/// the feature is one we listed, and inherit its highlight ids; otherwise just
/// name it.
fn apply_feature_ref(
    meta: &mut Vec<[String; 2]>,
    hl: &mut Vec<u32>,
    feat_map: &HashMap<EntityKey, (u32, Vec<u32>)>,
    f: &Feature<'_>,
) {
    match feat_map.get(&f.key()) {
        Some((node_id, fhl)) => {
            meta.push(kv(
                "applies to",
                &format!("Feature #{node_id}: {}", feature_label(f)),
            ));
            hl.extend(fhl);
        }
        None => meta.push(kv("applies to", &feature_label(f))),
    }
}

fn fg_label(g: &FeatureGeometry<'_>) -> String {
    match g {
        FeatureGeometry::Face(h) => format!("Face {:?}", h.key()),
        FeatureGeometry::Edge(h) => format!("Edge {:?}", h.key()),
        FeatureGeometry::Point(h) => format!("Point {:?}", h.key()),
        FeatureGeometry::Solid(h) => format!("Solid {:?}", h.key()),
        FeatureGeometry::Curve(h) => format!("Curve {:?}", h.key()),
        FeatureGeometry::Mesh(h) => format!("Mesh {:?}", h.key()),
        FeatureGeometry::GeometricSet(h) => format!("Wireframe {:?}", h.key()),
        FeatureGeometry::Other(k) => format!("{} {k:?}", k.keyword().unwrap_or("complex entity")),
        other => format!("{other:?}"),
    }
}

/// Product label + id/name/version metadata for an assembly node.
fn def_label_meta(def: &ProductDef<'_>) -> (String, Vec<[String; 2]>) {
    let mut meta = Vec::new();
    let mut label = def.id().to_string();
    if let Some(p) = def.product() {
        if !p.id().is_empty() {
            meta.push(kv("id", p.id()));
        }
        if !p.name().is_empty() {
            meta.push(kv("name", p.name()));
            label = p.name().to_string();
        }
    }
    if let Some(v) = def.version() {
        if !v.id().is_empty() {
            meta.push(kv("version", v.id()));
        }
    }
    if label.is_empty() {
        label = "Part".into();
    }
    (label, meta)
}

/// Sample an edge into a flat polyline (`[x,y,z, ...]`) baked to `world`. Falls
/// back to a straight segment between the endpoints when `to_nurbs()` fails.
fn sample_edge(edge: &Edge<'_>, world: &Mat) -> (Vec<f32>, bool) {
    if let Some(nc) = edge.to_nurbs() {
        let pts = tessellate_nurbs(&nc.control_points, &nc.weights, &nc.knots, nc.degree);
        if pts.len() >= 2 {
            return (flatten_world(&pts, world), false);
        }
    }
    // fallback: straight line between the two vertices
    let mut out = Vec::new();
    for v in [edge.start(), edge.end()].into_iter().flatten() {
        if let Some(p) = v.point() {
            let c = apply(world, p.xyz());
            out.extend([c[0] as f32, c[1] as f32, c[2] as f32]);
        }
    }
    (out, true)
}

fn flatten_world(pts: &[[f64; 3]], world: &Mat) -> Vec<f32> {
    let mut d = Vec::with_capacity(pts.len() * 3);
    for p in pts {
        let c = apply(world, *p);
        d.extend([c[0] as f32, c[1] as f32, c[2] as f32]);
    }
    d
}

// ---------- NURBS evaluation (rational B-spline) ----------

/// Curvature-adaptive tessellation of a rational B-spline curve: subdivide the
/// parameter interval only where the curve bends, so it stays smooth at any
/// scale while a straight span collapses to its endpoints. Scale-invariant (the
/// flatness test is relative to the local chord).
const CURVE_FLATNESS: f64 = 0.01; // chord deviation / chord (~2.8° per segment)
const CURVE_MIN_DEPTH: u32 = 1; // force one split (S-curves whose mid sits on the chord)
const CURVE_MAX_DEPTH: u32 = 9; // ≤ 2^9 segments — a safety cap; flatness stops earlier

fn tessellate_nurbs(cp: &[[f64; 3]], w: &[f64], u: &[f64], p: usize) -> Vec<[f64; 3]> {
    let n = cp.len();
    if n == 0 || w.len() != n || u.len() < n + p + 1 {
        return cp.to_vec();
    }
    let u0 = u[p];
    let u1 = u[n];
    // Bail unless the knot span is strictly positive (partial_cmp also bails on NaN).
    if u1.partial_cmp(&u0) != Some(std::cmp::Ordering::Greater) {
        return cp.to_vec();
    }
    let eval = |t: f64| eval_point(cp, w, u, p, t);
    let p0 = eval(u0);
    let p1 = eval(u1 - (u1 - u0) * 1e-9); // half-open span: nudge off the last knot
    let mut out = vec![p0];
    subdivide_curve(&eval, u0, u1, p0, p1, 0, &mut out);
    out.push(p1);
    out
}

fn subdivide_curve<F: Fn(f64) -> [f64; 3]>(
    eval: &F,
    a: f64,
    b: f64,
    pa: [f64; 3],
    pb: [f64; 3],
    depth: u32,
    out: &mut Vec<[f64; 3]>,
) {
    if depth >= CURVE_MAX_DEPTH {
        return;
    }
    let m = 0.5 * (a + b);
    let pm = eval(m);
    // Flat enough once past the minimum depth: the midpoint barely leaves the chord.
    if depth >= CURVE_MIN_DEPTH && point_line_dist(pm, pa, pb) <= CURVE_FLATNESS * dist3(pa, pb) {
        return;
    }
    subdivide_curve(eval, a, m, pa, pm, depth + 1, out);
    out.push(pm);
    subdivide_curve(eval, m, b, pm, pb, depth + 1, out);
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Perpendicular distance from `p` to the line through `a`-`b` (the chord's
/// sagitta at `p`); falls back to point distance when the chord is degenerate
/// (a≈b, e.g. a closed curve).
fn point_line_dist(p: [f64; 3], a: [f64; 3], b: [f64; 3]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    if len2 < 1e-24 {
        return dist3(p, a);
    }
    let cross = [
        ap[1] * ab[2] - ap[2] * ab[1],
        ap[2] * ab[0] - ap[0] * ab[2],
        ap[0] * ab[1] - ap[1] * ab[0],
    ];
    ((cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]) / len2).sqrt()
}

fn eval_point(cp: &[[f64; 3]], w: &[f64], u: &[f64], p: usize, t: f64) -> [f64; 3] {
    let mut num = [0.0f64; 3];
    let mut den = 0.0f64;
    for i in 0..cp.len() {
        let b = basis(i, p, t, u) * w[i];
        den += b;
        for k in 0..3 {
            num[k] += b * cp[i][k];
        }
    }
    if den.abs() < 1e-12 {
        return cp[0];
    }
    [num[0] / den, num[1] / den, num[2] / den]
}

fn basis(i: usize, p: usize, t: f64, u: &[f64]) -> f64 {
    if p == 0 {
        return if u[i] <= t && t < u[i + 1] { 1.0 } else { 0.0 };
    }
    let mut left = 0.0;
    let d1 = u[i + p] - u[i];
    if d1 > 0.0 {
        left = (t - u[i]) / d1 * basis(i, p - 1, t, u);
    }
    let mut right = 0.0;
    let d2 = u[i + p + 1] - u[i + 1];
    if d2 > 0.0 {
        right = (u[i + p + 1] - t) / d2 * basis(i + 1, p - 1, t, u);
    }
    left + right
}

// ---------- header (from the model, step-io >= 0.2) ----------

fn build_header(h: &step_io::FileHeader) -> Header {
    // "AP242 ed2 (IS)"; an unrecognized schema prints its FILE_SCHEMA names.
    let schema = h.schema.to_string();
    Header {
        file_name: h.file_name.clone(),
        time_stamp: h.time_stamp.clone(),
        author: h.authors.clone(),
        organization: h.organizations.clone(),
        preprocessor: h.preprocessor_version.clone(),
        originating_system: h.originating_system.clone(),
        authorisation: h.authorisation.clone(),
        description: h.descriptions.join(", "),
        schema,
        raw_schema: h.schema.raw.clone(),
    }
}

// ---------- small helpers ----------

fn kind_name(debug: &str) -> String {
    debug
        .split(['(', ' ', '{'])
        .next()
        .unwrap_or(debug)
        .to_string()
}

fn kv(k: &str, v: &str) -> [String; 2] {
    [k.to_string(), v.to_string()]
}

/// `Rgb` (each channel 0..1) to a `#rrggbb` hex string; the front-end renders a
/// colour swatch for any meta value under the `color` key.
fn rgb_hex(c: Rgb) -> String {
    let q = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c.red), q(c.green), q(c.blue))
}
