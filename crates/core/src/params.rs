//! Decoding an op's params (ADR 0014). One rule for every module and the daemon, so a module never
//! keeps its own copy.

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::{Error, Result};

/// Decode an op's `params` into `T`. Absent params (`null` on the wire) read as `{}`, so an op whose
/// params are all optional can be called with none; anything that doesn't fit `T` is
/// `invalid_params`, with serde's message.
pub fn decode<T: DeserializeOwned>(params: Value) -> Result<T> {
    let params = if params.is_null() { json!({}) } else { params };
    serde_json::from_value(params).map_err(|e| Error::invalid_params(e.to_string()))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;
    use crate::ErrorCode;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Optional {
        #[serde(default)]
        limit: Option<u32>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Required {
        id: String,
    }

    #[test]
    fn absent_params_read_as_an_empty_object() {
        assert_eq!(decode::<Optional>(Value::Null).unwrap(), Optional { limit: None });
        assert_eq!(decode::<Optional>(json!({})).unwrap(), Optional { limit: None });
    }

    #[test]
    fn params_decode_into_the_type() {
        assert_eq!(decode::<Optional>(json!({"limit": 5})).unwrap(), Optional { limit: Some(5) });
        assert_eq!(decode::<Required>(json!({"id": "two-sum"})).unwrap(), Required { id: "two-sum".into() });
    }

    #[test]
    fn anything_that_does_not_fit_is_invalid_params_with_serdes_message() {
        for (params, part) in [
            (Value::Null, "missing field `id`"),
            (json!({"id": 7}), "invalid type: integer `7`, expected a string"),
            (json!("two-sum"), "invalid type: string \"two-sum\", expected struct Required"),
        ] {
            let e = decode::<Required>(params.clone()).unwrap_err();
            assert_eq!(e.code, ErrorCode::InvalidParams, "{params}");
            assert!(e.message.contains(part), "{params}: {}", e.message);
        }
    }
}
