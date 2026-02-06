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

    // Decode delta stream (column-major with zero-RLE).
    // When the stream ends early, remaining deltas are implicitly zero.
    let deltas = decode_deltas(&mut cursor, metric_count, sample_count);

    // Apply cumulative sum from reference values to build actual values
    let metrics = ref_metrics
        .into_iter()
        .enumerate()
        .map(|(i, ref_metric)| {
            let mut values = Vec::with_capacity(1 + sample_count);
            values.push(ref_metric.value); // reference (sample 0)

            let mut current = ref_metric.value;
            for &delta in &deltas[i] {
                current = current.wrapping_add(delta);
                values.push(current);
            }

            MetricSeries {
                path: ref_metric.path,
                values,
            }
        })
        .collect();

    Ok(DecodedChunk { metrics })
}

/// Decode the varint delta stream.
///
/// MongoDB's FTDC encoding: column-major deltas with zero-RLE.
/// The zero run counter persists across metric column boundaries —
/// a single zero-RLE pair can span multiple metrics.
///
/// Encoding: varint(0) varint(N) means 1 zero (from the marker) + N more zeros.
/// When the stream ends early, remaining deltas are implicitly zero.
fn decode_deltas(
    cursor: &mut Cursor<&Vec<u8>>,
    metric_count: usize,
    sample_count: usize,
) -> Vec<Vec<i64>> {
    let mut deltas = vec![vec![0i64; sample_count]; metric_count];
    let mut zeros_remaining: u64 = 0;

    for metric_deltas in deltas.iter_mut() {
        for delta in metric_deltas.iter_mut() {
            if zeros_remaining > 0 {
                // Still consuming a zero run — delta is already 0
                zeros_remaining -= 1;
                continue;
            }

            let Ok(raw) = read_uvarint(cursor) else {
                // Stream exhausted — remaining deltas are implicitly zero
                return deltas;
            };

            if raw == 0 {
                // Zero-RLE: varint(0) varint(N) means current position is 0,
                // then N more zeros follow (spanning across metric boundaries)
                let Ok(count) = read_uvarint(cursor) else {
                    return deltas;
                };
                zeros_remaining = count;
                // Current position delta is already 0
            } else {
                *delta = raw as i64;
            }
        }
    }

    deltas
}

#[derive(Debug)]
pub enum ChunkError {
    TooShort,
    Decompress(std::io::Error),
    BsonParse(bson::de::Error),
    MetricCountMismatch { expected: usize, actual: usize },
}

impl std::fmt::Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChunkError::TooShort => write!(f, "chunk data too short"),
            ChunkError::Decompress(e) => write!(f, "zlib decompress failed: {e}"),
            ChunkError::BsonParse(e) => write!(f, "BSON parse failed: {e}"),
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
        // FTDC deltas are unsigned: +1 stored as 1u64, -5 stored as wrapping (u64::MAX - 4)
        // Deltas for a: +1, +1 -> values: 10, 11, 12
        // Deltas for b: +10, -5 -> values: 100, 110, 105
        let ref_doc = bson::doc! { "a": 10_i64, "b": 100_i64 };

        let mut delta_bytes = Vec::new();
        // Metric 0 (a): deltas [1, 1]
        delta_bytes.extend_from_slice(&encode_uvarint(1));
        delta_bytes.extend_from_slice(&encode_uvarint(1));
        // Metric 1 (b): deltas [10, wrapping(-5)]
        delta_bytes.extend_from_slice(&encode_uvarint(10));
        delta_bytes.extend_from_slice(&encode_uvarint((-5_i64) as u64));

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
        // Deltas for a: 4 zeros
        // Encoding: varint(0) varint(3) → 1 zero (marker) + 3 more = 4 total
        // Values: 5, 5, 5, 5, 5
        let ref_doc = bson::doc! { "a": 5_i64 };

        let mut delta_bytes = Vec::new();
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // zero marker
        delta_bytes.extend_from_slice(&encode_uvarint(3)); // 3 more zeros

        let data = build_test_chunk(&ref_doc, 1, 4, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics[0].values, vec![5, 5, 5, 5, 5]);
    }

    #[test]
    fn test_decode_chunk_mixed_zeros_and_nonzeros() {
        // Reference: a=0, 5 additional samples
        // Deltas: [0, 0, 5, 0, 3]
        // Encoding: varint(0) varint(1) [1+1=2 zeros], varint(5),
        //           varint(0) varint(0) [1+0=1 zero], varint(3)
        // Values: 0, 0, 0, 5, 5, 8
        let ref_doc = bson::doc! { "a": 0_i64 };

        let mut delta_bytes = Vec::new();
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // zero marker
        delta_bytes.extend_from_slice(&encode_uvarint(1)); // 1 more zero (2 total)
        delta_bytes.extend_from_slice(&encode_uvarint(5)); // delta +5
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // zero marker
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // 0 more zeros (1 total)
        delta_bytes.extend_from_slice(&encode_uvarint(3)); // delta +3

        let data = build_test_chunk(&ref_doc, 1, 5, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics[0].values, vec![0, 0, 0, 5, 5, 8]);
    }

    #[test]
    fn test_decode_chunk_multi_metric_zero_rle_alignment() {
        // 3 metrics, 3 additional samples
        // Metric a: deltas [0, 0, 0]
        // Metric b: deltas [1, 2, 3]
        // Metric c: deltas [10, 20, 30]
        //
        // Compressor processes column-major; zeros accumulate until non-zero:
        //   a[0]=0 (count=1), a[1]=0 (count=2), a[2]=0 (count=3),
        //   b[0]=1 → flush: varint(0) varint(2) [3-1=2], then varint(1)
        //   b[1]=2 → varint(2), b[2]=3 → varint(3)
        //   c[0]=10 → varint(10), c[1]=20 → varint(20), c[2]=30 → varint(30)
        let ref_doc = bson::doc! { "a": 0_i64, "b": 0_i64, "c": 0_i64 };

        let mut delta_bytes = Vec::new();
        // 3 zeros for metric a: varint(0) varint(2) → 1 + 2 = 3
        delta_bytes.extend_from_slice(&encode_uvarint(0));
        delta_bytes.extend_from_slice(&encode_uvarint(2));
        // Metric b: 1, 2, 3
        delta_bytes.extend_from_slice(&encode_uvarint(1));
        delta_bytes.extend_from_slice(&encode_uvarint(2));
        delta_bytes.extend_from_slice(&encode_uvarint(3));
        // Metric c: 10, 20, 30
        delta_bytes.extend_from_slice(&encode_uvarint(10));
        delta_bytes.extend_from_slice(&encode_uvarint(20));
        delta_bytes.extend_from_slice(&encode_uvarint(30));

        let data = build_test_chunk(&ref_doc, 3, 3, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics[0].values, vec![0, 0, 0, 0]);
        assert_eq!(chunk.metrics[1].values, vec![0, 1, 3, 6]);
        assert_eq!(chunk.metrics[2].values, vec![0, 10, 30, 60]);
    }

    #[test]
    fn test_decode_chunk_zero_run_in_middle() {
        // Reference: a=10, 6 additional samples
        // Deltas: [1, 2, 0, 0, 0, 3]
        // Encoding: varint(1), varint(2), varint(0) varint(2) [1+2=3 zeros], varint(3)
        // Values: 10, 11, 13, 13, 13, 13, 16
        let ref_doc = bson::doc! { "a": 10_i64 };

        let mut delta_bytes = Vec::new();
        delta_bytes.extend_from_slice(&encode_uvarint(1)); // +1
        delta_bytes.extend_from_slice(&encode_uvarint(2)); // +2
        delta_bytes.extend_from_slice(&encode_uvarint(0)); // zero marker
        delta_bytes.extend_from_slice(&encode_uvarint(2)); // 2 more zeros (3 total)
        delta_bytes.extend_from_slice(&encode_uvarint(3)); // +3

        let data = build_test_chunk(&ref_doc, 1, 6, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        assert_eq!(chunk.metrics[0].values, vec![10, 11, 13, 13, 13, 13, 16]);
    }

    #[test]
    fn test_decode_chunk_zero_rle_spans_metrics() {
        // Critical test: zero run that spans across metric column boundaries.
        // 2 metrics (a, b), 3 additional samples.
        // Metric a: deltas [0, 0, 0]
        // Metric b: deltas [0, 0, 5]
        //
        // Compressor accumulates 5 consecutive zeros across both columns,
        // then flushes when it hits 5:
        //   varint(0) varint(4) [1+4=5 zeros], varint(5)
        //
        // The zero run starts in metric a and bleeds into metric b.
        let ref_doc = bson::doc! { "a": 0_i64, "b": 100_i64 };

        let mut delta_bytes = Vec::new();
        // 5 zeros spanning both metrics: varint(0) varint(4)
        delta_bytes.extend_from_slice(&encode_uvarint(0));
        delta_bytes.extend_from_slice(&encode_uvarint(4));
        // Then the non-zero delta for b[2]
        delta_bytes.extend_from_slice(&encode_uvarint(5));

        let data = build_test_chunk(&ref_doc, 2, 3, &delta_bytes);
        let chunk = decode_chunk(&data).unwrap();

        // a: ref=0, deltas=[0,0,0] → [0, 0, 0, 0]
        assert_eq!(chunk.metrics[0].values, vec![0, 0, 0, 0]);
        // b: ref=100, deltas=[0,0,5] → [100, 100, 100, 105]
        assert_eq!(chunk.metrics[1].values, vec![100, 100, 100, 105]);
    }
}
