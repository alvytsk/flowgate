use http::request::Parts;
use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;

use crate::body::Request;
use crate::context::RequestContext;
use crate::error::PathRejection;
use crate::extract::{FromRequest, FromRequestParts};

/// Extractor that deserializes path parameters into `T`.
///
/// Supports single values, tuples (positional), and structs (by name).
///
/// ```ignore
/// // Single: /users/{id}
/// async fn get_user(Path(id): Path<u64>) -> String {
///     format!("user {id}")
/// }
///
/// // Tuple: /users/{uid}/posts/{pid}
/// async fn get_post(Path((uid, pid)): Path<(u64, u64)>) -> String {
///     format!("user {uid} post {pid}")
/// }
/// ```
#[derive(Debug, Clone)]
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: for<'de> Deserialize<'de>,
    S: Send + Sync,
{
    type Rejection = PathRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ctx = parts
            .extensions
            .get::<RequestContext>()
            .ok_or(PathRejection::MissingRouteParams)?;

        let value = T::deserialize(PathDeserializer {
            params: &ctx.route_params.0,
        })
        .map_err(|e| PathRejection::DeserializeError(e.0))?;

        Ok(Path(value))
    }
}

/// Also implement `FromRequest` so `Path<T>` can appear in the last
/// handler argument position (which uses `FromRequest`).
impl<T, S> FromRequest<S> for Path<T>
where
    T: for<'de> Deserialize<'de>,
    S: Send + Sync,
{
    type Rejection = PathRejection;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let (mut parts, _body) = req.into_parts();
        Self::from_request_parts(&mut parts, state).await
    }
}

// --- Serde deserializer for route params ---

#[derive(Debug)]
struct PathError(String);

impl std::fmt::Display for PathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PathError {}

impl de::Error for PathError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        PathError(msg.to_string())
    }
}

// -- Top-level deserializer: dispatches by how T wants to be deserialized --

struct PathDeserializer<'de> {
    params: &'de [(String, String)],
}

impl<'de> PathDeserializer<'de> {
    fn single_value(&self) -> Result<ValueDeserializer<'de>, PathError> {
        self.params
            .first()
            .map(|(_, v)| ValueDeserializer(v.as_str()))
            .ok_or_else(|| PathError("no path parameters".into()))
    }
}

macro_rules! forward_to_single {
    ($($method:ident),*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
                self.single_value()?.$method(visitor)
            }
        )*
    };
}

impl<'de> Deserializer<'de> for PathDeserializer<'de> {
    type Error = PathError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        if self.params.len() == 1 {
            visitor.visit_str(&self.params[0].1)
        } else {
            self.deserialize_map(visitor)
        }
    }

    forward_to_single! {
        deserialize_bool,
        deserialize_i8, deserialize_i16, deserialize_i32, deserialize_i64,
        deserialize_u8, deserialize_u16, deserialize_u32, deserialize_u64,
        deserialize_f32, deserialize_f64,
        deserialize_char, deserialize_str, deserialize_string
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_seq(ParamsSeq {
            params: self.params,
            index: 0,
        })
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_map(ParamsMap {
            params: self.params,
            index: 0,
        })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.single_value()?
            .deserialize_enum(name, variants, visitor)
    }

    serde::forward_to_deserialize_any! {
        bytes byte_buf unit unit_struct identifier ignored_any
    }
}

// -- Value deserializer: parses a single &str into the target type --

struct ValueDeserializer<'de>(&'de str);

macro_rules! parse_value {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
            let v: $ty = self.0.parse().map_err(de::Error::custom)?;
            visitor.$visit(v)
        }
    };
}

impl<'de> Deserializer<'de> for ValueDeserializer<'de> {
    type Error = PathError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_str(self.0)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_str(self.0)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_string(self.0.to_owned())
    }

    parse_value!(deserialize_bool, visit_bool, bool);
    parse_value!(deserialize_i8, visit_i8, i8);
    parse_value!(deserialize_i16, visit_i16, i16);
    parse_value!(deserialize_i32, visit_i32, i32);
    parse_value!(deserialize_i64, visit_i64, i64);
    parse_value!(deserialize_u8, visit_u8, u8);
    parse_value!(deserialize_u16, visit_u16, u16);
    parse_value!(deserialize_u32, visit_u32, u32);
    parse_value!(deserialize_u64, visit_u64, u64);
    parse_value!(deserialize_f32, visit_f32, f32);
    parse_value!(deserialize_f64, visit_f64, f64);
    parse_value!(deserialize_char, visit_char, char);

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        visitor.visit_enum(de::value::StrDeserializer::<PathError>::new(self.0))
    }

    serde::forward_to_deserialize_any! {
        bytes byte_buf unit unit_struct seq tuple tuple_struct
        map struct identifier ignored_any
    }
}

// -- Sequence access for tuples --

struct ParamsSeq<'de> {
    params: &'de [(String, String)],
    index: usize,
}

impl<'de> SeqAccess<'de> for ParamsSeq<'de> {
    type Error = PathError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Self::Error> {
        if self.index >= self.params.len() {
            return Ok(None);
        }
        let value = &self.params[self.index].1;
        self.index += 1;
        seed.deserialize(ValueDeserializer(value)).map(Some)
    }
}

// -- Map access for structs --

struct ParamsMap<'de> {
    params: &'de [(String, String)],
    index: usize,
}

impl<'de> MapAccess<'de> for ParamsMap<'de> {
    type Error = PathError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        if self.index >= self.params.len() {
            return Ok(None);
        }
        seed.deserialize(ValueDeserializer(&self.params[self.index].0))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = &self.params[self.index].1;
        self.index += 1;
        seed.deserialize(ValueDeserializer(value))
    }
}
