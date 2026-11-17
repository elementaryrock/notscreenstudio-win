//! Small serde helpers where Swift's `Codable` output differs from serde's default.
//!
//! The only case so far: Swift's `UUID` encodes as an **uppercase** string, while the
//! `uuid` crate writes lowercase. Project files round-trip either way, but matching
//! upstream byte for byte makes the committed Mac fixtures diff-clean.

pub mod uppercase_uuid {
    use serde::{de::Error as _, Deserialize, Deserializer, Serializer};
    use uuid::Uuid;

    pub fn serialize<S: Serializer>(v: &Uuid, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string().to_uppercase())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Uuid, D::Error> {
        let s = String::deserialize(d)?;
        Uuid::parse_str(&s).map_err(|e| D::Error::custom(format!("invalid uuid {s:?}: {e}")))
    }

    /// Same, for a field that may be absent (`#[serde(default, with = "...::option")]`).
    pub mod option {
        use serde::{Deserialize, Deserializer, Serializer};
        use uuid::Uuid;

        pub fn serialize<S: Serializer>(v: &Option<Uuid>, s: S) -> Result<S::Ok, S::Error> {
            match v {
                Some(u) => s.serialize_str(&u.to_string().to_uppercase()),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Uuid>, D::Error> {
            let s = Option::<String>::deserialize(d)?;
            match s {
                None => Ok(None),
                Some(s) => Uuid::parse_str(&s)
                    .map(Some)
                    .map_err(|e| serde::de::Error::custom(format!("invalid uuid {s:?}: {e}"))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Holder {
        #[serde(with = "super::uppercase_uuid")]
        id: Uuid,
    }

    #[test]
    fn uuid_serialises_uppercase_like_swift() {
        // The exact string upstream's ZoomBlockModelTests uses.
        let id = Uuid::parse_str("7f0c3a9e-2b7e-4c55-9d1d-0d4b7b8e2f11").unwrap();
        let json = serde_json::to_string(&Holder { id }).unwrap();
        assert_eq!(json, r#"{"id":"7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11"}"#);
    }

    #[test]
    fn uuid_reads_lowercase_and_uppercase() {
        for s in [
            r#"{"id":"7F0C3A9E-2B7E-4C55-9D1D-0D4B7B8E2F11"}"#,
            r#"{"id":"7f0c3a9e-2b7e-4c55-9d1d-0d4b7b8e2f11"}"#,
        ] {
            let h: Holder = serde_json::from_str(s).unwrap();
            assert_eq!(h.id.to_string(), "7f0c3a9e-2b7e-4c55-9d1d-0d4b7b8e2f11");
        }
    }
}
