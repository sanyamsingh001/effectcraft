//! Audio sample rates clamp JSON integers before narrowing.
use effectcraft_engine::Session;
use serde_json::json;

#[test]
fn audio_sample_rate_clamps_before_narrowing_the_json_integer() {
    let mut session = Session::default();
    session.execute("comp.new", json!({"width":4,"height":4,"duration":0.1})).unwrap();
    for (requested, expected) in [(0u64, 8_000u32), (44_100, 44_100), (192_001, 192_000), (u64::from(u32::MAX) + 1, 192_000), (u64::MAX, 192_000)] {
        let result = session.execute("renderQueue.add", json!({"format":"aiff","sampleRate":requested})).unwrap();
        let actual = result["output"]["audio_sample_rate"].as_u64().unwrap();
        eprintln!("sampleRate {requested} => {actual}");
        assert_eq!(actual, u64::from(expected), "sample-rate clamping must not wrap oversizedJSON integers");
    }
}
