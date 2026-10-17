//! Integration tests for XLSX inspection and bounded reading.

use agent_harness::adapters;
use harness_domain::{
    contracts::{BatchArgs, Request},
    xlsx::{XlsxInspectArgs, XlsxReadArgs},
};
use serde_json::json;
use std::fs;
use tempfile::tempdir;

const SAMPLE_XLSX: &[u8] = include_bytes!("sample.xlsx");

#[test]
fn test_xlsx_inspect_and_read() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    fs::write(dir.path().join("reporte.xlsx"), SAMPLE_XLSX).unwrap();

    // 1. Inspect
    let inspect_req = Request::XlsxInspect(XlsxInspectArgs {
        path: "reporte.xlsx".to_string(),
        expected_hash: None,
        session: None,
        external_scope: None,
    });
    let result = app.execute(inspect_req);
    assert_eq!(result["state"], "ok", "{result}");
    assert_eq!(result["data"]["path"], "reporte.xlsx");
    let hash = result["data"]["hash"].as_str().unwrap().to_string();
    assert!(!hash.is_empty());

    let sheets = result["data"]["sheets"].as_array().unwrap();
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[0]["name"], "Ventas");
    assert_eq!(sheets[0]["rows"], 4);
    assert_eq!(sheets[0]["columns"], 4);
    assert_eq!(sheets[1]["name"], "Metricas");
    assert_eq!(sheets[1]["rows"], 2);
    assert_eq!(sheets[1]["columns"], 2);

    // 2. Read full range from Ventas
    let read_req = Request::XlsxRead(XlsxReadArgs {
        path: "reporte.xlsx".to_string(),
        sheet: "Ventas".to_string(),
        range: Some("A1:D4".to_string()),
        row_offset: None,
        page_size: None,
        expected_hash: Some(hash.clone()),
        formulas: None,
        session: None,
        external_scope: None,
    });
    let read_res = app.execute(read_req);
    assert_eq!(read_res["state"], "ok", "{read_res}");
    assert_eq!(read_res["data"]["rowCount"], 4);
    assert_eq!(read_res["data"]["rows"][0][0], "ID");
    assert_eq!(read_res["data"]["rows"][0][1], "Producto");
    assert_eq!(read_res["data"]["rows"][1][0], 1.0);
    assert_eq!(read_res["data"]["rows"][1][1], "Laptop");
    assert_eq!(read_res["data"]["rows"][1][2], 1250.50);
    assert_eq!(read_res["data"]["rows"][1][3], true);

    // 3. Paging
    let page_req = Request::XlsxRead(XlsxReadArgs {
        path: "reporte.xlsx".to_string(),
        sheet: "Ventas".to_string(),
        range: Some("A2:D4".to_string()),
        row_offset: Some(1),
        page_size: Some(1),
        expected_hash: Some(hash.clone()),
        formulas: None,
        session: None,
        external_scope: None,
    });
    let page_res = app.execute(page_req);
    assert_eq!(page_res["state"], "ok", "{page_res}");
    assert_eq!(page_res["data"]["rowCount"], 1);
    assert_eq!(page_res["data"]["rows"][0][1], "Mouse");

    // 4. Stale hash rejection
    let stale_req = Request::XlsxInspect(XlsxInspectArgs {
        path: "reporte.xlsx".to_string(),
        expected_hash: Some("badhash".to_string()),
        session: None,
        external_scope: None,
    });
    let stale_res = app.execute(stale_req);
    assert_eq!(stale_res["state"], "error");
    assert_eq!(stale_res["diagnostics"][0]["code"], "STALE");

    // 5. Non-existent sheet
    let bad_sheet_req = Request::XlsxRead(XlsxReadArgs {
        path: "reporte.xlsx".to_string(),
        sheet: "NoExiste".to_string(),
        range: None,
        row_offset: None,
        page_size: None,
        expected_hash: None,
        formulas: None,
        session: None,
        external_scope: None,
    });
    let sheet_res = app.execute(bad_sheet_req);
    assert_eq!(sheet_res["state"], "error");
    assert_eq!(sheet_res["diagnostics"][0]["code"], "XLSX_SHEET");
}

#[test]
fn test_xlsx_batch_execution() {
    let dir = tempdir().unwrap();
    let app = adapters::application(dir.path()).unwrap();

    fs::write(dir.path().join("reporte.xlsx"), SAMPLE_XLSX).unwrap();

    let batch_req = Request::Batch(serde_json::from_value::<BatchArgs>(json!({
        "jobs": [
            {
                "tool": "harness_xlsx_inspect",
                "arguments": {
                    "path": "reporte.xlsx"
                }
            },
            {
                "tool": "harness_xlsx_read",
                "arguments": {
                    "path": "reporte.xlsx",
                    "sheet": "Metricas"
                }
            }
        ]
    })).unwrap());

    let batch_res = app.execute(batch_req);
    assert_eq!(batch_res["state"], "ok", "{batch_res}");
    let results = batch_res["data"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["state"], "ok");
    assert_eq!(results[1]["state"], "ok");
    assert_eq!(results[1]["data"]["rows"][1][0], "Total");
    assert_eq!(results[1]["data"]["rows"][1][1], 1350.70);
}
