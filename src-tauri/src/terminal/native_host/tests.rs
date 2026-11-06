use super::*;
#[test]
fn font_configuration_and_grid() {
    assert_eq!(grid_size(640, 480, 11, 23), (56, 19));
    assert_eq!(grid_size(1, 1, 11, 23), (1, 1));
    let cfg:ViewConfig=serde_json::from_value(serde_json::json!({"fontFamily":"monospace","fontSize":15,"fontWeight":700,"linePadding":4})).unwrap();
    assert!(cfg.validate().is_ok());
    let mut bad = cfg;
    bad.font_size = f64::NAN;
    assert!(bad.validate().is_err());
}
#[test]
fn native_frames_do_not_require_increasing_sequence() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let engine = super::super::engine::TerminalEngine::new(Default::default(), tx);
    let a = engine.native_snapshot_frame();
    engine.feed(b"NEW_CONTENT");
    let b = engine.native_snapshot_frame();
    let a = Snapshot::decode(&a, &ViewConfig::default()).unwrap();
    let b = Snapshot::decode(&b, &ViewConfig::default()).unwrap();
    assert_eq!(a.seq, b.seq);
    // Wire seq is deliberately unchanged; configuration draw readiness advances
    // independently, without mutating canonical damage/seq just to get an ACK.
    let mut revision = input::ConfigRevision::default();
    revision.apply(1, Some(1), 1).unwrap();
    assert!(revision.draw_complete(1));
    revision.apply(1, Some(1), 2).unwrap();
    assert_eq!(revision.drawn, Some(1));
    assert!(revision.draw_complete(2));
    assert_eq!(revision.drawn, Some(2));
    assert!(b
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect::<String>()
        .contains("NEW_CONTENT"));
}

#[test]
fn geometry_rejects_nonfinite_negative_and_oversized_allocations() {
    for g in [
        Geometry {
            x: f64::NAN,
            ..Geometry::default()
        },
        Geometry {
            width: 0.,
            ..Geometry::default()
        },
        Geometry {
            x: -1.,
            ..Geometry::default()
        },
        Geometry {
            width: 100000.,
            ..Geometry::default()
        },
    ] {
        assert!(g.validate().is_err());
    }
    assert!(Geometry::default().validate().is_ok());
}
#[test]
fn attachment_detach_is_view_only_and_duplicate_rejected() {
    let mut registry = Registry::default();
    let id = uuid::Uuid::new_v4();
    assert!(registry
        .attach(id, Geometry::default(), ViewConfig::default())
        .is_ok());
    assert_eq!(
        registry
            .attach(id, Geometry::default(), ViewConfig::default())
            .unwrap_err()
            .code,
        "already_attached"
    );
    assert!(registry.update(id, Geometry::default(), false).is_ok());
    assert!(!registry.get(id).unwrap().visible);
    assert!(registry.detach(id).is_ok());
    assert_eq!(registry.get(id).unwrap_err().code, "not_attached");
    assert!(registry
        .attach(id, Geometry::default(), ViewConfig::default())
        .is_ok());
}
#[test]
fn existing_engine_full_frame_decodes_without_second_parser() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let engine = super::super::engine::TerminalEngine::new(Default::default(), tx);
    engine.feed(b"\x1b[31mENGINE ONLY");
    let bytes = engine.native_snapshot_frame();
    let snapshot = Snapshot::decode(&bytes, &ViewConfig::default()).unwrap();
    assert_eq!(snapshot.cols, 80);
    assert!(snapshot
        .cells
        .iter()
        .map(|c| c.text.as_str())
        .collect::<String>()
        .contains("ENGINE ONLY"));
    assert!(Snapshot::decode(&bytes[..20], &ViewConfig::default()).is_err());
    assert!(Snapshot::decode(&[0; 32], &ViewConfig::default()).is_err());
}
#[test]
fn decoder_rejects_raw_and_partial_instead_of_blank_fallback() {
    use super::super::engine::frame::*;
    for kind in [KIND_PARTIAL, KIND_RAW] {
        let bytes = FrameWriter::new(&FrameHeader {
            kind,
            cols: 2,
            rows: 2,
            ..Default::default()
        })
        .finish();
        assert_eq!(
            Snapshot::decode(&bytes, &ViewConfig::default())
                .unwrap_err()
                .code,
            "unsupported_frame"
        );
    }
}
