use std::collections::BTreeMap;

use hdrhistogram::serialization::{Serializer, V2Serializer};
use hdrhistogram::Histogram;

pub fn encode_histograms(histograms: &BTreeMap<String, Histogram<u64>>) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    output.extend_from_slice(b"HELIX-HDR-V1\n");
    let mut serializer = V2Serializer::new();
    for (name, histogram) in histograms {
        let mut encoded = Vec::new();
        serializer
            .serialize(histogram, &mut encoded)
            .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: HDR serialize: {error:?}"))?;
        output.extend_from_slice(&(name.len() as u32).to_le_bytes());
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        output.extend_from_slice(&encoded);
    }
    Ok(output)
}
