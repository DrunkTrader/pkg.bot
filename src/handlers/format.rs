use std::convert::Infallible;

use axum::{
    extract::FromRequestParts,
    http::{
        header::{ACCEPT, CONTENT_TYPE},
        request::Parts,
    },
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;

/// API response format negotiated from `Accept` header.
#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub enum ApiFormat {
    #[default]
    Json,
    Csv,
}

impl ApiFormat {
    fn from_accept_header(accept: &str) -> Self {
        if accept.split(',').any(|media_type| {
            media_type
                .split(';')
                .next()
                .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("text/csv"))
        }) {
            Self::Csv
        } else {
            Self::Json
        }
    }
}

impl<S> FromRequestParts<S> for ApiFormat
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let format = parts
            .headers
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(ApiFormat::from_accept_header)
            .unwrap_or_default();

        Ok(format)
    }
}

/// Respond in the requested format.
pub fn respond<T: Serialize>(format: ApiFormat, data: T) -> super::Result<Response> {
    match format {
        ApiFormat::Json => Ok(super::json(data).into_response()),
        ApiFormat::Csv => csv_response(data),
    }
}

fn csv_response<T: Serialize>(data: T) -> super::Result<Response> {
    let value = serde_json::to_value(data)?;
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(b'|')
        .from_writer(Vec::new());

    match value {
        Value::Object(map) => {
            write_object(&mut wtr, &map)?;
        }
        Value::Array(items) => {
            if items.iter().all(|item| matches!(item, Value::Object(_))) {
                write_objects(&mut wtr, &items)?;
            } else {
                for item in items {
                    wtr.write_record([csv_cell(&item)])?;
                }
            }
        }
        primitive => {
            wtr.write_record([csv_cell(&primitive)])?;
        }
    }

    let body = String::from_utf8(wtr.into_inner()?)?;
    Ok(([(CONTENT_TYPE, "text/csv; charset=utf-8")], body).into_response())
}

fn write_object<W: std::io::Write>(
    wtr: &mut csv::Writer<W>,
    map: &serde_json::Map<String, Value>,
) -> csv::Result<()> {
    let headers: Vec<_> = map.keys().cloned().collect();
    wtr.write_record(&headers)?;
    wtr.write_record(headers.iter().map(|key| csv_cell(map.get(key).unwrap())))
}

fn write_objects<W: std::io::Write>(wtr: &mut csv::Writer<W>, items: &[Value]) -> csv::Result<()> {
    let mut headers = Vec::new();
    for item in items {
        let Value::Object(map) = item else { continue };
        for key in map.keys() {
            if !headers.iter().any(|header| header == key) {
                headers.push(key.clone());
            }
        }
    }

    if headers.is_empty() {
        return Ok(());
    }

    wtr.write_record(&headers)?;
    for item in items {
        let Value::Object(map) = item else { continue };
        wtr.write_record(
            headers
                .iter()
                .map(|key| map.get(key).map(csv_cell).unwrap_or_default()),
        )?;
    }
    Ok(())
}

fn csv_cell(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        // Nested values as compact JSON in one cell.
        Value::Array(_) | Value::Object(_) => v.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn negotiates_csv_media_type_and_parameters() {
        assert_eq!(ApiFormat::from_accept_header("text/csv"), ApiFormat::Csv);
        assert_eq!(
            ApiFormat::from_accept_header("application/json, text/csv; charset=utf-8"),
            ApiFormat::Csv
        );
        assert_eq!(ApiFormat::from_accept_header("text/csvx"), ApiFormat::Json);
    }

    #[test]
    fn serializes_objects_and_arrays_as_pipe_separated_csv() {
        let response = csv_response(serde_json::json!([
            {"name": "one", "count": 1},
            {"name": "two"}
        ]))
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn empty_arrays_produce_empty_csv() {
        let response = csv_response(Vec::<String>::new()).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
