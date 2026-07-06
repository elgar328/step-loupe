//! WASM glue for step-loupe (the step-io STEP viewer): `load_step(bytes)` reads a STEP
//! file, walks the scene, and returns a JSON bundle (report, header, tree,
//! geometry) that the HTML/three.js front-end renders.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use step_io::scene::geometry::{Edge, Solid, Vertex};
use step_io::scene::pmi::{Feature, FeatureGeometry};
use step_io::scene::product::{ApprovalDate, Person, ProductDef, Transform};
use step_io::scene::Rgb;
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

#[derive(Serialize, Default)]
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
    norm: Vec<String>,
}

#[derive(Serialize)]
struct Node {
    id: u32,
    kind: &'static str,
    label: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    meta: Vec<[String; 2]>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    children: Vec<Node>,
}

/// Renderable geometry for a node id. `t`: "line" (polyline pts), "point"
/// ([x,y,z]), or "mesh" (pts + `tri` indices).
#[derive(Serialize)]
struct Geom {
    t: &'static str,
    d: Vec<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tri: Vec<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    fallback: bool,
}

#[derive(Serialize)]
struct Output {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    header: Header,
    report: Option<Report>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    units: Vec<[String; 2]>,
    tree: Vec<Node>,
    geom: HashMap<u32, Geom>,
    /// Node id -> rendered geometry ids to highlight when that node is selected.
    /// Only carries entries for nodes without their own geometry subtree
    /// (features and PMI, which reference geometry indirectly).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    hl: HashMap<u32, Vec<u32>>,
    warnings: Vec<String>,
}

// ---------- entry point ----------

#[wasm_bindgen]
pub fn load_step(bytes: &[u8]) -> String {
    let (model, rep) = match step_io::read(bytes) {
        Ok(v) => v,
        Err(e) => {
            return json(&Output {
                ok: false,
                error: Some(format!("parse failed: {e}")),
                header: Header::default(),
                report: None,
                units: vec![],
                tree: vec![],
                geom: HashMap::new(),
                hl: HashMap::new(),
                warnings: vec![],
            });
        }
    };

    let header = build_header(model.header());

    let report = Report {
        n_in: rep.n_in,
        validated: rep.validated,
        n_synth: rep.n_synth,
        dropped: rep
            .dropped
            .iter()
            .map(|(id, r)| (*id, format!("{:?}: {}", r.kind, r.key)))
            .collect(),
        norm: rep.norm.iter().map(|s| s.to_string()).collect(),
    };

    let scene = model.scene();
    let mut b = Builder::default();
    let mut roots = Vec::new();

    // Model tree: assembly hierarchy (root_definitions -> occurrences), with
    // each occurrence's placement transform baked into the emitted geometry.
    let mut visited = Vec::new();
    let mut emitted = HashSet::new();
    let mut model_nodes = Vec::new();
    for def in scene.root_definitions() {
        model_nodes.push(b.emit_def(&def, &IDENTITY, true, None, &mut visited, &mut emitted));
    }
    // Canonical top-level categories are always emitted (empty ones render
    // disabled), so "unsupported" reads differently from "absent in this file".
    roots.push(group("Model", model_nodes));

    // Fallback: any solid not reached through the product structure is shown at
    // the origin, so geometry-only files never lose their geometry.
    let mut orphans = Vec::new();
    for solid in scene.all_solids() {
        if !emitted.contains(&solid.key()) {
            let mut imap = HashMap::new();
            orphans.push(b.solid_node(&solid, &IDENTITY, &mut imap));
        }
    }
    roots.push(group("Unplaced solids", orphans));

    // Node id -> rendered geometry ids to highlight (features/PMI only).
    let mut out_hl: HashMap<u32, Vec<u32>> = HashMap::new();

    // Features first: build the nodes and a key -> (node id, highlight ids) map
    // so PMI can cross-reference each by number and inherit its 3D highlight.
    // Description and referenced geometry go to the detail pane as meta rows.
    let mut feat_map: HashMap<EntityKey, (u32, Vec<u32>)> = HashMap::new();
    let mut features = Vec::new();
    for f in scene.features() {
        let node_id = b.next();
        let mut meta = Vec::new();
        if let Some(desc) = f.description() {
            if !desc.is_empty() {
                meta.push(kv("description", desc));
            }
        }
        let mut hl: Vec<u32> = Vec::new();
        for g in f.geometry().iter() {
            meta.push(kv("ref", &fg_label(g)));
            if let Some(ids) = b.hl_ids.get(&fg_key(g)) {
                hl.extend(ids);
            }
        }
        if !hl.is_empty() {
            out_hl.insert(node_id, hl.clone());
        }
        feat_map.insert(f.key(), (node_id, hl));
        features.push(leaf(node_id, "Feature", feature_label(&f), meta));
    }

    // PMI. Datums are built first so tolerances can cross-reference them by
    // number; in the group they display after dimensions and tolerances.
    let mut datum_map: HashMap<EntityKey, u32> = HashMap::new();
    let mut datum_nodes = Vec::new();
    for dt in scene.datums() {
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
        datum_nodes.push(leaf(id, "Datum", format!("Datum {}", dt.letter()), meta));
    }

    let mut pmi = Vec::new();
    for d in scene.dimensions() {
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
        pmi.push(leaf(id, "Dimension", label, meta));
    }
    for t in scene.tolerances() {
        let id = b.next();
        let mut meta = Vec::new();
        let mut tail = String::new();
        if let Some(m) = t.magnitude() {
            meta.push(kv("magnitude", &m.to_string()));
            tail.push_str(&format!(": {m}"));
        }
        let ds = t.datums();
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
        pmi.push(leaf(
            id,
            "Tolerance",
            format!("Tolerance ({:?}){tail}", t.kind()),
            meta,
        ));
    }
    pmi.extend(datum_nodes);
    roots.push(group("PMI", pmi));
    roots.push(group("Features", features));

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
            let pts = m.points();
            let tris = m.triangles();
            let mut d = Vec::with_capacity(pts.len() * 3);
            for p in &pts {
                d.extend([p[0] as f32, p[1] as f32, p[2] as f32]);
            }
            let mut tri = Vec::with_capacity(tris.len() * 3);
            for t in &tris {
                tri.extend([t[0] as u32, t[1] as u32, t[2] as u32]);
            }
            b.geom.insert(
                id,
                Geom {
                    t: "mesh",
                    d,
                    tri,
                    fallback: false,
                },
            );
            meshes.push(leaf(
                id,
                "Mesh",
                format!("Mesh ({} tris)", tris.len()),
                vec![],
            ));
        }
    }
    roots.push(group("Meshes", meshes));

    json(&Output {
        ok: true,
        error: None,
        header,
        report: Some(report),
        units,
        tree: roots,
        geom: b.geom,
        hl: out_hl,
        warnings: scene.warnings(),
    })
}

// ---------- scene walk helpers ----------

#[derive(Default)]
struct Builder {
    next: u32,
    geom: HashMap<u32, Geom>,
    /// Geometry EntityKey -> rendered leaf ids (edge lines, vertex points) under
    /// it, accumulated across every placed instance. Lets features/PMI resolve
    /// the faces/edges they reference back to highlightable 3D objects.
    hl_ids: HashMap<EntityKey, Vec<u32>>,
}

impl Builder {
    fn next(&mut self) -> u32 {
        let i = self.next;
        self.next += 1;
        i
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
    ) -> Node {
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
                children.push(self.solid_node(&solid, world, &mut imap));
            }
            // placed children (occurrences carry the transform)
            for occ in def.occurrences() {
                if let Some(child) = occ.definition() {
                    let tf = occ.transform();
                    let local = tf.as_ref().and_then(|t| t.matrix()).unwrap_or(IDENTITY);
                    let child_world = mat_mul(world, &local);
                    children.push(self.emit_def(&child, &child_world, false, tf, visited, emitted));
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
        Node {
            id: node_id,
            kind: if is_root { "Part" } else { "Component" },
            label,
            meta,
            children,
        }
    }

    fn solid_node(
        &mut self,
        solid: &Solid<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> Node {
        let sid = self.imap_id(imap, solid.key());
        let mut faces = Vec::new();
        let mut face_count = 0u32;
        let mut solid_edge_ids: Vec<u32> = Vec::new();
        for face in solid.faces() {
            face_count += 1;
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
            fmeta.push(kv("same sense", &face.same_sense().to_string()));
            let mut bounds = Vec::new();
            let mut face_edge_ids: Vec<u32> = Vec::new();
            for bound in face.bounds() {
                let bid = self.imap_id(imap, bound.key());
                let mut edges = Vec::new();
                for (edge, _fwd) in bound.oriented_edges() {
                    let e = self.edge_node(&edge, world, imap);
                    face_edge_ids.push(e.id);
                    edges.push(e);
                }
                bounds.push(Node {
                    id: bid,
                    kind: "Bound",
                    label: "Bound".into(),
                    meta: vec![],
                    children: edges,
                });
            }
            // a feature referencing this face highlights the face's edges
            self.hl_ids
                .entry(face.key())
                .or_default()
                .extend(&face_edge_ids);
            solid_edge_ids.extend(&face_edge_ids);
            faces.push(Node {
                id: fid,
                kind: "Face",
                label: format!("Face ({sk})"),
                meta: fmeta,
                children: bounds,
            });
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
        Node {
            id: sid,
            kind: "Solid",
            label: "Solid".into(),
            meta,
            children: faces,
        }
    }

    fn edge_node(
        &mut self,
        edge: &Edge<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> Node {
        let eid = self.imap_id(imap, edge.key());
        self.hl_ids.entry(edge.key()).or_default().push(eid);
        if !self.geom.contains_key(&eid) {
            let (d, fallback) = sample_edge(edge, world);
            self.geom.insert(
                eid,
                Geom {
                    t: "line",
                    d,
                    tri: vec![],
                    fallback,
                },
            );
        }
        let curve = edge.curve();
        let cname = kind_name(&format!("{:?}", curve.kind()));
        let mut meta = vec![kv("curve", &cname)];
        if let Some(c) = curve.color() {
            meta.push(kv("color", &rgb_hex(c)));
        }
        if let Some(w) = curve.width() {
            meta.push(kv("width", &w.to_string()));
        }
        if let Some(f) = curve.line_font() {
            meta.push(kv("line font", f));
        }
        meta.push(kv("same sense", &edge.same_sense().to_string()));
        let mut children = Vec::new();
        for v in [edge.start(), edge.end()].into_iter().flatten() {
            children.push(self.vertex_node(&v, world, imap));
        }
        Node {
            id: eid,
            kind: "Edge",
            label: format!("Edge ({cname})"),
            meta,
            children,
        }
    }

    fn vertex_node(
        &mut self,
        v: &Vertex<'_>,
        world: &Mat,
        imap: &mut HashMap<EntityKey, u32>,
    ) -> Node {
        let vid = self.imap_id(imap, v.key());
        self.hl_ids.entry(v.key()).or_default().push(vid);
        let mut meta = vec![];
        if let Some(p) = v.point() {
            // A feature may reference the cartesian point directly; map it to the
            // vertex's rendered point so such references still highlight.
            self.hl_ids.entry(p.key()).or_default().push(vid);
            if !self.geom.contains_key(&vid) {
                let c = apply(world, p.xyz());
                self.geom.insert(
                    vid,
                    Geom {
                        t: "point",
                        d: vec![c[0] as f32, c[1] as f32, c[2] as f32],
                        tri: vec![],
                        fallback: false,
                    },
                );
            }
            let [x, y, z] = p.xyz();
            meta.push(kv("coords", &format!("{x}, {y}, {z}")));
        }
        Node {
            id: vid,
            kind: "Vertex",
            label: "Vertex".into(),
            meta,
            children: vec![],
        }
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
            .iter()
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
/// it up in `Builder::hl_ids`.
fn fg_key(g: &FeatureGeometry<'_>) -> EntityKey {
    match g {
        FeatureGeometry::Face(h) => h.key(),
        FeatureGeometry::Edge(h) => h.key(),
        FeatureGeometry::Point(h) => h.key(),
        FeatureGeometry::Solid(h) => h.key(),
        FeatureGeometry::Curve(h) => h.key(),
        FeatureGeometry::Other(k) => *k,
    }
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
        FeatureGeometry::Other(k) => format!("Other {k:?}"),
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
        if !v.is_empty() {
            meta.push(kv("version", v));
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
    if !(u1 > u0) {
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
    // Recognized schemas render a clean label ("AP242 ed2 (IS)"); an
    // unrecognized family renders "unrecognized schema", so fall back to the
    // raw FILE_SCHEMA strings ("ISO-10303-042") which are more informative.
    let schema = if matches!(h.schema.family, step_io::ApFamily::Other) {
        h.schema
            .raw()
            .map(|r| r.as_slice().join(", "))
            .unwrap_or_else(|| h.schema.to_string())
    } else {
        h.schema.to_string()
    };
    Header {
        file_name: h.file_name.clone(),
        time_stamp: h.time_stamp.clone(),
        author: h.authors.clone(),
        organization: h.organizations.clone(),
        preprocessor: h.preprocessor_version.clone(),
        originating_system: h.originating_system.clone(),
        authorisation: h.authorisation.clone(),
        description: h.description.clone(),
        schema,
        raw_schema: h
            .schema
            .raw()
            .map(|r| r.as_slice().to_vec())
            .unwrap_or_default(),
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

fn leaf(id: u32, kind: &'static str, label: String, meta: Vec<[String; 2]>) -> Node {
    Node {
        id,
        kind,
        label,
        meta,
        children: vec![],
    }
}

fn group(label: &str, children: Vec<Node>) -> Node {
    Node {
        id: u32::MAX,
        kind: "Group",
        label: label.to_string(),
        meta: vec![],
        children,
    }
}

fn json(o: &Output) -> String {
    serde_json::to_string(o).unwrap_or_else(|e| format!("{{\"ok\":false,\"error\":\"{e}\"}}"))
}
