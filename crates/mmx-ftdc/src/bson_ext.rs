use bson::{Bson, Document};

/// A single flattened metric: dot-separated path and its numeric value as i64.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatMetric {
    pub path: String,
    pub value: i64,
}

/// Flatten a BSON document into a list of `(path, i64)` pairs.
///
/// Performs depth-first traversal, extracting only numeric fields.
/// Nested documents and arrays are recursed into, producing dot-separated paths
/// (e.g., `serverStatus.connections.current`, `locks.0.mode`).
///
/// Handled BSON types (matching MongoDB server's FTDC collector):
/// - `Boolean`: `false` = 0, `true` = 1
/// - `Int32`: directly cast to i64
/// - `Int64`: used directly
/// - `Double`: cast to i64 (truncated)
/// - `Decimal128`: lossy conversion via f64 to i64
/// - `DateTime`: milliseconds since epoch
/// - `Timestamp`: split into two metrics: `.t` (seconds) and `.i` (increment)
/// - `Document`: recurse
/// - `Array`: recurse with positional index keys (0, 1, 2, ...)
pub fn flatten_bson(doc: &Document) -> Vec<FlatMetric> {
    let mut result = Vec::new();
    flatten_recursive(doc, &mut String::new(), &mut result);
    result
}

fn flatten_recursive(doc: &Document, prefix: &mut String, out: &mut Vec<FlatMetric>) {
    for (key, value) in doc {
        let path_start = prefix.len();
        if !prefix.is_empty() {
            prefix.push('.');
        }
        prefix.push_str(key);

        flatten_value(value, prefix, out);

        prefix.truncate(path_start);
    }
}

fn flatten_value(value: &Bson, prefix: &mut String, out: &mut Vec<FlatMetric>) {
    match value {
        Bson::Boolean(b) => {
            out.push(FlatMetric {
                path: prefix.clone(),
                value: *b as i64,
            });
        }
        Bson::Int32(n) => {
            out.push(FlatMetric {
                path: prefix.clone(),
                value: *n as i64,
            });
        }
        Bson::Int64(n) => {
            out.push(FlatMetric {
                path: prefix.clone(),
                value: *n,
            });
        }
        Bson::Double(f) => {
            out.push(FlatMetric {
                path: prefix.clone(),
                value: *f as i64,
            });
        }
        Bson::Decimal128(d) => {
            let s = d.to_string();
            let value = s.parse::<f64>().map(|f| f as i64).unwrap_or(0);
            out.push(FlatMetric {
                path: prefix.clone(),
                value,
            });
        }
        Bson::DateTime(dt) => {
            out.push(FlatMetric {
                path: prefix.clone(),
                value: dt.timestamp_millis(),
            });
        }
        Bson::Timestamp(ts) => {
            out.push(FlatMetric {
                path: format!("{prefix}.t"),
                value: ts.time as i64,
            });
            out.push(FlatMetric {
                path: format!("{prefix}.i"),
                value: ts.increment as i64,
            });
        }
        Bson::Document(nested) => {
            flatten_recursive(nested, prefix, out);
        }
        Bson::Array(arr) => {
            // MongoDB FTDC treats arrays like documents, indexing by position
            for (i, item) in arr.iter().enumerate() {
                let path_start = prefix.len();
                prefix.push('.');
                prefix.push_str(&i.to_string());

                flatten_value(item, prefix, out);

                prefix.truncate(path_start);
            }
        }
        _ => {
            // Skip non-numeric types (String, Binary, ObjectId, etc.)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, Timestamp, doc};

    #[test]
    fn test_simple_int32() {
        let doc = doc! { "a": 42_i32 };
        let flat = flatten_bson(&doc);
        assert_eq!(
            flat,
            vec![FlatMetric {
                path: "a".into(),
                value: 42
            }]
        );
    }

    #[test]
    fn test_simple_int64() {
        let doc = doc! { "big": 1_000_000_000_000_i64 };
        let flat = flatten_bson(&doc);
        assert_eq!(
            flat,
            vec![FlatMetric {
                path: "big".into(),
                value: 1_000_000_000_000
            }]
        );
    }

    #[test]
    fn test_nested_document() {
        let doc = doc! {
            "serverStatus": {
                "connections": {
                    "current": 10_i32,
                    "available": 100_i32,
                }
            }
        };
        let flat = flatten_bson(&doc);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].path, "serverStatus.connections.current");
        assert_eq!(flat[0].value, 10);
        assert_eq!(flat[1].path, "serverStatus.connections.available");
        assert_eq!(flat[1].value, 100);
    }

    #[test]
    fn test_boolean() {
        let doc = doc! { "ok": true, "fail": false };
        let flat = flatten_bson(&doc);
        assert_eq!(
            flat[0],
            FlatMetric {
                path: "ok".into(),
                value: 1
            }
        );
        assert_eq!(
            flat[1],
            FlatMetric {
                path: "fail".into(),
                value: 0
            }
        );
    }

    #[test]
    fn test_double() {
        let doc = doc! { "rate": 3.14_f64 };
        let flat = flatten_bson(&doc);
        assert_eq!(flat[0].value, 3); // truncated
    }

    #[test]
    fn test_timestamp_splits() {
        let doc = doc! {
            "ts": Bson::Timestamp(Timestamp { time: 1700000000, increment: 1 })
        };
        let flat = flatten_bson(&doc);
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].path, "ts.t");
        assert_eq!(flat[0].value, 1700000000);
        assert_eq!(flat[1].path, "ts.i");
        assert_eq!(flat[1].value, 1);
    }

    #[test]
    fn test_skips_strings() {
        let doc = doc! { "name": "test", "count": 5_i32 };
        let flat = flatten_bson(&doc);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].path, "count");
    }

    #[test]
    fn test_mixed_nested() {
        let doc = doc! {
            "a": {
                "x": 1_i32,
                "y": "skip",
                "z": {
                    "deep": 99_i64,
                }
            },
            "b": true,
        };
        let flat = flatten_bson(&doc);
        let paths: Vec<&str> = flat.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["a.x", "a.z.deep", "b"]);
    }

    #[test]
    fn test_ordering_matches_document_order() {
        let doc = doc! {
            "c": 3_i32,
            "a": 1_i32,
            "b": 2_i32,
        };
        let flat = flatten_bson(&doc);
        let paths: Vec<&str> = flat.iter().map(|m| m.path.as_str()).collect();
        assert_eq!(paths, vec!["c", "a", "b"]);
    }

    #[test]
    fn test_datetime() {
        let dt = bson::DateTime::from_millis(1700000000000);
        let doc = doc! { "time": dt };
        let flat = flatten_bson(&doc);
        assert_eq!(flat[0].value, 1700000000000);
    }

    #[test]
    fn test_array_of_ints() {
        let doc = doc! { "arr": [10_i32, 20_i32, 30_i32] };
        let flat = flatten_bson(&doc);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].path, "arr.0");
        assert_eq!(flat[0].value, 10);
        assert_eq!(flat[1].path, "arr.1");
        assert_eq!(flat[1].value, 20);
        assert_eq!(flat[2].path, "arr.2");
        assert_eq!(flat[2].value, 30);
    }

    #[test]
    fn test_array_of_documents() {
        let doc = doc! {
            "locks": [
                { "mode": 1_i32, "count": 100_i64 },
                { "mode": 2_i32, "count": 200_i64 },
            ]
        };
        let flat = flatten_bson(&doc);
        assert_eq!(flat.len(), 4);
        assert_eq!(flat[0].path, "locks.0.mode");
        assert_eq!(flat[1].path, "locks.0.count");
        assert_eq!(flat[2].path, "locks.1.mode");
        assert_eq!(flat[3].path, "locks.1.count");
    }
}
