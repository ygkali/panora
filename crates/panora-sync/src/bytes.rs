// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Serde adapters: binary fields as unpadded URL-safe base64 strings.

/// Fixed-size arrays (`[u8; N]`); a wrong length is a decoding error.
pub(crate) mod b64 {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(text)
            .map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("binary field has the wrong length"))
    }
}

/// Variable-length byte strings.
pub(crate) mod b64_vec {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        URL_SAFE_NO_PAD
            .decode(text)
            .map_err(serde::de::Error::custom)
    }
}

/// Fixed-size secrets (the group key): like [`b64`], but the base64 text
/// and the decoded bytes are wiped when dropped.
pub(crate) mod b64_secret {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let text = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes));
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let text = Zeroizing::new(String::deserialize(deserializer)?);
        let bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(text.as_bytes())
                .map_err(serde::de::Error::custom)?,
        );
        let mut out = [0u8; N];
        if bytes.len() != N {
            return Err(serde::de::Error::custom(
                "binary field has the wrong length",
            ));
        }
        out.copy_from_slice(&bytes);
        Ok(out)
    }
}

/// Variable-length secrets (the identity key's PKCS#8 document).
pub(crate) mod b64_secret_vec {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S: Serializer>(
        bytes: &Zeroizing<Vec<u8>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let text = Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_slice()));
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Zeroizing<Vec<u8>>, D::Error> {
        let text = Zeroizing::new(String::deserialize(deserializer)?);
        Ok(Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(text.as_bytes())
                .map_err(serde::de::Error::custom)?,
        ))
    }
}

/// Append a length-prefixed field to a canonical byte encoding (what gets
/// hashed and signed), so no two different field lists encode the same.
pub(crate) fn put(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&(field.len() as u64).to_be_bytes());
    out.extend_from_slice(field);
}
