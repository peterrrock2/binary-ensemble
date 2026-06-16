use crate::io::bundle::manifest::BendlManifest;

#[test]
fn manifest_json_round_trip() {
    let manifest = BendlManifest {
        major_version: 1,
        minor_version: 0,
        assignment_format: "xben".to_string(),
        variant: Some("mkv_chain".to_string()),
        complete: false,
    };
    let json = serde_json::to_string(&manifest).unwrap();
    let decoded: BendlManifest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, manifest);
}

#[test]
fn manifest_accepts_missing_variant() {
    let json = r#"{"major_version":1,"minor_version":0,"assignment_format":"ben","complete":true}"#;
    let decoded: BendlManifest = serde_json::from_str(json).unwrap();
    assert_eq!(decoded.variant, None);
    assert!(decoded.complete);
}
