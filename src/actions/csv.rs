use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::errors::{MCSError, Result};
use crate::validation;
use memmap2::Mmap;

// ── Tool: csv_create ─────────────────────────────────────

pub async fn csv_create(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_destination(&path, config)?;

    if valid_path.exists() {
        let overwrite = get_opt_bool(args, "overwrite").unwrap_or(false);
        if !overwrite {
            return Err(MCSError::FilesystemError(format!("File already exists: {path}. Set 'overwrite': true to replace.")));
        }
    }

    let headers: Vec<String> = get_str_array(args, "headers")?;
    if headers.is_empty() {
        return Err(MCSError::InvalidParams("'headers' must be a non-empty array".into()));
    }

    let rows: Vec<Vec<String>> = get_opt_str_array_of_arrays(args, "rows").unwrap_or_default();
    for (i, row) in rows.iter().enumerate() {
        if row.len() != headers.len() {
            return Err(MCSError::InvalidParams(format!(
                "Row {i} has {} values, expected {}", row.len(), headers.len()
            )));
        }
    }

    let dst = valid_path.clone();
    let hdrs = headers.clone();
    let rows_count = rows.len();
    tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        let mut wtr = csv::Writer::from_path(&dst)
            .map_err(|e| format!("Cannot create CSV writer: {e}"))?;
        wtr.write_record(&hdrs)
            .map_err(|e| format!("Cannot write headers: {e}"))?;
        for row in &rows {
            wtr.write_record(row)
                .map_err(|e| format!("Cannot write row: {e}"))?;
        }
        wtr.flush().map_err(|e| format!("Cannot flush: {e}"))?;
        Ok(())
    }).await.map_err(|e| MCSError::FilesystemError(format!("CSV task failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "headers": headers,
        "rowsCreated": rows_count,
    }))
}

// ── Tool: csv_read ───────────────────────────────────────

pub async fn csv_read(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let limit = get_opt_u64(args, "limit").unwrap_or(u64::MAX).min(10_000);
    let offset = get_opt_u64(args, "offset").unwrap_or(0);
    let filter_cols: Option<Vec<String>> = get_opt_str_array(args, "columns");

    let data = read_csv_file(&valid_path).await?;
    let total_rows = data.rows.len();

    if let Some(ref cols) = filter_cols {
        let col_indices: Vec<usize> = cols.iter()
            .map(|c| find_column(&data, c))
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let start = offset.min(total_rows as u64) as usize;
        let end = (offset.saturating_add(limit)).min(total_rows as u64) as usize;
        let page: Vec<Vec<String>> = data.rows[start..end].iter()
            .map(|row| col_indices.iter().map(|&i| row.get(i).cloned().unwrap_or_default()).collect())
            .collect();

        Ok(json!({
            "success": true,
            "path": valid_path.to_string_lossy(),
            "headers": cols,
            "rows": page,
            "totalRows": total_rows,
            "offset": offset,
            "returnedRows": page.len(),
        }))
    } else {
        let start = offset.min(total_rows as u64) as usize;
        let end = (offset.saturating_add(limit)).min(total_rows as u64) as usize;
        let page: Vec<Vec<String>> = data.rows[start..end].to_vec();

        Ok(json!({
            "success": true,
            "path": valid_path.to_string_lossy(),
            "headers": data.headers,
            "rows": page,
            "totalRows": total_rows,
            "offset": offset,
            "returnedRows": page.len(),
        }))
    }
}

// ── Tool: csv_add_row ────────────────────────────────────

pub async fn csv_add_row(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let added = add_rows_from_args(&mut data, args)?;
    let total_rows = data.rows.len();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "rowsAdded": added,
        "totalRows": total_rows,
    }))
}

// ── Tool: csv_update_cell ────────────────────────────────

pub async fn csv_update_cell(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let row_idx = get_opt_u64(args, "row")
        .ok_or_else(|| MCSError::InvalidParams("Missing required: 'row'".into()))? as usize;
    let value = get_str_arg(args, "value")?;
    let col_idx = resolve_column(&data, args)?;

    if row_idx >= data.rows.len() {
        return Err(MCSError::InvalidParams(format!(
            "Row index {row_idx} out of range. File has {} data rows (0..{})",
            data.rows.len(), data.rows.len().saturating_sub(1)
        )));
    }

    let column_name = data.headers[col_idx].clone();
    // Flexible CSV parsing allows ragged rows shorter than the header; pad so
    // the indexed assignment can never panic.
    let target_len = data.headers.len();
    let row = &mut data.rows[row_idx];
    if col_idx >= row.len() {
        row.resize(target_len.max(col_idx + 1), String::new());
    }
    row[col_idx] = value.clone();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "row": row_idx,
        "column": column_name,
        "value": value,
    }))
}

// ── Tool: csv_remove_row ─────────────────────────────────

pub async fn csv_remove_row(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let row_idx = get_opt_u64(args, "row")
        .ok_or_else(|| MCSError::InvalidParams("Missing required: 'row'".into()))? as usize;

    if row_idx >= data.rows.len() {
        return Err(MCSError::InvalidParams(format!(
            "Row index {row_idx} out of range. File has {} data rows (0..{})",
            data.rows.len(), data.rows.len().saturating_sub(1)
        )));
    }

    data.rows.remove(row_idx);
    let total_rows = data.rows.len();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "removedRowIndex": row_idx,
        "totalRows": total_rows,
    }))
}

// ── Tool: csv_add_column ─────────────────────────────────

pub async fn csv_add_column(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let column = get_str_arg(args, "column")?;
    if data.column_map.contains_key(&column) {
        return Err(MCSError::InvalidParams(format!("Column already exists: '{column}'")));
    }

    let default_val = get_opt_str(args, "defaultValue").unwrap_or_default();
    let col_name = column.clone();
    let new_idx = data.headers.len();
    data.headers.push(column);
    data.column_map.insert(col_name.clone(), new_idx);
    for row in &mut data.rows {
        row.push(default_val.clone());
    }
    let total_rows = data.rows.len();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "column": col_name,
        "defaultValue": default_val,
        "totalRows": total_rows,
    }))
}

// ── Tool: csv_remove_column ──────────────────────────────

pub async fn csv_remove_column(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let col_idx = resolve_column(&data, args)?;
    let col_name = data.headers.remove(col_idx);
    data.column_map = data.headers.iter().enumerate().map(|(i, h)| (h.clone(), i)).collect();
    for row in &mut data.rows {
        if col_idx < row.len() {
            row.remove(col_idx);
        }
    }
    let total_columns = data.headers.len();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "removedColumn": col_name,
        "totalColumns": total_columns,
    }))
}

// ── Tool: csv_rename_column ──────────────────────────────

pub async fn csv_rename_column(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let mut data = read_csv_file(&valid_path).await?;

    let old_name = get_str_arg(args, "oldName")?;
    let new_name = get_str_arg(args, "newName")?;

    if !data.column_map.contains_key(&old_name) {
        return Err(MCSError::InvalidParams(format!("Column not found: '{old_name}'")));
    }
    if data.column_map.contains_key(&new_name) && old_name != new_name {
        return Err(MCSError::InvalidParams(format!("Target column name already exists: '{new_name}'")));
    }

    let old_idx = data.column_map.remove(&old_name).unwrap();
    data.column_map.insert(new_name.clone(), old_idx);
    data.headers[old_idx] = new_name.clone();
    write_csv_file(&valid_path, data).await?;

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "oldName": old_name,
        "newName": new_name,
    }))
}

// ── Tool: csv_read_column_values_range ───────────────────

pub async fn csv_read_column_values_range(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let column = get_str_arg(args, "column")?;
    let start = get_opt_u64(args, "start").unwrap_or(0) as usize;
    let end = get_opt_u64(args, "end").map(|e| e as usize);

    if let Some(end) = end {
        if end < start {
            return Err(MCSError::InvalidParams(format!("end ({end}) must be >= start ({start})")));
        }
        if end - start > 1000 {
            return Err(MCSError::InvalidParams(format!(
                "Range too large: {end} - {start} = {} exceeds maximum of 1000 rows", end - start
            )));
        }
    }

    let data = read_csv_file(&valid_path).await?;
    let col_idx = find_column(&data, &column)?;

    let values: Vec<&str> = data.rows.iter()
        .skip(start)
        .take(end.map(|e| e.saturating_sub(start)).unwrap_or(usize::MAX))
        .map(|row| row.get(col_idx).map(|s| s.as_str()).unwrap_or(""))
        .collect();

    let actual_end = start + values.len();

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "column": column,
        "values": values,
        "start": start,
        "end": actual_end,
        "totalRows": data.rows.len(),
    }))
}

// ── Tool: csv_read_row_range ──────────────────────────────

pub async fn csv_read_row_range(args: Option<&Value>, config: &Config) -> Result<Value> {
    let path = get_str_arg(args, "path")?;
    let valid_path = validation::validate_path(&path, config)?;
    let start = get_opt_u64(args, "start").unwrap_or(0) as usize;
    let end = get_opt_u64(args, "end").map(|e| e as usize);

    if let Some(end) = end {
        if end < start {
            return Err(MCSError::InvalidParams(format!("end ({end}) must be >= start ({start})")));
        }
        if end - start > 100 {
            return Err(MCSError::InvalidParams(format!(
                "Range too large: {end} - {start} = {} exceeds maximum of 100 rows", end - start
            )));
        }
    }

    let data = read_csv_file(&valid_path).await?;

    let rows: Vec<Vec<String>> = data.rows.iter()
        .skip(start)
        .take(end.map(|e| e.saturating_sub(start)).unwrap_or(1))
        .cloned()
        .collect();

    let actual_end = start + rows.len();

    Ok(json!({
        "success": true,
        "path": valid_path.to_string_lossy(),
        "headers": data.headers,
        "rows": rows,
        "start": start,
        "end": actual_end,
        "totalRows": data.rows.len(),
    }))
}

// ── Internal Data ────────────────────────────────────────

struct CsvData {
    headers: Vec<String>,
    column_map: HashMap<String, usize>,
    rows: Vec<Vec<String>>,
}

/// Files below this size are read with a plain `read`; larger ones are mapped.
const CSV_MMAP_THRESHOLD: u64 = 256 * 1024;

async fn read_csv_file(path: &Path) -> Result<CsvData> {
    let path_buf = path.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::result::Result<CsvData, String> {
        let file = std::fs::File::open(&path_buf)
            .map_err(|e| format!("Cannot open CSV: {e}"))?;
        let file_size = file.metadata().ok().map(|m| m.len()).unwrap_or(0);
        if file_size < CSV_MMAP_THRESHOLD {
            let bytes = std::fs::read(&path_buf)
                .map_err(|e| format!("Cannot read CSV: {e}"))?;
            parse_csv(&bytes, file_size)
        } else {
            let mmap = unsafe {
                Mmap::map(&file)
                    .map_err(|e| format!("Cannot mmap CSV: {e}"))?
            };
            parse_csv(&mmap, file_size)
        }
    }).await.map_err(|e| MCSError::FilesystemError(format!("CSV read task failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    Ok(result)
}

fn parse_csv(data: &[u8], file_size: u64) -> std::result::Result<CsvData, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(data);

    let headers: Vec<String> = rdr.headers()
        .map_err(|e| format!("Cannot read CSV headers: {e}"))?
        .iter()
        .map(|s| s.to_string())
        .collect();

    let column_map: HashMap<String, usize> = headers.iter().enumerate()
        .map(|(i, h)| (h.clone(), i))
        .collect();

    let estimated_rows = if file_size > 0 && !headers.is_empty() {
        (file_size as usize / (headers.len() * 16)).max(64)
    } else {
        0
    };
    let mut rows = Vec::with_capacity(estimated_rows);
    for result in rdr.records() {
        let record = result.map_err(|e| format!("Cannot read CSV record: {e}"))?;
        let row: Vec<String> = record.iter().map(|s| s.to_string()).collect();
        rows.push(row);
    }

    Ok(CsvData { headers, column_map, rows })
}

async fn write_csv_file(path: &Path, data: CsvData) -> Result<()> {
    let path_buf = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        let mut wtr = csv::Writer::from_path(&path_buf)
            .map_err(|e| format!("Cannot create CSV writer: {e}"))?;
        wtr.write_record(&data.headers)
            .map_err(|e| format!("Cannot write headers: {e}"))?;
        for row in &data.rows {
            wtr.write_record(row)
                .map_err(|e| format!("Cannot write row: {e}"))?;
        }
        wtr.flush().map_err(|e| format!("Cannot flush: {e}"))?;
        Ok(())
    }).await.map_err(|e| MCSError::FilesystemError(format!("CSV write task failed: {e}")))?
      .map_err(MCSError::FilesystemError)?;

    Ok(())
}

fn resolve_column(data: &CsvData, args: Option<&Value>) -> Result<usize> {
    if let Some(col_name) = get_opt_str(args, "column") {
        find_column(data, &col_name)
    } else if let Some(col_idx) = get_opt_u64(args, "columnIndex") {
        let idx = col_idx as usize;
        if idx >= data.headers.len() {
            Err(MCSError::InvalidParams(format!("Column index {idx} out of range (0..{})", data.headers.len())))
        } else {
            Ok(idx)
        }
    } else {
        Err(MCSError::InvalidParams("Missing required: specify 'column' (name) or 'columnIndex' (number)".into()))
    }
}

fn find_column(data: &CsvData, name: &str) -> Result<usize> {
    data.column_map.get(name).copied()
        .ok_or_else(|| MCSError::InvalidParams(
            format!("Column not found: '{name}'. Available: {}", data.headers.join(", "))
        ))
}

fn add_rows_from_args(data: &mut CsvData, args: Option<&Value>) -> Result<usize> {
    let rows_val = args.and_then(|a| a.get("rows"))
        .ok_or_else(|| MCSError::InvalidParams("Missing required: 'rows'".into()))?;

    let rows_arr = rows_val.as_array()
        .ok_or_else(|| MCSError::InvalidParams("'rows' must be an array".into()))?;

    if rows_arr.is_empty() {
        return Err(MCSError::InvalidParams("'rows' must be a non-empty array".into()));
    }

    let mut count = 0;
    for (i, item) in rows_arr.iter().enumerate() {
        if let Some(obj) = item.as_object() {
            // Object format: {"column_name": "value", ...}
            let mut row = vec![String::new(); data.headers.len()];
            for (key, val) in obj {
                let col_idx = find_column(data, key)?;
                row[col_idx] = val_as_string(val);
            }
            data.rows.push(row);
        } else if let Some(arr) = item.as_array() {
            // Array format: ["val1", "val2", ...]
            let row: Vec<String> = arr.iter().map(val_as_string).collect();
            if row.len() != data.headers.len() {
                return Err(MCSError::InvalidParams(format!(
                    "Row {i} has {} values, expected {}", row.len(), data.headers.len()
                )));
            }
            data.rows.push(row);
        } else {
            return Err(MCSError::InvalidParams(format!("Row {i} must be an object or array")));
        }
        count += 1;
    }

    Ok(count)
}

fn val_as_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ── Argument Helpers ─────────────────────────────────────

fn get_str_arg(args: Option<&Value>, name: &str) -> Result<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| MCSError::InvalidParams(format!("Missing required: '{name}'")))
}

fn get_opt_str(args: Option<&Value>, name: &str) -> Option<String> {
    args.and_then(|a| a.get(name))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_opt_bool(args: Option<&Value>, name: &str) -> Option<bool> {
    args.and_then(|a| a.get(name)).and_then(|v| v.as_bool())
}

fn get_opt_u64(args: Option<&Value>, name: &str) -> Option<u64> {
    args.and_then(|a| a.get(name)).and_then(|v| v.as_u64())
}

fn get_str_array(args: Option<&Value>, name: &str) -> Result<Vec<String>> {
    let arr = args.and_then(|a| a.get(name))
        .and_then(|v| v.as_array())
        .ok_or_else(|| MCSError::InvalidParams(format!("Missing required: '{name}' (must be an array)")))?;
    let mut result = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let s = v.as_str().ok_or_else(|| {
            MCSError::InvalidParams(format!("'{name}[{i}]' must be a string"))
        })?;
        result.push(s.to_string());
    }
    Ok(result)
}

fn get_opt_str_array(args: Option<&Value>, name: &str) -> Option<Vec<String>> {
    let arr = args.and_then(|a| a.get(name)).and_then(|v| v.as_array())?;
    let result: Vec<String> = arr.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect();
    Some(result)
}

fn get_opt_str_array_of_arrays(args: Option<&Value>, name: &str) -> Option<Vec<Vec<String>>> {
    let arr = args.and_then(|a| a.get(name)).and_then(|v| v.as_array())?;
    let mut result = Vec::new();
    for item in arr {
        if let Some(inner) = item.as_array() {
            let row: Vec<String> = inner.iter().map(val_as_string).collect();
            result.push(row);
        } else {
            return None;
        }
    }
    Some(result)
}
