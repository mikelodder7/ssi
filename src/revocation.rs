use crate::did_resolve::DIDResolver;
use crate::jsonld::REVOCATION_LIST_2020_V1_CONTEXT;
use crate::one_or_many::OneOrMany;
use crate::vc::{Credential, CredentialStatus, Issuer, VerificationResult, URI};
use async_trait::async_trait;
use bitvec::prelude::Lsb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use core::convert::TryFrom;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

type URL = String;

/// Minimum length of a revocation list bitstring
/// <https://w3c-ccg.github.io/vc-status-rl-2020/#revocation-bitstring-length>
pub const MIN_BITSTRING_LENGTH: usize = 131072;

const EMPTY_RLIST: &str = "H4sIAAAAAAAA_-3AMQEAAADCoPVPbQwfKAAAAAAAAAAAAAAAAAAAAOBthtJUqwBAAAA";

/// Credential Status object for use in a Verifiable Credential.
/// <https://w3c-ccg.github.io/vc-status-rl-2020/#revocationlist2020status>
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RevocationList2020Status {
    /// URL for status information of the verifiable credential - but not the URL of the revocation
    /// list.
    pub id: URI,
    /// Index of this credential's status in the revocation list credential
    pub revocation_list_index: RevocationListIndex,
    /// URL to a [RevocationList2020Credential]
    pub revocation_list_credential: URL,
}

/// Integer identifying a bit position of the revocation status of a verifiable credential in a
/// revocation list, e.g. in a [RevocationList2020].
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(try_from = "String")]
#[serde(into = "String")]
pub struct RevocationListIndex(usize);

/// Verifiable Credential of type RevocationList2020Credential.
/// <https://w3c-ccg.github.io/vc-status-rl-2020/#revocationlist2020credential>
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RevocationList2020Credential {
    pub id: URI,
    pub issuer: Issuer,
    pub credential_subject: RevocationList2020Subject,
    #[serde(flatten)]
    pub more_properties: Value,
}

/// [Credential subject](https://www.w3.org/TR/vc-data-model/#credential-subject) of a [RevocationList2020Credential]
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum RevocationList2020Subject {
    RevocationList2020(RevocationList2020),
}

/// Credential subject of type RevocationList2020, expected to be used in a Verifiable Credential of type [RevocationList2020Credential]
/// <https://w3c-ccg.github.io/vc-status-rl-2020/#revocationlist2020credential>
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct RevocationList2020 {
    pub encoded_list: EncodedList,
    #[serde(flatten)]
    pub more_properties: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct EncodedList(pub String);

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum RevocationSubject {
    RevocationList2020(RevocationList2020),
}

/// A decoded [revocation list][EncodedList].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List(pub Vec<u8>);

impl TryFrom<String> for RevocationListIndex {
    type Error = std::num::ParseIntError;
    fn try_from(string: String) -> Result<Self, Self::Error> {
        Ok(Self(string.parse()?))
    }
}

impl From<RevocationListIndex> for String {
    fn from(idx: RevocationListIndex) -> String {
        idx.0.to_string()
    }
}

#[derive(Error, Debug)]
pub enum SetStatusError {
    #[error("Encode list: {0}")]
    Encode(#[from] EncodeListError),
    #[error("Decode list: {0}")]
    Decode(#[from] DecodeListError),
    #[error("Out of bounds: bitstring index {0} but length is {1}")]
    OutOfBounds(usize, usize),
    #[error("Revocation list bitstring is too large for BitVec: {0}")]
    ListTooLarge(usize),
    #[error("Revocation list bitstring is too small: {0}. Minimum: {1}")]
    ListTooSmall(usize, usize),
}

impl RevocationList2020 {
    /// Set the revocation status for a given index in the list.
    pub fn set_status(&mut self, index: usize, revoked: bool) -> Result<(), SetStatusError> {
        let mut list = List::try_from(&self.encoded_list)?;
        let bitstring_len = list.0.len() * 8;
        let mut bitstring = BitVec::<Lsb0, u8>::try_from_vec(list.0)
            .map_err(|_| SetStatusError::ListTooLarge(bitstring_len))?;
        if bitstring_len < MIN_BITSTRING_LENGTH {
            return Err(SetStatusError::ListTooSmall(
                bitstring_len,
                MIN_BITSTRING_LENGTH,
            ));
        }
        if let Some(mut bitref) = bitstring.get_mut(index) {
            *bitref = revoked;
        } else {
            return Err(SetStatusError::OutOfBounds(index, bitstring_len));
        }
        list.0 = bitstring.into_vec();
        self.encoded_list = EncodedList::try_from(&list)?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum ListIterDecodeError {
    #[error("Unable to reference indexes: {0}")]
    BitSpan(#[from] bitvec::ptr::BitSpanError<u8>),
    #[error("Revocation list bitstring is too small: {0}. Minimum: {1}")]
    ListTooSmall(usize, usize),
}

impl List {
    /// Get an array of indices in the revocation list for credentials that are revoked.
    pub fn iter_revoked_indexes(
        &self,
    ) -> Result<bitvec::slice::IterOnes<Lsb0, u8>, ListIterDecodeError> {
        let bitstring = BitSlice::<Lsb0, u8>::from_slice(&self.0[..])?;
        if bitstring.len() < MIN_BITSTRING_LENGTH {
            return Err(ListIterDecodeError::ListTooSmall(
                bitstring.len(),
                MIN_BITSTRING_LENGTH,
            ));
        }
        Ok(bitstring.iter_ones())
    }
}

#[derive(Error, Debug)]
pub enum DecodeListError {
    #[error("Base64url: {0}")]
    Build(#[from] base64::DecodeError),
    #[error("Decompression: {0}")]
    Decompress(#[from] std::io::Error),
}

#[derive(Error, Debug)]
pub enum EncodeListError {
    #[error("Compression: {0}")]
    Compress(#[from] std::io::Error),
}

impl Default for EncodedList {
    /// Generate a 16KB list of zeros.
    fn default() -> Self {
        Self(EMPTY_RLIST.to_string())
    }
}

impl TryFrom<&EncodedList> for List {
    type Error = DecodeListError;
    fn try_from(encoded_list: &EncodedList) -> Result<Self, Self::Error> {
        let string = &encoded_list.0;
        let bytes = base64::decode_config(string, base64::URL_SAFE)?;
        let mut data = Vec::new();
        use flate2::bufread::GzDecoder;
        use std::io::Read;
        GzDecoder::new(bytes.as_slice()).read_to_end(&mut data)?;
        Ok(Self(data))
        // TODO: streaming decode the revocation list, for less memory use for large bitvecs.
    }
}

impl TryFrom<&List> for EncodedList {
    type Error = EncodeListError;
    fn try_from(list: &List) -> Result<Self, Self::Error> {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;
        let mut e = GzEncoder::new(Vec::new(), Compression::default());
        e.write_all(&list.0)?;
        let bytes = e.finish()?;
        let string = base64::encode_config(bytes, base64::URL_SAFE_NO_PAD);
        Ok(EncodedList(string))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CredentialStatus for RevocationList2020Status {
    /// Validate a credential's revocation status according to [Revocation List 2020](https://w3c-ccg.github.io/vc-status-rl-2020/#validate-algorithm).
    async fn check(
        &self,
        credential: &Credential,
        resolver: &dyn DIDResolver,
    ) -> VerificationResult {
        let mut result = VerificationResult::new();
        // TODO: prefix errors or change return type
        let issuer_id = match &credential.issuer {
            Some(issuer) => issuer.get_id().clone(),
            None => {
                return result.with_error("Credential is missing issuer".to_string());
            }
        };
        if !credential
            .context
            .contains_uri(REVOCATION_LIST_2020_V1_CONTEXT)
        {
            // TODO: support JSON-LD credentials defining the terms elsewhere.
            return result.with_error(format!(
                "Missing expected context URI {} for RevocationList2020",
                REVOCATION_LIST_2020_V1_CONTEXT
            ));
        }
        if self.id == URI::String(self.revocation_list_credential.clone()) {
            return result.with_error(format!(
                "Expected revocationListCredential to be different from status id: {}",
                self.id
            ));
        }
        let revocation_list_credential =
            match load_credential(&self.revocation_list_credential).await {
                Ok(credential) => credential,
                Err(e) => {
                    return result.with_error(format!(
                        "Unable to fetch revocation list credential: {}",
                        e.to_string()
                    ));
                }
            };
        let list_issuer_id = match &revocation_list_credential.issuer {
            Some(issuer) => issuer.get_id().clone(),
            None => {
                return result.with_error(format!("Revocation list credential is missing issuer"));
            }
        };
        if issuer_id != list_issuer_id {
            return result.with_error(format!(
                "Revocation list issuer mismatch. Credential: {}, Revocation list: {}",
                issuer_id, list_issuer_id
            ));
        }

        match revocation_list_credential.validate() {
            Err(e) => {
                return result.with_error(format!("Invalid list credential: {}", e.to_string()));
            }
            Ok(()) => {}
        }
        let vc_result = revocation_list_credential.verify(None, resolver).await;
        for warning in vc_result.warnings {
            result
                .warnings
                .push(format!("Revocation list: {}", warning));
        }
        for error in vc_result.errors {
            result.errors.push(format!("Revocation list: {}", error));
            return result;
        }
        // Note: vc_result.checks is not checked here. It is assumed that default checks passed.

        let revocation_list_credential =
            match RevocationList2020Credential::try_from(revocation_list_credential) {
                Ok(credential) => credential,
                Err(e) => {
                    return result.with_error(format!(
                        "Unable to parse revocation list credential: {}",
                        e.to_string()
                    ));
                }
            };
        if revocation_list_credential.id != URI::String(self.revocation_list_credential.to_string())
        {
            return result.with_error(format!(
                "Revocation list credential id mismatch. revocationListCredential: {}, id: {}",
                self.revocation_list_credential, revocation_list_credential.id
            ));
        }
        let RevocationList2020Subject::RevocationList2020(revocation_list) =
            revocation_list_credential.credential_subject;

        let list = match List::try_from(&revocation_list.encoded_list) {
            Ok(list) => list,
            Err(e) => {
                return result.with_error(format!(
                    "Unable to decode revocation list: {}",
                    e.to_string()
                ))
            }
        };
        let credential_index = self.revocation_list_index.0;
        use bitvec::prelude::*;
        let bitstring = match BitVec::<Lsb0, u8>::try_from_vec(list.0) {
            Ok(bitstring) => bitstring,
            Err(list) => {
                return result.with_error(format!(
                    "Revocation list is too large for bitvec: {}",
                    list.len()
                ))
            }
        };
        let revoked = match bitstring.get(credential_index) {
            Some(bitref) => *bitref,
            None => {
                return result
                    .with_error("Credential index in revocation list is invalid.".to_string());
            }
        };
        if revoked {
            return result.with_error("Credential is revoked.".to_string());
        }
        result
    }
}

#[derive(Error, Debug)]
pub enum LoadResourceError {
    #[error("Error building HTTP client: {0}")]
    Build(reqwest::Error),
    #[error("Error sending HTTP request: {0}")]
    Request(reqwest::Error),
    #[error("Parse error: {0}")]
    Response(String),
    #[error("Not found")]
    NotFound,
    #[error("HTTP error: {0}")]
    HTTP(String),
}

#[derive(Error, Debug)]
pub enum LoadCredentialError {
    #[error("Unable to load resource: {0}")]
    Load(#[from] LoadResourceError),
    #[error("Error reading HTTP response: {0}")]
    Parse(#[from] serde_json::Error),
}

async fn load_resource(url: &str) -> Result<Vec<u8>, LoadCredentialError> {
    #[cfg(test)]
    match url {
        crate::vc::tests::EXAMPLE_REVOCATION_2020_LIST_URL => {
            return Ok(crate::vc::tests::EXAMPLE_REVOCATION_2020_LIST.to_vec());
        }
        _ => {}
    }
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "User-Agent",
        reqwest::header::HeaderValue::from_static(crate::USER_AGENT),
    );
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| LoadResourceError::Build(e))?;
    let accept = "application/json".to_string();
    let resp = client
        .get(url)
        .header("Accept", accept)
        .send()
        .await
        .map_err(|e| LoadResourceError::Request(e))?;
    if let Err(err) = resp.error_for_status_ref() {
        if err.status() == Some(reqwest::StatusCode::NOT_FOUND) {
            Err(LoadResourceError::NotFound)?;
        }
        Err(LoadResourceError::HTTP(err.to_string()))?;
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| LoadResourceError::Response(e.to_string()))?
        .to_vec();
    Ok(bytes)
}

/// Fetch a credential from a HTTP(S) URL.
/// The resulting verifiable credential is not yet validated or verified.
pub async fn load_credential(url: &str) -> Result<Credential, LoadCredentialError> {
    let data = load_resource(url).await?;
    // TODO: support JWT-VC
    let credential: Credential = serde_json::from_slice(&data)?;
    Ok(credential)
}

#[derive(Error, Debug)]
pub enum CredentialConversionError {
    #[error("Conversion to JSON: {0}")]
    ToValue(serde_json::Error),
    #[error("Conversion from JSON: {0}")]
    FromValue(serde_json::Error),
    #[error("Missing expected URI in @context: {0}")]
    MissingContext(&'static str),
    #[error("Missing expected type: {0}. Found: {0:?}")]
    MissingType(&'static str, OneOrMany<String>),
    #[error("Missing issuer")]
    MissingIssuer,
}

/// Convert Credential to a [RevocationList2020Credential], while validating it.
// https://w3c-ccg.github.io/vc-status-rl-2020/#validate-algorithm
impl TryFrom<Credential> for RevocationList2020Credential {
    type Error = CredentialConversionError;
    fn try_from(credential: Credential) -> Result<Self, Self::Error> {
        if !credential
            .context
            .contains_uri(REVOCATION_LIST_2020_V1_CONTEXT)
        {
            return Err(CredentialConversionError::MissingContext(
                REVOCATION_LIST_2020_V1_CONTEXT,
            ));
        }
        if !credential
            .type_
            .contains(&"RevocationList2020Credential".to_string())
        {
            return Err(CredentialConversionError::MissingType(
                "RevocationList2020Credential",
                credential.type_,
            ));
        }
        let credential =
            serde_json::to_value(credential).map_err(|e| CredentialConversionError::ToValue(e))?;
        let credential = serde_json::from_value(credential)
            .map_err(|e| CredentialConversionError::FromValue(e))?;
        Ok(credential)
    }
}

impl TryFrom<RevocationList2020Credential> for Credential {
    type Error = CredentialConversionError;
    fn try_from(credential: RevocationList2020Credential) -> Result<Self, Self::Error> {
        let mut credential =
            serde_json::to_value(credential).map_err(|e| CredentialConversionError::ToValue(e))?;
        use crate::vc::DEFAULT_CONTEXT;
        use serde_json::json;
        credential["@context"] = json!([DEFAULT_CONTEXT, REVOCATION_LIST_2020_V1_CONTEXT]);
        credential["type"] = json!(["VerifiableCredential", "RevocationList2020Credential"]);
        let credential = serde_json::from_value(credential)
            .map_err(|e| CredentialConversionError::FromValue(e))?;
        Ok(credential)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_list() {
        let list = List(vec![0; MIN_BITSTRING_LENGTH / 8]);
        let revoked_indexes = list.iter_revoked_indexes().unwrap().collect::<Vec<usize>>();
        let empty: Vec<usize> = Vec::new();
        assert_eq!(revoked_indexes, empty);
        let el = EncodedList::try_from(&list).unwrap();
        assert_eq!(EncodedList::default(), el);
        let decoded_list = List::try_from(&el).unwrap();
        assert_eq!(decoded_list, list);
    }

    #[test]
    fn set_status() {
        let mut rl = RevocationList2020::default();
        rl.set_status(1, true).unwrap();
        rl.set_status(5, true).unwrap();
        let decoded_list = List::try_from(&rl.encoded_list).unwrap();
        let revoked_indexes = decoded_list
            .iter_revoked_indexes()
            .unwrap()
            .collect::<Vec<usize>>();
        assert_eq!(revoked_indexes, vec![1, 5]);
    }
}
