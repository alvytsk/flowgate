use serde::{Deserialize, Serialize};

/// Operation metadata attached to a single route.
///
/// Build with the builder pattern and attach via `_with` route registration
/// variants (e.g. `app.get_with("/users", handler, meta)`).
///
/// # Example
///
/// ```ignore
/// OperationMeta::new()
///     .summary("List users")
///     .description("Returns all users in the system")
///     .tag("users")
///     .response(200, "Successful response")
/// ```
#[derive(Clone, Debug, Default, Serialize)]
pub struct OperationMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParamMeta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_body: Option<BodyMeta>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub responses: Vec<ResponseMeta>,
}

impl OperationMeta {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn summary(mut self, s: impl Into<String>) -> Self {
        self.summary = Some(s.into());
        self
    }

    pub fn description(mut self, s: impl Into<String>) -> Self {
        self.description = Some(s.into());
        self
    }

    pub fn operation_id(mut self, id: impl Into<String>) -> Self {
        self.operation_id = Some(id.into());
        self
    }

    pub fn tag(mut self, t: impl Into<String>) -> Self {
        self.tags.push(t.into());
        self
    }

    pub fn deprecated(mut self) -> Self {
        self.deprecated = true;
        self
    }

    pub fn param(mut self, p: ParamMeta) -> Self {
        self.parameters.push(p);
        self
    }

    pub fn request_body(mut self, b: BodyMeta) -> Self {
        self.request_body = Some(b);
        self
    }

    pub fn response(mut self, status: u16, description: impl Into<String>) -> Self {
        self.responses.push(ResponseMeta {
            status,
            description: description.into(),
            schema: None,
        });
        self
    }

    pub fn response_with_schema(
        mut self,
        status: u16,
        description: impl Into<String>,
        schema: SchemaObject,
    ) -> Self {
        self.responses.push(ResponseMeta {
            status,
            description: description.into(),
            schema: Some(schema),
        });
        self
    }
}

/// Parameter metadata for a path, query, or header parameter.
#[derive(Clone, Debug, Serialize)]
pub struct ParamMeta {
    pub name: String,
    #[serde(rename = "in")]
    pub location: ParamLocation,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaObject>,
}

impl ParamMeta {
    pub fn path(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            location: ParamLocation::Path,
            required: true,
            description: None,
            schema: None,
        }
    }

    pub fn query(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            location: ParamLocation::Query,
            required: false,
            description: None,
            schema: None,
        }
    }

    pub fn header(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            location: ParamLocation::Header,
            required: false,
            description: None,
            schema: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn schema(mut self, s: SchemaObject) -> Self {
        self.schema = Some(s);
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParamLocation {
    Path,
    Query,
    Header,
}

/// Request body metadata.
#[derive(Clone, Debug, Serialize)]
pub struct BodyMeta {
    pub content_type: String,
    pub schema: SchemaObject,
    pub required: bool,
}

impl BodyMeta {
    pub fn json(schema: SchemaObject) -> Self {
        Self {
            content_type: "application/json".to_owned(),
            schema,
            required: true,
        }
    }
}

/// Response metadata for a single status code.
#[derive(Clone, Debug, Serialize)]
pub struct ResponseMeta {
    pub status: u16,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<SchemaObject>,
}

/// Minimal JSON Schema object — enough for OpenAPI generation
/// without pulling in schemars.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SchemaObject {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub schema_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub properties: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<SchemaObject>>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub required: Vec<String>,
}

impl SchemaObject {
    pub fn string() -> Self {
        Self { schema_type: Some("string".into()), ..Default::default() }
    }
    pub fn integer() -> Self {
        Self { schema_type: Some("integer".into()), ..Default::default() }
    }
    pub fn number() -> Self {
        Self { schema_type: Some("number".into()), ..Default::default() }
    }
    pub fn boolean() -> Self {
        Self { schema_type: Some("boolean".into()), ..Default::default() }
    }
    pub fn array(items: SchemaObject) -> Self {
        Self { schema_type: Some("array".into()), items: Some(Box::new(items)), ..Default::default() }
    }
    pub fn object() -> Self {
        Self { schema_type: Some("object".into()), ..Default::default() }
    }
}
