use std::collections::HashSet;

use crate::heartbeat::AgentId;
use crate::types::AssignmentId;

use super::hash::sha256_digest;
use super::profile::{MediaProfileSpec, find_media_profile};
use super::types::{
    ARTIFACT_PROTOCOL_V1, ArtifactError, ArtifactFile, ArtifactKind, ArtifactManifest,
    ArtifactProperty, HashDigest, PropertyType,
};

pub fn compute_manifest_hash(manifest: &ArtifactManifest) -> Result<HashDigest, ArtifactError> {
    validate_manifest_shape(manifest)?;
    sha256_digest(canonical_manifest_payload(manifest).as_bytes())
}

pub fn seal_manifest(mut manifest: ArtifactManifest) -> Result<ArtifactManifest, ArtifactError> {
    let manifest_hash = compute_manifest_hash(&manifest)?;
    manifest.manifest_hash = Some(manifest_hash);
    Ok(manifest)
}

pub fn validate_manifest(manifest: &ArtifactManifest) -> Result<HashDigest, ArtifactError> {
    let expected = compute_manifest_hash(manifest)?;
    let actual = manifest
        .manifest_hash
        .clone()
        .ok_or(ArtifactError::MissingManifestHash)?;
    if actual != expected {
        return Err(ArtifactError::ManifestHashMismatch { expected, actual });
    }

    Ok(expected)
}

pub fn validate_manifest_submission(
    manifest: &ArtifactManifest,
    assignment_id: &AssignmentId,
    producer_agent_id: &AgentId,
) -> Result<HashDigest, ArtifactError> {
    if manifest.assignment_id != *assignment_id {
        return Err(ArtifactError::AssignmentMismatch {
            expected: assignment_id.clone(),
            actual: manifest.assignment_id.clone(),
        });
    }
    if manifest.producer_agent_id != *producer_agent_id {
        return Err(ArtifactError::ProducerMismatch {
            expected: producer_agent_id.clone(),
            actual: manifest.producer_agent_id.clone(),
        });
    }

    validate_manifest(manifest)
}

pub fn canonical_manifest_payload(manifest: &ArtifactManifest) -> String {
    let mut output = String::new();
    push_pair(&mut output, "protocol", &manifest.protocol);
    push_pair(&mut output, "artifact_id", manifest.artifact_id.as_str());
    push_pair(&mut output, "task_id", manifest.task_id.as_str());
    push_pair(
        &mut output,
        "assignment_id",
        manifest.assignment_id.as_str(),
    );
    push_pair(
        &mut output,
        "producer_agent_id",
        manifest.producer_agent_id.as_str(),
    );
    push_pair(
        &mut output,
        "kind",
        match manifest.kind {
            ArtifactKind::Single => "single",
            ArtifactKind::Bundle => "bundle",
        },
    );
    push_pair(
        &mut output,
        "created_at",
        &manifest.created_at.0.to_string(),
    );

    let mut files = manifest.files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| {
        left.path
            .as_deref()
            .unwrap_or("")
            .cmp(right.path.as_deref().unwrap_or(""))
            .then_with(|| left.uri.cmp(&right.uri))
            .then_with(|| left.media_profile.cmp(&right.media_profile))
    });
    for (index, file) in files.into_iter().enumerate() {
        push_file(&mut output, index, file);
    }

    output
}

fn validate_manifest_shape(manifest: &ArtifactManifest) -> Result<(), ArtifactError> {
    if manifest.protocol != ARTIFACT_PROTOCOL_V1 {
        return Err(ArtifactError::UnsupportedProtocol(
            manifest.protocol.clone(),
        ));
    }
    if manifest.files.is_empty() {
        return Err(ArtifactError::EmptyFiles);
    }
    if manifest.kind == ArtifactKind::Single && manifest.files.len() != 1 {
        return Err(ArtifactError::SingleManifestMustHaveOneFile);
    }

    let mut paths = HashSet::new();
    for (index, file) in manifest.files.iter().enumerate() {
        if file.uri.trim().is_empty() {
            return Err(ArtifactError::EmptyUri { index });
        }
        if file.size_bytes == 0 {
            return Err(ArtifactError::ZeroSize { index });
        }
        if manifest.kind == ArtifactKind::Bundle {
            let path = file
                .path
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .ok_or(ArtifactError::BundleFileMissingPath { index })?;
            if !paths.insert(path.clone()) {
                return Err(ArtifactError::DuplicateBundlePath(path.clone()));
            }
        }

        let profile = find_media_profile(&file.media_profile).ok_or_else(|| {
            ArtifactError::UnsupportedMediaProfile {
                index,
                profile: file.media_profile.clone(),
            }
        })?;
        validate_file_against_profile(index, file, profile)?;
    }

    Ok(())
}

fn validate_file_against_profile(
    index: usize,
    file: &ArtifactFile,
    profile: &MediaProfileSpec,
) -> Result<(), ArtifactError> {
    if file.media_type != profile.media_type {
        return Err(ArtifactError::MediaTypeMismatch {
            index,
            profile: file.media_profile.clone(),
            expected: profile.media_type.to_string(),
            actual: file.media_type.clone(),
        });
    }
    if profile.requires_schema {
        let schema = file
            .schema
            .as_ref()
            .ok_or_else(|| ArtifactError::MissingSchema {
                index,
                media_profile: file.media_profile.clone(),
            })?;
        if schema.name.trim().is_empty() || schema.version.trim().is_empty() {
            return Err(ArtifactError::InvalidSchema { index });
        }
    }

    for requirement in profile.required_properties {
        let property = file.properties.get(requirement.name).ok_or_else(|| {
            ArtifactError::MissingProperty {
                index,
                profile: file.media_profile.clone(),
                property: requirement.name.to_string(),
            }
        })?;
        if property_type(property) != requirement.kind {
            return Err(ArtifactError::PropertyTypeMismatch {
                index,
                property: requirement.name.to_string(),
                expected: requirement.kind,
            });
        }
        validate_property_value(index, requirement.name, property, requirement.expected_text)?;
    }

    Ok(())
}

fn validate_property_value(
    index: usize,
    name: &str,
    property: &ArtifactProperty,
    expected_text: Option<&str>,
) -> Result<(), ArtifactError> {
    match property {
        ArtifactProperty::Integer(value) if *value == 0 => {
            Err(ArtifactError::InvalidPropertyValue {
                index,
                property: name.to_string(),
                expected: "> 0".to_string(),
                actual: value.to_string(),
            })
        }
        ArtifactProperty::Text(value) => {
            let Some(expected) = expected_text else {
                return Ok(());
            };
            if value != expected {
                return Err(ArtifactError::InvalidPropertyValue {
                    index,
                    property: name.to_string(),
                    expected: expected.to_string(),
                    actual: value.clone(),
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn property_type(property: &ArtifactProperty) -> PropertyType {
    match property {
        ArtifactProperty::Bool(_) => PropertyType::Bool,
        ArtifactProperty::Integer(_) => PropertyType::Integer,
        ArtifactProperty::Text(_) => PropertyType::Text,
    }
}

fn push_file(output: &mut String, index: usize, file: &ArtifactFile) {
    let prefix = format!("file[{index}].");
    push_pair(
        output,
        &(prefix.clone() + "path"),
        file.path.as_deref().unwrap_or(""),
    );
    push_pair(output, &(prefix.clone() + "uri"), &file.uri);
    push_pair(
        output,
        &(prefix.clone() + "content_hash"),
        file.content_hash.as_str(),
    );
    push_pair(output, &(prefix.clone() + "media_type"), &file.media_type);
    push_pair(
        output,
        &(prefix.clone() + "media_profile"),
        file.media_profile.as_str(),
    );
    push_pair(
        output,
        &(prefix.clone() + "size_bytes"),
        &file.size_bytes.to_string(),
    );
    if let Some(schema) = &file.schema {
        push_pair(output, &(prefix.clone() + "schema.name"), &schema.name);
        push_pair(
            output,
            &(prefix.clone() + "schema.version"),
            &schema.version,
        );
        push_pair(
            output,
            &(prefix.clone() + "schema.hash"),
            schema.hash.as_str(),
        );
    }
    for (name, value) in &file.properties {
        let key = prefix.clone() + "property." + name;
        match value {
            ArtifactProperty::Bool(value) => {
                push_pair(output, &key, if *value { "true" } else { "false" });
            }
            ArtifactProperty::Integer(value) => {
                push_pair(output, &key, &value.to_string());
            }
            ArtifactProperty::Text(value) => {
                push_pair(output, &key, value);
            }
        }
    }
}

fn push_pair(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push('=');
    output.push_str(&escape(value));
    output.push('\n');
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '=' => escaped.push_str("\\="),
            other => escaped.push(other),
        }
    }
    escaped
}
