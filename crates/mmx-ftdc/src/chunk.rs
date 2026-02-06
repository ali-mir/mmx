use std::io::{Cursor, Read};

use bson::Document;
use flate2::read::ZlibDecoder;

use crate::bson_ext::{FlatMetric, flatten_bson};
use crate::varint::read_uvarint;

/// A decoded FTDC metric chunk containing time-series data for all metrics.
#[derive(Debug, Clone)]
pub struct DecodedChunk {
    pub metrics: Vec<MetricSeries>,
}

/// A single metric's name and time-series values across all samples in a chunk.
#[derive(Debug, Clone)]
pub struct MetricSeries {
    pub path: String,
    pub values: Vec<i64>,
}

/// Decode an FTDC metric chunk from the raw `data` field of a type-1 BSON document.
///
/// Layout of the data blob:
/// 1. `uncompressed_size` (u32 LE, 4 bytes)
/// 2. Zlib-compressed payload (rest of data)
///
/// The decompressed payload contains:
/// 1. A reference BSON document
/// 2. `metric_count` (u32 LE)
/// 3. `sample_count` (u32 LE)
/// 4. Varint-encoded delta stream (column-major, zero-RLE)
pub fn decode_chunk(data: &[u8]) -> Result<DecodedChunk, ChunkError> {
    if data.len() < 4 {
        return Err(ChunkError::TooShort);
    }

    let uncompressed_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
    let compressed = &data[4..];

    // Decompress
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decompressed = Vec::with_capacity(uncompressed_size);
    decoder
        .read_to_end(&mut decompressed)
        .map_err(ChunkError::Decompress)?;

    let mut cursor = Cursor::new(&decompressed);

    // Parse reference BSON document
    let ref_doc = Document::from_reader(&mut cursor).map_err(ChunkError::BsonParse)?;

    // Read metric_count and sample_count
    let pos = cursor.position() as usize;
    let remaining = &decompressed[pos..];
    if remaining.len() < 8 {
        return Err(ChunkError::TooShort);
    }
    let metric_count =
        u32::from_le_bytes([remaining[0], remaining[1], remaining[2], remaining[3]]) as usize;
    let sample_count =
        u32::from_le_bytes([remaining[4], remaining[5], remaining[6], remaining[7]]) as usize;

    cursor.set_position((pos + 8) as u64);

    // Flatten reference doc to get metric names and reference values
    let ref_metrics: Vec<FlatMetric> = flatten_bson(&ref_doc);
    if ref_metrics.len() != metric_count {
        return Err(ChunkError::MetricCountMismatch {
            expected: metric_count,
            actual: ref_metrics.len(),
        });
    }

    // If sample_count is 0, just return reference values
    if sample_count == 0 {
        let metrics = ref_metrics
            .into_iter()
            .map(|m| MetricSeries {
                path: m.path,
                values: vec![m.value],
            })
            .collect();
        return Ok(DecodedChunk { metrics });
    }

    // Decode delta stream (column-major with zero-RLE)
    // For each metric, decode `sample_count` deltas.
    // If the stream is truncated (e.g. partial interim file), fall back
    // to returning just the reference values.
    let deltas = decode_deltas(&mut cursor, metric_count, sample_count);

    // Apply cumulative sum from reference values to build actual values
    let metrics = ref_metrics
        .into_iter()
        .enumerate()
        .map(|(i, ref_metric)| {
            let mut values = Vec::with_capacity(1 + sample_count);
            values.push(ref_metric.value); // reference (sample 0)

            if let Some(all_deltas) = &deltas {
                let mut current = ref_metric.value;
                for &delta in &all_deltas[i] {
                    current = current.wrapping_add(delta);
                    values.push(current);
                }
            }

            MetricSeries {
                path: ref_metric.path,
                values,
            }
        })
        .collect();

    Ok(DecodedChunk { metrics })
}

/// Decode the varint delta stream. Returns None if the stream is truncated.
fn decode_deltas(
    cursor: &mut Cursor<&Vec<u8>>,
    metric_count: usize,
    sample_count: usize,
) -> Option<Vec<Vec<i64>>> {
    let mut deltas = vec![vec![0i64; sample_count]; metric_count];

    for metric_deltas in &mut deltas {
        let mut sample_idx = 0;
        while sample_idx < sample_count {
            let raw = read_uvarint(cursor).ok()?;
            if raw == 0 {
                let zero_count = read_uvarint(cursor).ok()? as usize;
                sample_idx += 1 + zero_count;
            } else {
                metric_deltas[sample_idx] = zigzag_decode(raw);
                sample_idx += 1;
            }
        }
    }

    Some(deltas)
}

/// Zig-zag decode: maps unsigned to signed.
/// 0 -> 0, 1 -> -1, 2 -> 1, 3 -> -2, 4 -> 2, ...
fn zigzag_decode(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

#[derive(Debug)]
pub enum ChunkError {
    TooShort,
    Decompress(std::io::Error),
    BsonParse(bson::de::Error),
    Varint(std::io::Error),
    MetricCountMismatch { expected: usize, actual: usize },
}

impl std::fmt::Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChunkError::TooShort => write!(f, "chunk data too short"),
            ChunkError::Decompress(e) => write!(f, "zlib decompress failed: {e}"),
            ChunkError::BsonParse(e) => write!(f, "BSON parse failed: {e}"),
            ChunkError::Varint(e) => write!(f, "varint decode failed: {e}"),
            ChunkError::MetricCountMismatch { expected, actual } => {
                write!(
                    f,
                    "metric count mismatch: header says {expected}, doc has {actual}"
                )
            }
        }
    }
}

impl std::error::Error for ChunkError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zigzag_decode() {
        assert_eq!(zigzag_decode(0), 0);
        assert_eq!(zigzag_decode(1), -1);
        assert_eq!(zigzag_decode(2), 1);
        assert_eq!(zigzag_decode(3), -2);
        assert_eq!(zigzag_decode(4), 2);
        assert_eq!(zigzag_decode(u64::MAX), i64::MIN);
    }

    #[test]
    fn test_decode_chunk_too_short() {
        assert!(decode_chunk(&[0, 0]).is_err());
    }

    /// Build a synthetic FTDC chunk for testing.
    ///
    /// Creates a chunk with the given reference doc, metric count, sample count,
    /// and raw varint-encoded delta bytes.
    fn build_test_chunk(
        ref_doc: &Document,
        metric_count: u32,
        sample_count: u32,
        delta_bytes: &[u8],
    ) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let mut uncompressed = Vec::new();

        // Write reference BSON document
        let mut doc_bytes = Vec::new();
        ref_doc
            .to_writer(&mut doc_bytes)
            .expect("serialize ref doc");
        uncompressed.extend_from_slice(&doc_bytes);

        // Write metric_count and sample_count
        uncompressed.extend_from_slice(&metric_count.to_le_bytes());
        uncompressed.extend_from_slice(&sample_count.to_le_bytes());

        // Write delta bytes
        uncompressed.extend_from_slice(delta_bytes);

        let uncompressed_size = uncompressed.len() as u32;

        // Compress
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&uncompressed).unwrap();
        let compressed = encoder.finish().unwrap();

        // Build final data blob
        let mut data = Vec::new();
        data.extend_from_slice(&uncompressed_size.to_le_bytes());
        data.extend_from_slice(&compressed);
        data
    }

    fn encode_uvarint(mut value: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        loop {
            let mut byte = (value & 0x7F) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            buf.push(byte);
            if value == 0 {
                break;
            }
        }
        buf
    }

    fn zigzag_encode(n: i64) -> u64 {
        ((n << 1) ^ (n >> 63)) as u64
    }

    #[test]
    fn test_decode_chunk_zero_samples() {
        let ref_doc = bson::doc! { "a": 10_i64, "b": 20_i64 };
        let data = build_test_chunk(&ref_doc, 2, 0, &[]);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics.len(), 2);
        assert_eq!(chunk.metrics[0].path, "a");
        assert_eq!(chunk.metrics[0].values, vec![10]);
        assert_eq!(chunk.metrics[1].path, "b");
        assert_eq!(chunk.metrics[1].values, vec![20]);
    }

    #[test]
    fn test_decode_chunk_with_deltas() {
        // Reference: a=10, b=100
        // 2 additional samples (sample_count=2)
        // Deltas for a: +1, +1 -> values: 10, 11, 12
        // Deltas for b: +10, -5 -> values: 100, 110, 105
        let ref_doc = bson::doc! { "a": 10_i64, "b": 100_i64 };

        let mut delta_bytes = Vec::new();
        // Metric 0 (a): deltas [+1, +1]
        delta_bytes.extend_from_slice(&encode_uvarint(zigzag_encode(1)));
        delta_bytes.extend_from_slice(&encode_uvarint(zigzag_encode(1)));
        // Metric 1 (b): deltas [+10, -5]
        delta_bytes.extend_from_slice(&encode_uvarint(zigzag_encode(10)));
        delta_bytes.extend_from_slice(&encode_uvarint(zigzag_encode(-5)));

        let data = build_test_chunk(&ref_doc, 2, 2, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics.len(), 2);
        assert_eq!(chunk.metrics[0].path, "a");
        assert_eq!(chunk.metrics[0].values, vec![10, 11, 12]);
        assert_eq!(chunk.metrics[1].path, "b");
        assert_eq!(chunk.metrics[1].values, vec![100, 110, 105]);
    }

    #[test]
    fn test_decode_chunk_with_zero_rle() {
        // Reference: a=5
        // 4 additional samples
        // Deltas for a: 0 (zero-RLE: 0 then run of 3 more) -> 4 zeros total
        // Values: 5, 5, 5, 5, 5
        let ref_doc = bson::doc! { "a": 5_i64 };

        let mut delta_bytes = Vec::new();
        // Zero-RLE: varint 0, then varint 3 (meaning 3 additional zeros after the first)
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // delta is 0
        delta_bytes.extend_from_slice(&encode_uvarint(3)); // 3 more zeros

        let data = build_test_chunk(&ref_doc, 1, 4, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics[0].values, vec![5, 5, 5, 5, 5]);
    }
}
