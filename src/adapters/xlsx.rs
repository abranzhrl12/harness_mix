//! Native Excel XLSX adapter using calamine.
//! Fast, read-only inspecting and bounded range reading.

use crate::adapters::project::Project;
use calamine::{Data, Reader, Xlsx, open_workbook};
use harness_domain::{
    Failure,
    execution::OperationContext,
    hash,
    xlsx::{XlsxInspectArgs, XlsxReadArgs},
};
use serde_json::{Value, json};

fn parse_cell_coord(coord: &str) -> Option<(u32, u32)> {
    let mut col: u32 = 0;
    let mut row_str = String::new();
    for c in coord.chars() {
        if c.is_ascii_alphabetic() {
            let val = (c.to_ascii_uppercase() as u32) - ('A' as u32) + 1;
            col = col * 26 + val;
        } else if c.is_ascii_digit() {
            row_str.push(c);
        }
    }
    if col == 0 || row_str.is_empty() {
        return None;
    }
    let row = row_str.parse::<u32>().ok()?;
    if row == 0 {
        return None;
    }
    Some((row - 1, col - 1))
}

fn parse_range(range_str: &str) -> ((u32, u32), (u32, u32)) {
    if let Some((start_s, end_s)) = range_str.split_once(':') {
        let start = parse_cell_coord(start_s).unwrap_or((0, 0));
        let end = parse_cell_coord(end_s).unwrap_or((start.0 + 50, start.1 + 25));
        (start, end)
    } else if let Some(single) = parse_cell_coord(range_str) {
        (single, single)
    } else {
        ((0, 0), (50, 25))
    }
}

pub fn inspect(
    project: &Project,
    args: &XlsxInspectArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if !args.path.ends_with(".xlsx") {
        return Err(("ARGUMENT", "Se requiere una ruta relativa .xlsx"));
    }
    let full_path = project.root.join(&args.path);
    if !full_path.exists() {
        return Err(("NOT_FOUND", "Archivo XLSX no encontrado"));
    }

    let bytes = std::fs::read(&full_path).map_err(|_| ("IO", "No se pudo leer el archivo XLSX"))?;
    let file_hash = hash(&bytes);

    if let Some(expected) = &args.expected_hash
        && expected != &file_hash
    {
        return Err((
            "STALE",
            "El libro cambió; volver a inspeccionar antes de continuar",
        ));
    }

    let mut workbook: Xlsx<_> = open_workbook(&full_path)
        .map_err(|_| ("XLSX_INVALID", "Libro XLSX dañado o formato no admitido"))?;

    let sheet_names = workbook.sheet_names().to_vec();
    let mut sheets = Vec::new();

    for name in &sheet_names {
        if let Ok(range) = workbook.worksheet_range(name) {
            sheets.push(json!({
                "name": name,
                "rows": range.height(),
                "columns": range.width()
            }));
        } else {
            sheets.push(json!({
                "name": name,
                "rows": 0,
                "columns": 0
            }));
        }
    }

    Ok(json!({
        "path": args.path,
        "hash": file_hash,
        "sheets": sheets,
        "engine": "calamine-rust",
        "fileBytes": bytes.len()
    }))
}

pub fn read(
    project: &Project,
    args: &XlsxReadArgs,
    _context: &OperationContext,
) -> Result<Value, Failure> {
    if !args.path.ends_with(".xlsx") {
        return Err(("ARGUMENT", "Se requiere una ruta relativa .xlsx"));
    }
    let full_path = project.root.join(&args.path);
    if !full_path.exists() {
        return Err(("NOT_FOUND", "Archivo XLSX no encontrado"));
    }

    let bytes = std::fs::read(&full_path).map_err(|_| ("IO", "No se pudo leer el archivo XLSX"))?;
    let file_hash = hash(&bytes);

    if let Some(expected) = &args.expected_hash
        && expected != &file_hash
    {
        return Err((
            "STALE",
            "El libro cambió; volver a inspeccionar antes de continuar",
        ));
    }

    let mut workbook: Xlsx<_> = open_workbook(&full_path)
        .map_err(|_| ("XLSX_INVALID", "Libro XLSX dañado o formato no admitido"))?;

    let range_obj = workbook
        .worksheet_range(&args.sheet)
        .map_err(|_| ("XLSX_SHEET", "La hoja indicada no existe"))?;

    let default_range = "A1:Z50".to_string();
    let r_str = args.range.as_ref().unwrap_or(&default_range);
    let (start_cell, end_cell) = parse_range(r_str);

    let row_offset = args.row_offset.unwrap_or(0) as u32;
    let page_size = args.page_size.unwrap_or(50).clamp(1, 100) as u32;

    let total_height = range_obj.height() as u32;
    let total_width = range_obj.width() as u32;

    let r_start = (start_cell.0 + row_offset).min(total_height);
    let r_end = (r_start + page_size).min(end_cell.0 + 1).min(total_height);

    let c_start = start_cell.1.min(total_width);
    let c_end = (end_cell.1 + 1).min(total_width);

    let mut rows = Vec::new();
    for r in r_start..r_end {
        let mut row_cells = Vec::new();
        for c in c_start..c_end {
            let val = match range_obj.get((r as usize, c as usize)) {
                Some(Data::Int(n)) => json!(n),
                Some(Data::Float(f)) => json!(f),
                Some(Data::String(s)) => Value::String(s.clone()),
                Some(Data::Bool(b)) => json!(b),
                Some(Data::DateTime(d)) => json!(d.as_f64()),
                Some(Data::DateTimeIso(s)) => Value::String(s.clone()),
                Some(Data::DurationIso(s)) => Value::String(s.clone()),
                Some(Data::Empty) | None => Value::Null,
                Some(Data::Error(e)) => Value::String(format!("#ERR:{e:?}")),
            };
            row_cells.push(val);
        }
        rows.push(row_cells);
    }

    let has_more = r_end < total_height && r_end <= end_cell.0;

    Ok(json!({
        "path": args.path,
        "sheet": args.sheet,
        "hash": file_hash,
        "rows": rows,
        "rowOffset": row_offset,
        "pageSize": page_size,
        "rowCount": rows.len(),
        "hasMore": has_more,
        "engine": "calamine-rust"
    }))
}
