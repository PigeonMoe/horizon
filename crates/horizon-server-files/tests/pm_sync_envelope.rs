use std::collections::{BTreeMap, BTreeSet};

use horizon_core::nbt::JavaString;
use horizon_server_files::pm_sync_envelope::{EnvelopeError, SnapshotEnvelope};
use sha2::{Digest, Sha256};

fn java_fixture(name: &str) -> Vec<u8> {
    let hex = include_str!("fixtures/pm-sync/java-v2.hex")
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(key, _)| *key == name)
        .expect("Java fixture exists")
        .1;
    (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).unwrap())
        .collect()
}

#[test]
fn rust_emits_the_exact_java_snapshot_and_preserves_opaque_payloads() {
    let canonical = java_fixture("canonical");
    let slimefun = b"SFP1\x00\xff".as_slice();
    let addon = b"\x00\x01\x80".as_slice();
    let husk = b"\x1f\x8b\x00\xff\x0a";
    let recipes = [
        "".into(),
        "pm:中文\0".into(),
        "pm:🙂".into(),
        "pm:\u{e000}".into(),
        JavaString(vec![0xd800]),
    ];
    let envelope = SnapshotEnvelope {
        health: 73.25,
        recipes: recipes.iter().cloned().collect(),
        sections: BTreeMap::from([("slimefun".into(), slimefun), ("addon_z-9".into(), addon)]),
        husk,
    };
    assert_eq!(envelope.encode(canonical.len()).unwrap(), canonical);

    let decoded = SnapshotEnvelope::decode(&canonical, canonical.len()).unwrap();
    assert_eq!(decoded.health, 73.25);
    assert!(decoded.recipes.iter().eq(&recipes));
    assert_eq!(decoded.sections, envelope.sections);
    assert_eq!(decoded.husk, husk);
    let input = canonical.as_slice().as_ptr_range();
    for bytes in decoded.sections.values().copied().chain([decoded.husk]) {
        assert!(input.contains(&bytes.as_ptr()));
    }
}

#[test]
fn java_input_accepts_noncanonical_utf_duplicates_and_different_order() {
    let noncanonical = java_fixture("noncanonical");
    let canonical = java_fixture("canonical");
    let decoded = SnapshotEnvelope::decode(&noncanonical, noncanonical.len()).unwrap();
    assert_eq!(decoded.recipes.len(), 5);
    assert_eq!(decoded.encode(canonical.len()).unwrap(), canonical);
}

#[test]
fn empty_metadata_and_signed_zero_round_trip_by_java_bytes() {
    let fixture = java_fixture("empty");
    let decoded = SnapshotEnvelope::decode(&fixture, fixture.len()).unwrap();
    assert_eq!(decoded.health.to_bits(), (-0.0f64).to_bits());
    assert!(decoded.recipes.is_empty());
    assert!(decoded.sections.is_empty());
    assert_eq!(decoded.husk, [0]);
    assert_eq!(decoded.encode(fixture.len()).unwrap(), fixture);
}

#[test]
fn checksum_trailing_and_configured_bounds_fail_closed() {
    let fixture = java_fixture("canonical");
    let limit = fixture.len();
    let decoded = SnapshotEnvelope::decode(&fixture, limit).unwrap();
    assert_eq!(decoded.encode(limit - 1), Err(EnvelopeError::SizeLimit));
    assert_eq!(
        SnapshotEnvelope::decode(&fixture, limit - 1).unwrap_err(),
        EnvelopeError::SizeLimit
    );
    assert_eq!(
        SnapshotEnvelope::decode(&fixture[..31], limit).unwrap_err(),
        EnvelopeError::Truncated
    );

    let mut corrupt = fixture;
    corrupt[4] ^= 1;
    assert_eq!(
        SnapshotEnvelope::decode(&corrupt, limit).unwrap_err(),
        EnvelopeError::Checksum
    );
    let trailing = java_fixture("trailing");
    assert_eq!(
        SnapshotEnvelope::decode(&trailing, trailing.len()).unwrap_err(),
        EnvelopeError::HuskLength
    );
}

fn with_checksum(mut body: Vec<u8>) -> Vec<u8> {
    let digest = Sha256::digest(&body);
    body.extend_from_slice(&digest);
    body
}

#[test]
fn signed_invalid_counts_lengths_and_names_still_fail_validation() {
    let fixture = java_fixture("empty");
    let body = &fixture[..fixture.len() - 32];
    for (offset, value, reason) in [
        (0, *b"PMS\x01", EnvelopeError::Version),
        (
            4,
            (-1.0f64).to_be_bytes()[..4].try_into().unwrap(),
            EnvelopeError::Health,
        ),
        (12, (-1i32).to_be_bytes(), EnvelopeError::RecipeCount),
        (12, 100_001i32.to_be_bytes(), EnvelopeError::RecipeCount),
        (16, 33i32.to_be_bytes(), EnvelopeError::SectionCount),
        (20, 0i32.to_be_bytes(), EnvelopeError::HuskLength),
        (20, i32::MAX.to_be_bytes(), EnvelopeError::HuskLength),
    ] {
        let mut invalid = body.to_vec();
        invalid[offset..offset + value.len()].copy_from_slice(&value);
        let invalid = with_checksum(invalid);
        assert_eq!(
            SnapshotEnvelope::decode(&invalid, fixture.len()).unwrap_err(),
            reason
        );
    }

    let mut invalid_recipe = body[..12].to_vec();
    invalid_recipe.extend_from_slice(b"\x00\x00\x00\x01\x00\x01\xf0");
    invalid_recipe.extend_from_slice(&body[16..]);
    let invalid_recipe = with_checksum(invalid_recipe);
    assert_eq!(
        SnapshotEnvelope::decode(&invalid_recipe, invalid_recipe.len()).unwrap_err(),
        EnvelopeError::ModifiedUtf
    );

    for (entries, reason) in [
        (
            b"\x00\x00\x00\x02\x00\x01a\x00\x00\x00\x01\x11\x00\x01a\x00\x00\x00\x01\x22"
                .as_slice(),
            EnvelopeError::DuplicateSection,
        ),
        (
            b"\x00\x00\x00\x01\x00\x01A\x00\x00\x00\x01\x11".as_slice(),
            EnvelopeError::SectionName,
        ),
        (
            b"\x00\x00\x00\x01\x00\x01a\x00\x00\x00\x00".as_slice(),
            EnvelopeError::SectionLength,
        ),
    ] {
        let mut invalid = body[..16].to_vec();
        invalid.extend_from_slice(entries);
        invalid.extend_from_slice(&body[20..]);
        let invalid = with_checksum(invalid);
        assert_eq!(
            SnapshotEnvelope::decode(&invalid, invalid.len()).unwrap_err(),
            reason
        );
    }
}

#[test]
fn enforces_java_two_byte_utf_length_on_output() {
    let max_width = JavaString(vec![0xd800; usize::from(u16::MAX) / 3]);
    let mut envelope = SnapshotEnvelope {
        health: 20.0,
        recipes: BTreeSet::from([max_width]),
        sections: BTreeMap::new(),
        husk: &[1],
    };
    let encoded = envelope.encode(66_000).unwrap();
    assert_eq!(&encoded[16..18], &u16::MAX.to_be_bytes());
    let decoded = SnapshotEnvelope::decode(&encoded, 66_000).unwrap();
    assert_eq!(decoded.recipes, envelope.recipes);
    envelope.recipes.insert(JavaString(vec![0xd800; 21_846]));
    assert_eq!(envelope.encode(140_000), Err(EnvelopeError::ModifiedUtf));
}
