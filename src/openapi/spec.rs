use serde_json::{json, Map, Value};

use crate::app::{AppMeta, ManifestEntry};
use super::meta::OperationMeta;

/// A manifest entry with optional operation metadata, used for OpenAPI generation.
pub(crate) struct SpecRoute {
    pub entry: ManifestEntry,
    pub meta: Option<OperationMeta>,
    pub tags: Vec<String>,
}

/// Generate an OpenAPI 3.1.0 JSON document from app metadata and routes.
pub(crate) fn generate_spec(
    meta: &Option<AppMeta>,
    routes: &[SpecRoute],
) -> Value {
    let info = match meta {
        Some(m) => {
            let mut info = json!({
                "title": m.title,
                "version": m.version,
            });
            if let Some(desc) = &m.description {
                info["description"] = json!(desc);
            }
            info
        }
        None => json!({
            "title": "API",
            "version": "0.0.0",
        }),
    };

    let mut paths: Map<String, Value> = Map::new();

    for route in routes {
        let method = route.entry.method.as_str().to_lowercase();
        let path = &route.entry.path;

        let operation = build_operation(route);

        let path_item = paths
            .entry(path.clone())
            .or_insert_with(|| json!({}));
        path_item[method] = operation;
    }

    json!({
        "openapi": "3.1.0",
        "info": info,
        "paths": paths,
    })
}

fn build_operation(route: &SpecRoute) -> Value {
    let mut op: Map<String, Value> = Map::new();

    // Merge route-level tags + meta tags
    let mut all_tags: Vec<String> = route.tags.clone();
    if let Some(meta) = &route.meta {
        all_tags.extend(meta.tags.iter().cloned());

        if let Some(s) = &meta.summary {
            op.insert("summary".into(), json!(s));
        }
        if let Some(d) = &meta.description {
            op.insert("description".into(), json!(d));
        }
        if let Some(id) = &meta.operation_id {
            op.insert("operationId".into(), json!(id));
        }
        if meta.deprecated {
            op.insert("deprecated".into(), json!(true));
        }

        // Parameters
        if !meta.parameters.is_empty() {
            let params: Vec<Value> = meta
                .parameters
                .iter()
                .map(|p| {
                    let mut param = json!({
                        "name": p.name,
                        "in": p.location,
                        "required": p.required,
                    });
                    if let Some(d) = &p.description {
                        param["description"] = json!(d);
                    }
                    if let Some(s) = &p.schema {
                        param["schema"] = serde_json::to_value(s).unwrap_or_default();
                    }
                    param
                })
                .collect();
            op.insert("parameters".into(), json!(params));
        }

        // Request body
        if let Some(body) = &meta.request_body {
            op.insert(
                "requestBody".into(),
                json!({
                    "required": body.required,
                    "content": {
                        &body.content_type: {
                            "schema": serde_json::to_value(&body.schema).unwrap_or_default()
                        }
                    }
                }),
            );
        }

        // Responses
        if !meta.responses.is_empty() {
            let mut responses: Map<String, Value> = Map::new();
            for r in &meta.responses {
                let mut resp = json!({ "description": r.description });
                if let Some(s) = &r.schema {
                    resp["content"] = json!({
                        "application/json": {
                            "schema": serde_json::to_value(s).unwrap_or_default()
                        }
                    });
                }
                responses.insert(r.status.to_string(), resp);
            }
            op.insert("responses".into(), json!(responses));
        }
    }

    // Default responses if none specified
    if !op.contains_key("responses") {
        op.insert(
            "responses".into(),
            json!({ "200": { "description": "Successful response" } }),
        );
    }

    if !all_tags.is_empty() {
        // Deduplicate tags
        all_tags.sort();
        all_tags.dedup();
        op.insert("tags".into(), json!(all_tags));
    }

    json!(op)
}
