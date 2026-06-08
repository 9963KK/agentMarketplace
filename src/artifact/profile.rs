use super::types::{MediaProfileId, PropertyType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropertyRequirement {
    pub name: &'static str,
    pub kind: PropertyType,
    pub expected_text: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MediaProfileSpec {
    pub id: &'static str,
    pub media_type: &'static str,
    pub requires_schema: bool,
    pub required_properties: &'static [PropertyRequirement],
}

const IMAGE_PROPERTIES: &[PropertyRequirement] = &[
    PropertyRequirement {
        name: "width",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "height",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "color_space",
        kind: PropertyType::Text,
        expected_text: Some("srgb"),
    },
    PropertyRequirement {
        name: "bit_depth",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "alpha",
        kind: PropertyType::Bool,
        expected_text: None,
    },
];

const VIDEO_PROPERTIES: &[PropertyRequirement] = &[
    PropertyRequirement {
        name: "container",
        kind: PropertyType::Text,
        expected_text: Some("mp4"),
    },
    PropertyRequirement {
        name: "video_codec",
        kind: PropertyType::Text,
        expected_text: Some("h264"),
    },
    PropertyRequirement {
        name: "width",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "height",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "duration_ms",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "fps",
        kind: PropertyType::Integer,
        expected_text: None,
    },
];

const AUDIO_PROPERTIES: &[PropertyRequirement] = &[
    PropertyRequirement {
        name: "duration_ms",
        kind: PropertyType::Integer,
        expected_text: None,
    },
    PropertyRequirement {
        name: "codec",
        kind: PropertyType::Text,
        expected_text: Some("mp3"),
    },
];

pub const BASELINE_MEDIA_PROFILES: &[MediaProfileSpec] = &[
    MediaProfileSpec {
        id: "text.plain.utf8.v1",
        media_type: "text/plain",
        requires_schema: false,
        required_properties: &[],
    },
    MediaProfileSpec {
        id: "text.markdown.utf8.v1",
        media_type: "text/markdown",
        requires_schema: false,
        required_properties: &[],
    },
    MediaProfileSpec {
        id: "application.json.v1",
        media_type: "application/json",
        requires_schema: true,
        required_properties: &[],
    },
    MediaProfileSpec {
        id: "application.vnd.agent.review-verdict-json.v1",
        media_type: "application/vnd.agent.review-verdict+json",
        requires_schema: true,
        required_properties: &[],
    },
    MediaProfileSpec {
        id: "image.png.srgb.v1",
        media_type: "image/png",
        requires_schema: false,
        required_properties: IMAGE_PROPERTIES,
    },
    MediaProfileSpec {
        id: "image.jpeg.srgb.v1",
        media_type: "image/jpeg",
        requires_schema: false,
        required_properties: IMAGE_PROPERTIES,
    },
    MediaProfileSpec {
        id: "video.mp4.h264-aac.v1",
        media_type: "video/mp4",
        requires_schema: false,
        required_properties: VIDEO_PROPERTIES,
    },
    MediaProfileSpec {
        id: "audio.mpeg.mp3.v1",
        media_type: "audio/mpeg",
        requires_schema: false,
        required_properties: AUDIO_PROPERTIES,
    },
];

pub fn find_media_profile(profile: &MediaProfileId) -> Option<&'static MediaProfileSpec> {
    BASELINE_MEDIA_PROFILES
        .iter()
        .find(|spec| spec.id == profile.as_str())
}
