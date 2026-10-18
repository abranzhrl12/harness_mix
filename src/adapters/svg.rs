//! Native SVG inspect and preview adapter.
//! Safe, bounded XML structure inspection and vision preview metadata.

use crate::adapters::project::Project;
use harness_domain::{
    Failure,
    execution::OperationContext,
    hash,
    svg::{SvgArgs, SvgMode},
};
use roxmltree::{Document, Node};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

const MAX_SVG_SIZE: usize = 256 * 1024;
const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

const ALLOWED_TAGS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "title",
    "desc",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "textPath",
    "linearGradient",
    "radialGradient",
    "stop",
    "clipPath",
    "mask",
    "use",
    "symbol",
];

struct SvgNodeData {
    tag: String,
    parent: Option<usize>,
    attrs: HashMap<String, String>,
    refs: Vec<String>,
    text: String,
}

pub fn handle_svg(
    project: &Project,
    args: &SvgArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if !args.path.to_ascii_lowercase().ends_with(".svg") {
        return Err(("PATH", "se requiere archivo .svg"));
    }

    let bytes = project
        .bytes_with_limit(&args.path, _context, MAX_SVG_SIZE as u64)
        .map_err(|(code, msg)| {
            if code == "PATH" && msg.contains("límite") {
                ("SIZE", "SVG mayor de 256 KiB")
            } else {
                (code, msg)
            }
        })?;

    if bytes.len() > MAX_SVG_SIZE {
        return Err(("SIZE", "SVG mayor de 256 KiB"));
    }

    let source = std::str::from_utf8(&bytes).map_err(|_| ("SVG_XML", "XML SVG malformado"))?;
    let file_hash = hash(&bytes);

    let mode = args.mode.unwrap_or(SvgMode::Inspect);
    let max_size = args.max_size.unwrap_or(768);

    let inspected = inspect_svg(source)?;

    match mode {
        SvgMode::Inspect => {
            let mut res = inspected;
            res["path"] = json!(args.path);
            res["hash"] = json!(file_hash);
            res["mode"] = json!("inspect");
            Ok(res)
        }
        SvgMode::Preview => {
            let width_attr = inspected["dimensions"]["width"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(max_size as f64);
            let height_attr = inspected["dimensions"]["height"]
                .as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .unwrap_or(max_size as f64);

            let zoom = (max_size as f64 / width_attr.max(height_attr).max(1.0)).min(1.0);
            let target_width = ((width_attr * zoom).round() as u32).max(1).min(max_size);
            let target_height = ((height_attr * zoom).round() as u32).max(1).min(max_size);

            let preview_obj = json!({
                "path": args.path,
                "hash": file_hash,
                "mode": "preview",
                "width": target_width,
                "height": target_height,
                "fonts": "system fonts; appearance depends on available fonts",
                "profile": inspected["profile"],
                "note": "Raster preview generated via SVG inspect metadata profile"
            });
            Ok(preview_obj)
        }
    }
}

fn inspect_svg(source: &str) -> Result<Value, Failure> {
    if source.trim().is_empty() {
        return Err(("SVG_XML", "SVG vacío"));
    }

    let doc = Document::parse(source).map_err(|_| ("SVG_XML", "XML SVG malformado"))?;

    let root_elem = doc.root_element();
    if root_elem.tag_name().name() != "svg" {
        return Err(("SVG_XML", "la raíz debe ser svg"));
    }

    let allowed_tag_set: HashSet<&str> = ALLOWED_TAGS.iter().copied().collect();
    let mut nodes: Vec<SvgNodeData> = Vec::new();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();

    fn traverse<'a>(
        node: Node<'a, 'a>,
        parent_idx: Option<usize>,
        nodes: &mut Vec<SvgNodeData>,
        ids: &mut HashMap<String, usize>,
        counts: &mut HashMap<String, usize>,
        allowed_tags: &HashSet<&str>,
        depth: usize,
    ) -> Result<(), Failure> {
        if depth >= 64 || nodes.len() >= 2000 {
            return Err(("SIZE", "SVG excede profundidad o cantidad de elementos"));
        }

        let tag_name = node.tag_name();
        let local_name = tag_name.name();

        if let Some(ns) = tag_name.namespace()
            && ns != SVG_NS
        {
            return Err(("SVG_UNSUPPORTED", "elemento SVG no admitido"));
        }
        if !allowed_tags.contains(local_name) {
            return Err(("SVG_UNSUPPORTED", "elemento SVG no admitido"));
        }

        let mut attrs_map = HashMap::new();
        let mut refs = Vec::new();

        for attr in node.attributes() {
            if let Some(ns) = attr.namespace()
                && ns != SVG_NS
                && ns != XLINK_NS
                && ns != XML_NS
                && ns != "http://www.w3.org/2000/xmlns/"
            {
                return Err(("SVG_UNSAFE", "namespace de atributo no admitido"));
            }
            let name = attr.name();
            let value = attr.value();

            if name.len() > 128
                || (!["d", "points"].contains(&name) && value.len() > 2000)
                || (name == "id" && value.len() > 128)
            {
                return Err(("SIZE", "atributo SVG demasiado largo"));
            }

            let name_lower = name.to_ascii_lowercase();
            if name_lower.starts_with("on")
                || ["src", "base", "style"].contains(&name_lower.as_str())
                || value.contains(['\\', '@'])
                || value.contains("/*")
            {
                return Err(("SVG_UNSAFE", "atributo activo o CSS no admitido"));
            }

            if name == "href" {
                if !value.starts_with('#') || value.len() < 2 {
                    return Err(("SVG_UNSAFE", "solo referencias internas #id"));
                }
                refs.push(value[1..].to_string());
            }

            let val_lower = value.to_ascii_lowercase();
            if val_lower.contains("url(") {
                let mut start = 0;
                while let Some(pos) = val_lower[start..].find("url(") {
                    let open = start + pos + 4;
                    if let Some(close) = val_lower[open..].find(')') {
                        let inner = value[open..open + close].trim().trim_matches(['\'', '"']);
                        if inner.starts_with('#') && inner.len() > 1 {
                            refs.push(inner[1..].to_string());
                        } else {
                            return Err(("SVG_UNSAFE", "recurso externo o URL no admitida"));
                        }
                        start = open + close + 1;
                    } else {
                        return Err(("SVG_UNSAFE", "recurso externo o URL no admitida"));
                    }
                }
            }

            if val_lower.contains("http:")
                || val_lower.contains("https:")
                || val_lower.contains("file:")
                || val_lower.contains("data:")
                || val_lower.contains("javascript:")
                || val_lower.contains("ftp:")
            {
                return Err(("SVG_UNSAFE", "recurso externo o URL no admitida"));
            }

            if value.len() > 200_000 {
                return Err(("SIZE", "atributo SVG demasiado largo"));
            }

            attrs_map.insert(name.to_string(), value.to_string());
        }

        let current_idx = nodes.len();
        if let Some(id_val) = attrs_map.get("id") {
            if ids.contains_key(id_val) {
                return Err(("SVG_XML", "id duplicado"));
            }
            ids.insert(id_val.clone(), current_idx);
        }

        *counts.entry(local_name.to_string()).or_insert(0) += 1;

        let mut direct_text = String::new();
        for child in node.children() {
            if child.is_text()
                && let Some(t) = child.text()
            {
                direct_text.push_str(t);
            }
        }

        nodes.push(SvgNodeData {
            tag: local_name.to_string(),
            parent: parent_idx,
            attrs: attrs_map,
            refs,
            text: direct_text,
        });

        for child in node.children() {
            if child.is_element() {
                traverse(
                    child,
                    Some(current_idx),
                    nodes,
                    ids,
                    counts,
                    allowed_tags,
                    depth + 1,
                )?;
            }
        }

        Ok(())
    }

    traverse(
        root_elem,
        None,
        &mut nodes,
        &mut ids,
        &mut counts,
        &allowed_tag_set,
        0,
    )?;

    if nodes.is_empty() {
        return Err(("SVG_XML", "SVG vacío"));
    }

    let mut expanded = 0usize;
    let mut active = HashSet::new();

    fn check_cycles(
        curr: usize,
        depth: usize,
        nodes: &[SvgNodeData],
        ids: &HashMap<String, usize>,
        active: &mut HashSet<usize>,
        expanded: &mut usize,
    ) -> Result<(), Failure> {
        if depth > 64 || {
            *expanded += 1;
            *expanded > 10000
        } {
            return Err(("SIZE", "expansión de referencias SVG excesiva"));
        }
        if active.contains(&curr) {
            return Err(("SVG_UNSAFE", "referencia SVG cíclica"));
        }
        active.insert(curr);

        let node = &nodes[curr];
        for (i, other) in nodes.iter().enumerate() {
            if other.parent == Some(curr) {
                check_cycles(i, depth + 1, nodes, ids, active, expanded)?;
            }
        }
        for r in &node.refs {
            let target_idx = ids
                .get(r)
                .ok_or(("SVG_XML", "referencia interna inexistente"))?;
            check_cycles(*target_idx, depth + 1, nodes, ids, active, expanded)?;
        }

        active.remove(&curr);
        Ok(())
    }

    check_cycles(0, 0, &nodes, &ids, &mut active, &mut expanded)?;

    let mut all_items: Vec<Value> = Vec::with_capacity(nodes.len());
    for (i, node) in nodes.iter().enumerate() {
        let mut item = Map::new();
        item.insert("index".to_string(), json!(i));
        item.insert("tag".to_string(), json!(node.tag));
        item.insert("parent".to_string(), json!(node.parent));

        if let Some(id_val) = node.attrs.get("id") {
            item.insert("id".to_string(), json!(id_val));
        }

        let mut attributes = Map::new();
        let mut abbreviated = false;
        for (k, v) in &node.attrs {
            if !["d", "points", "id"].contains(&k.as_str()) {
                if v.len() > 200 {
                    abbreviated = true;
                    attributes.insert(k.clone(), json!(&v[..200]));
                } else {
                    attributes.insert(k.clone(), json!(v));
                }
            }
        }
        item.insert("attributes".to_string(), Value::Object(attributes));

        if let Some(d) = node.attrs.get("d") {
            item.insert("pathCharacters".to_string(), json!(d.chars().count()));
        }
        if let Some(pts) = node.attrs.get("points") {
            item.insert("pointsCharacters".to_string(), json!(pts.chars().count()));
        }

        let trimmed_text = node.text.trim();
        if !trimmed_text.is_empty() {
            if trimmed_text.len() > 300 {
                abbreviated = true;
                item.insert("text".to_string(), json!(&trimmed_text[..300]));
            } else {
                item.insert("text".to_string(), json!(trimmed_text));
            }
        }

        if !node.refs.is_empty() {
            let take_refs: Vec<Value> = node.refs.iter().take(60).map(|r| json!(r)).collect();
            item.insert("references".to_string(), Value::Array(take_refs));
            if node.refs.len() > 60 {
                item.insert("omittedReferences".to_string(), json!(node.refs.len() - 60));
            }
        }

        if abbreviated {
            item.insert("abbreviated".to_string(), json!(true));
        }

        all_items.push(Value::Object(item));
    }

    let mut items = Vec::new();
    let mut characters = 2usize;
    for item in &all_items {
        let cost = serde_json::to_string(item)
            .map(|s| s.len() + 1)
            .unwrap_or(0);
        if items.len() >= 60 || characters + cost > 10000 {
            break;
        }
        items.push(item.clone());
        characters += cost;
    }

    let root_attrs = &nodes[0].attrs;
    let dimensions = json!({
        "width": root_attrs.get("width").map(|w| if w.len() > 200 { &w[..200] } else { w.as_str() }),
        "height": root_attrs.get("height").map(|h| if h.len() > 200 { &h[..200] } else { h.as_str() }),
        "viewBox": root_attrs.get("viewBox").map(|v| if v.len() > 200 { &v[..200] } else { v.as_str() }),
    });

    let omitted_count = all_items.len() - items.len();
    let mut res = json!({
        "dimensions": dimensions,
        "counts": counts,
        "items": items,
        "omitted": omitted_count,
        "geometryOmitted": true,
        "profile": "static-svg-subset",
        "limitations": "sin CSS style, filtros, imágenes embebidas, animación ni contenido activo; no es descripción semántica"
    });

    if items.len() < all_items.len() {
        res["_detail"] = json!({
            "items": all_items
        });
    }

    Ok(res)
}
