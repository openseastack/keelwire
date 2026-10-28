//! Integration tests over recorded captures.

use keelwire_core::fastpacket::Reassembler;
use keelwire_core::formats::parse_candump;
use keelwire_core::iso::ProductInfo;

#[test]
fn python_sender_capture_reassembles() {
    let text = include_str!("data/python_sim.candump");
    let mut r = Reassembler::default();
    let mut gnss = 0;
    let mut products = Vec::new();
    for line in text.lines() {
        let f = parse_candump(line).expect("valid candump line");
        let id = f.id();
        if r.is_fast(id.pgn) {
            if let Some(a) = r.push(f.timestamp_us, id.priority, id.pgn, id.source, id.destination, f.payload()) {
                if a.pgn == 129029 {
                    assert_eq!(a.data.len(), 43);
                    gnss += 1;
                }
                if a.pgn == 126996 {
                    products.push(ProductInfo::parse(&a.data).unwrap().model_id);
                }
            }
        }
    }
    assert!(gnss >= 4, "gnss = {gnss}");
    assert!(products.contains(&"PyGPS".to_string()));
    assert_eq!(r.stats.orphans, 0);
    assert_eq!(r.stats.invalid, 0);
}

#[test]
fn canboat_sample_pcap_parses() {
    let bytes = include_bytes!("data/canboat_can0-1.pcap");
    let frames = keelwire_core::pcap::parse_bytes(bytes).expect("pcap");
    assert!(frames.len() > 10, "{} frames", frames.len());
    let mut r = Reassembler::default();
    let mut complete = 0;
    for f in &frames {
        let id = f.id();
        if r.is_fast(id.pgn) && r.push(f.timestamp_us, id.priority, id.pgn, id.source, id.destination, f.payload()).is_some() {
            complete += 1;
        }
    }
    // Real-world capture: must contain extended frames with sane timestamps.
    assert!(frames.iter().all(|f| f.is_extended));
    assert_eq!(frames.len(), 49_928);
    assert!(complete > 100, "{complete} fast packets completed");
    assert!(r.stats.completed > 0);
}
