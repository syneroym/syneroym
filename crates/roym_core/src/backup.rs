use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syneroym_signed_record::{EnvelopeError, content_digest};

pub const BUNDLE_VERSION: u32 = 1;
pub const SECTION_PROFILE: &str = "profile";
pub const SECTION_CONTACTS: &str = "contacts";
pub const SECTION_BLOCKS: &str = "blocks";
pub const SECTION_REPORTS: &str = "reports";
/// The digest prefix, so a section digest can never be mistaken for a
/// record id or a report id.
pub const SECTION_DIGEST_PREFIX: &str = "sec_";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionDigest {
    /// The section's own schema version, chosen by whichever service
    /// produced it. No serde default: a section with no version fails to
    /// parse rather than assuming one.
    pub schema_version: u32,
    pub record_count: u64,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub bundle_version: u32,
    pub produced_at_secs: u64,
    /// The person this bundle belongs to. Checked on import against the
    /// identity the importing node holds -- an import that would graft
    /// one person's data onto another's node is refused, not merged.
    pub subject_did: String,
    pub sections: BTreeMap<String, SectionDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub manifest: BundleManifest,
    /// Section name -> the section's documents, in the order the exporter
    /// wrote them. Order is part of the hashed bytes.
    pub sections: BTreeMap<String, Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BundleError {
    #[error("bundle version {0} is not understood by this build")]
    UnknownBundleVersion(u32),
    #[error("bundle json: {0}")]
    Json(String),
    #[error("section '{0}' has a digest but no content")]
    MissingSection(String),
    #[error("section '{0}' has content but no digest")]
    UndeclaredSection(String),
    #[error("section '{section}': {count} records, manifest says {expected}")]
    CountMismatch { section: String, count: u64, expected: u64 },
    #[error("section '{section}': content hash does not match the manifest")]
    DigestMismatch { section: String },
    #[error("bundle belongs to '{subject}', this node holds '{holder}'")]
    WrongSubject { subject: String, holder: String },
}

impl From<EnvelopeError> for BundleError {
    fn from(e: EnvelopeError) -> Self {
        BundleError::Json(e.to_string())
    }
}

impl Bundle {
    /// One hash definition, shared with the envelope's own record id.
    pub fn digest(schema_version: u32, records: &[Value]) -> Result<SectionDigest, BundleError> {
        let digest_str = content_digest(
            SECTION_DIGEST_PREFIX,
            &json!({ "schema_version": schema_version, "records": records }),
        )?;
        Ok(SectionDigest { schema_version, record_count: records.len() as u64, digest: digest_str })
    }

    /// Every check that does not need a node: version, section symmetry,
    /// counts, hashes. `subject_did` is checked by the caller, which is
    /// the only party that knows who this node holds.
    pub fn check_integrity(&self) -> Result<(), BundleError> {
        if self.manifest.bundle_version != BUNDLE_VERSION {
            return Err(BundleError::UnknownBundleVersion(self.manifest.bundle_version));
        }
        for (name, declared) in &self.manifest.sections {
            let records =
                self.sections.get(name).ok_or_else(|| BundleError::MissingSection(name.clone()))?;
            if records.len() as u64 != declared.record_count {
                return Err(BundleError::CountMismatch {
                    section: name.clone(),
                    count: records.len() as u64,
                    expected: declared.record_count,
                });
            }
            let computed = Self::digest(declared.schema_version, records)?;
            if computed.digest != declared.digest {
                return Err(BundleError::DigestMismatch { section: name.clone() });
            }
        }
        for name in self.sections.keys() {
            if !self.manifest.sections.contains_key(name) {
                return Err(BundleError::UndeclaredSection(name.clone()));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, BundleError> {
        serde_json::to_string(self).map_err(|e| BundleError::Json(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self, BundleError> {
        serde_json::from_str(s).map_err(|e| BundleError::Json(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_bundle() -> Bundle {
        let records = vec![json!({"id": "row1", "val": 10})];
        let digest = Bundle::digest(1, &records).unwrap();
        let mut sections_digest = BTreeMap::new();
        sections_digest.insert("profile".to_string(), digest);

        let mut sections_data = BTreeMap::new();
        sections_data.insert("profile".to_string(), records);

        Bundle {
            manifest: BundleManifest {
                bundle_version: BUNDLE_VERSION,
                produced_at_secs: 1000,
                subject_did: "did:key:z6M123".to_string(),
                sections: sections_digest,
            },
            sections: sections_data,
        }
    }

    #[test]
    fn valid_bundle_check_integrity() {
        let b = sample_bundle();
        assert!(b.check_integrity().is_ok());
    }

    #[test]
    fn unknown_bundle_version() {
        let mut b = sample_bundle();
        b.manifest.bundle_version = 2;
        assert_eq!(b.check_integrity(), Err(BundleError::UnknownBundleVersion(2)));
    }

    #[test]
    fn missing_section_content() {
        let mut b = sample_bundle();
        b.sections.clear();
        assert_eq!(b.check_integrity(), Err(BundleError::MissingSection("profile".to_string())));
    }

    #[test]
    fn undeclared_section_content() {
        let mut b = sample_bundle();
        b.sections.insert("contacts".to_string(), vec![]);
        assert_eq!(
            b.check_integrity(),
            Err(BundleError::UndeclaredSection("contacts".to_string()))
        );
    }

    #[test]
    fn count_mismatch() {
        let mut b = sample_bundle();
        b.manifest.sections.get_mut("profile").unwrap().record_count = 5;
        assert_eq!(
            b.check_integrity(),
            Err(BundleError::CountMismatch {
                section: "profile".to_string(),
                count: 1,
                expected: 5,
            })
        );
    }

    #[test]
    fn digest_mismatch_on_record_change() {
        let mut b = sample_bundle();
        b.sections.get_mut("profile").unwrap()[0] = json!({"id": "row1", "val": 999});
        assert_eq!(
            b.check_integrity(),
            Err(BundleError::DigestMismatch { section: "profile".to_string() })
        );
    }

    #[test]
    fn produced_at_secs_does_not_affect_digest() {
        let records = vec![json!({"id": "r1"})];
        let d1 = Bundle::digest(1, &records).unwrap();
        let d2 = Bundle::digest(1, &records).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn schema_version_affects_digest() {
        let records = vec![json!({"id": "r1"})];
        let d1 = Bundle::digest(1, &records).unwrap();
        let d2 = Bundle::digest(2, &records).unwrap();
        assert_ne!(d1.digest, d2.digest);
    }
}
