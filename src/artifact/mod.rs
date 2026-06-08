mod hash;
mod profile;
mod types;
mod validator;

pub use hash::sha256_digest;
pub use profile::{
    BASELINE_MEDIA_PROFILES, MediaProfileSpec, PropertyRequirement, find_media_profile,
};
pub use types::{
    ARTIFACT_PROTOCOL_V1, ArtifactError, ArtifactFile, ArtifactId, ArtifactKind, ArtifactManifest,
    ArtifactProperty, HashDigest, MediaProfileId, PropertyType, SchemaRef,
};
pub use validator::{
    canonical_manifest_payload, compute_manifest_hash, seal_manifest, validate_manifest,
    validate_manifest_submission,
};

#[cfg(test)]
mod tests {
    use crate::heartbeat::AgentId;
    use crate::types::{AssignmentId, TaskId, Timestamp};

    use super::*;

    fn hash(value: u8) -> HashDigest {
        HashDigest::from_sha256_hex(format!("{value:064x}")).unwrap()
    }

    fn text_file() -> ArtifactFile {
        ArtifactFile::new(
            "https://agent.example/report.md",
            hash(1),
            "text/markdown",
            "text.markdown.utf8.v1",
            120,
        )
    }

    fn manifest(files: Vec<ArtifactFile>, kind: ArtifactKind) -> ArtifactManifest {
        ArtifactManifest::new(
            "artifact-1",
            "task-1",
            "assignment-1",
            "agent-1",
            kind,
            files,
            Timestamp(10),
        )
    }

    #[test]
    fn validates_sealed_manifest_and_submission_identity() {
        let manifest = seal_manifest(manifest(vec![text_file()], ArtifactKind::Single)).unwrap();
        let manifest_hash = validate_manifest_submission(
            &manifest,
            &AssignmentId::from("assignment-1"),
            &AgentId::from("agent-1"),
        )
        .unwrap();

        assert_eq!(manifest.manifest_hash.unwrap(), manifest_hash);
    }

    #[test]
    fn rejects_manifest_hash_mismatch() {
        let mut manifest =
            seal_manifest(manifest(vec![text_file()], ArtifactKind::Single)).unwrap();
        manifest.manifest_hash = Some(hash(2));

        let error = validate_manifest(&manifest).unwrap_err();

        assert!(matches!(error, ArtifactError::ManifestHashMismatch { .. }));
    }

    #[test]
    fn rejects_media_type_mismatch() {
        let file = ArtifactFile::new(
            "https://agent.example/report.md",
            hash(1),
            "text/plain",
            "text.markdown.utf8.v1",
            120,
        );

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert!(matches!(error, ArtifactError::MediaTypeMismatch { .. }));
    }

    #[test]
    fn image_profile_requires_declared_properties() {
        let file = ArtifactFile::new(
            "https://agent.example/image.png",
            hash(1),
            "image/png",
            "image.png.srgb.v1",
            1024,
        )
        .with_property("width", ArtifactProperty::Integer(1024))
        .with_property("height", ArtifactProperty::Integer(768))
        .with_property("color_space", ArtifactProperty::Text("srgb".to_string()))
        .with_property("bit_depth", ArtifactProperty::Integer(8));

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert_eq!(
            error,
            ArtifactError::MissingProperty {
                index: 0,
                profile: MediaProfileId::from("image.png.srgb.v1"),
                property: "alpha".to_string()
            }
        );
    }

    #[test]
    fn profile_rejects_wrong_fixed_encoding_value() {
        let file = ArtifactFile::new(
            "https://agent.example/video.mp4",
            hash(1),
            "video/mp4",
            "video.mp4.h264-aac.v1",
            4096,
        )
        .with_property("container", ArtifactProperty::Text("webm".to_string()))
        .with_property("video_codec", ArtifactProperty::Text("h264".to_string()))
        .with_property("width", ArtifactProperty::Integer(1920))
        .with_property("height", ArtifactProperty::Integer(1080))
        .with_property("duration_ms", ArtifactProperty::Integer(60_000))
        .with_property("fps", ArtifactProperty::Integer(30));

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert_eq!(
            error,
            ArtifactError::InvalidPropertyValue {
                index: 0,
                property: "container".to_string(),
                expected: "mp4".to_string(),
                actual: "webm".to_string()
            }
        );
    }

    #[test]
    fn profile_rejects_zero_integer_property() {
        let file = ArtifactFile::new(
            "https://agent.example/image.png",
            hash(1),
            "image/png",
            "image.png.srgb.v1",
            1024,
        )
        .with_property("width", ArtifactProperty::Integer(0))
        .with_property("height", ArtifactProperty::Integer(768))
        .with_property("color_space", ArtifactProperty::Text("srgb".to_string()))
        .with_property("bit_depth", ArtifactProperty::Integer(8))
        .with_property("alpha", ArtifactProperty::Bool(true));

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert_eq!(
            error,
            ArtifactError::InvalidPropertyValue {
                index: 0,
                property: "width".to_string(),
                expected: "> 0".to_string(),
                actual: "0".to_string()
            }
        );
    }

    #[test]
    fn structured_json_profile_requires_schema() {
        let file = ArtifactFile::new(
            "https://agent.example/verdict.json",
            hash(1),
            "application/vnd.agent.review-verdict+json",
            "application.vnd.agent.review-verdict-json.v1",
            512,
        );

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert!(matches!(error, ArtifactError::MissingSchema { .. }));
    }

    #[test]
    fn bundle_requires_unique_paths() {
        let files = vec![
            text_file().with_path("report.md"),
            text_file().with_path("report.md"),
        ];

        let error = seal_manifest(manifest(files, ArtifactKind::Bundle)).unwrap_err();

        assert_eq!(
            error,
            ArtifactError::DuplicateBundlePath("report.md".to_string())
        );
    }

    #[test]
    fn canonical_hash_is_stable_across_bundle_file_order() {
        let first = text_file().with_path("a.md");
        let second = ArtifactFile::new(
            "https://agent.example/b.md",
            hash(2),
            "text/markdown",
            "text.markdown.utf8.v1",
            80,
        )
        .with_path("b.md");

        let left = seal_manifest(manifest(
            vec![first.clone(), second.clone()],
            ArtifactKind::Bundle,
        ))
        .unwrap();
        let right = seal_manifest(manifest(vec![second, first], ArtifactKind::Bundle)).unwrap();

        assert_eq!(left.manifest_hash, right.manifest_hash);
        assert_eq!(
            compute_manifest_hash(&left).unwrap(),
            compute_manifest_hash(&right).unwrap()
        );
    }

    #[test]
    fn validates_assignment_and_producer_identity() {
        let manifest = seal_manifest(manifest(vec![text_file()], ArtifactKind::Single)).unwrap();

        let error = validate_manifest_submission(
            &manifest,
            &AssignmentId::from("assignment-2"),
            &AgentId::from("agent-1"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            ArtifactError::AssignmentMismatch {
                expected: AssignmentId::from("assignment-2"),
                actual: AssignmentId::from("assignment-1")
            }
        );
    }

    #[test]
    fn manifest_hash_can_be_used_as_livesession_output_hash() {
        let manifest = seal_manifest(manifest(vec![text_file()], ArtifactKind::Single)).unwrap();
        let manifest_hash = manifest.manifest_hash.unwrap();

        let output_hash = crate::types::OutputHash::from(manifest_hash.to_string());

        assert_eq!(output_hash.as_str(), manifest_hash.as_str());
    }

    #[test]
    fn canonical_payload_excludes_manifest_hash_and_signature() {
        let mut left = seal_manifest(manifest(vec![text_file()], ArtifactKind::Single)).unwrap();
        let mut right = left.clone().with_signature("signature");
        right.manifest_hash = Some(hash(3));

        left.signature = None;
        assert_eq!(
            canonical_manifest_payload(&left),
            canonical_manifest_payload(&right)
        );
    }

    #[test]
    fn artifact_ids_reject_empty_values() {
        assert_eq!(
            ArtifactId::try_new(" ").unwrap_err(),
            ArtifactError::EmptyArtifactId
        );
    }

    #[test]
    fn schema_names_must_not_be_empty() {
        let schema = SchemaRef::new("", "v1", hash(3));
        let file = ArtifactFile::new(
            "https://agent.example/verdict.json",
            hash(1),
            "application/vnd.agent.review-verdict+json",
            "application.vnd.agent.review-verdict-json.v1",
            512,
        )
        .with_schema(schema);

        let error = seal_manifest(manifest(vec![file], ArtifactKind::Single)).unwrap_err();

        assert_eq!(error, ArtifactError::InvalidSchema { index: 0 });
    }

    #[test]
    fn known_baseline_profile_can_be_found() {
        let profile = find_media_profile(&MediaProfileId::from("video.mp4.h264-aac.v1")).unwrap();

        assert_eq!(profile.media_type, "video/mp4");
    }

    #[test]
    fn manifest_uses_expected_task_id() {
        let manifest = manifest(vec![text_file()], ArtifactKind::Single);

        assert_eq!(manifest.task_id, TaskId::from("task-1"));
    }
}
