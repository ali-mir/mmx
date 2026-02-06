use std::io::{self, Read, Seek};

use bson::Document;

use crate::chunk::{ChunkError, DecodedChunk, decode_chunk};

/// The type of FTDC record.
#[derive(Debug, Clone)]
pub enum FtdcRecord {
    /// Type 0: Metadata document.
    Metadata(Document),
    /// Type 1: Decoded metric chunk.
    MetricChunk(DecodedChunk),
}

/// Errors that can occur while reading FTDC data.
#[derive(Debug)]
pub enum FtdcError {
    Io(io::Error),
    BsonParse(bson::de::Error),
    Chunk(ChunkError),
    UnknownType(i32),
}

impl std::fmt::Display for FtdcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FtdcError::Io(e) => write!(f, "IO error: {e}"),
            FtdcError::BsonParse(e) => write!(f, "BSON parse error: {e}"),
            FtdcError::Chunk(e) => write!(f, "chunk decode error: {e}"),
            FtdcError::UnknownType(t) => write!(f, "unknown FTDC document type: {t}"),
        }
    }
}

impl std::error::Error for FtdcError {}

impl From<io::Error> for FtdcError {
    fn from(e: io::Error) -> Self {
        FtdcError::Io(e)
    }
}

impl From<bson::de::Error> for FtdcError {
    fn from(e: bson::de::Error) -> Self {
        FtdcError::BsonParse(e)
    }
}

impl From<ChunkError> for FtdcError {
    fn from(e: ChunkError) -> Self {
        FtdcError::Chunk(e)
    }
}

/// An iterator over FTDC records from a reader.
///
/// Reads sequential BSON documents, classifies them by `type` field, and
/// decodes metric chunks.
pub struct FtdcReader<R> {
    reader: R,
    done: bool,
}

impl<R: Read> FtdcReader<R> {
    pub fn new(reader: R) -> Self {
        FtdcReader {
            reader,
            done: false,
        }
    }
}

impl<R: Read> FtdcReader<R> {
    /// Read the next FTDC record, or `None` if we've reached EOF.
    pub fn next_record(&mut self) -> Result<Option<FtdcRecord>, FtdcError> {
        if self.done {
            return Ok(None);
        }

        // Try to read the next BSON document
        let doc = match Document::from_reader(&mut self.reader) {
            Ok(doc) => doc,
            Err(e) => {
                // Check if it's an EOF-like error
                let msg = e.to_string();
                if msg.contains("end of file")
                    || msg.contains("unexpected EOF")
                    || msg.contains("failed to fill whole buffer")
                {
                    self.done = true;
                    return Ok(None);
                }
                return Err(FtdcError::BsonParse(e));
            }
        };

        // Classify by type field
        let doc_type = doc.get_i32("type").unwrap_or(-1);

        match doc_type {
            0 => {
                // Metadata document — extract the `doc` subdocument if present
                if let Some(bson::Bson::Document(inner)) = doc.get("doc") {
                    Ok(Some(FtdcRecord::Metadata(inner.clone())))
                } else {
                    Ok(Some(FtdcRecord::Metadata(doc)))
                }
            }
            1 => {
                // Metric chunk — extract the `data` binary field
                let data = match doc.get_binary_generic("data") {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        // Some FTDC files use raw_document_buf; try alternate extraction
                        return Err(FtdcError::Chunk(ChunkError::TooShort));
                    }
                };
                let chunk = decode_chunk(data)?;
                Ok(Some(FtdcRecord::MetricChunk(chunk)))
            }
            2 => {
                // Metadata delta — treat like metadata for now
                if let Some(bson::Bson::Document(inner)) = doc.get("doc") {
                    Ok(Some(FtdcRecord::Metadata(inner.clone())))
                } else {
                    Ok(Some(FtdcRecord::Metadata(doc)))
                }
            }
            other => Err(FtdcError::UnknownType(other)),
        }
    }
}

impl<R: Read> Iterator for FtdcReader<R> {
    type Item = Result<FtdcRecord, FtdcError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_record() {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => None,
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Read all metric chunks from an FTDC file, returning the final state
/// (most recent values for all metrics).
pub fn read_ftdc_file<R: Read>(reader: R) -> Result<Vec<DecodedChunk>, FtdcError> {
    let ftdc = FtdcReader::new(reader);
    let mut chunks = Vec::new();

    for record in ftdc {
        if let FtdcRecord::MetricChunk(chunk) = record? {
            chunks.push(chunk);
        }
    }

    Ok(chunks)
}

/// Scan a directory for FTDC files and return their paths sorted by name.
pub fn find_ftdc_files(dir: &std::path::Path) -> io::Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();

    if dir.is_file() {
        files.push(dir.to_path_buf());
        return Ok(files);
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            // FTDC files are typically named like `metrics.2024-01-01T00-00-00Z-00000`
            // or `diagnostic.data`; we accept any file in the directory
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Reader that can tail a file for new data.
pub struct TailingReader<R: Read + Seek> {
    reader: R,
    last_pos: u64,
}

impl<R: Read + Seek> TailingReader<R> {
    pub fn new(mut reader: R) -> io::Result<Self> {
        let pos = reader.stream_position()?;
        Ok(TailingReader {
            reader,
            last_pos: pos,
        })
    }

    /// Seek to the last known position and create a new FtdcReader
    /// to read any new data appended since our last read.
    pub fn read_new_chunks(&mut self) -> Result<Vec<DecodedChunk>, FtdcError> {
        self.reader.seek(io::SeekFrom::Start(self.last_pos))?;

        let mut chunks = Vec::new();
        loop {
            // Save position before attempting to read
            let pos = self.reader.stream_position()?;
            let doc = match Document::from_reader(&mut self.reader) {
                Ok(doc) => doc,
                Err(_) => {
                    // EOF or incomplete doc — rewind to before this attempt
                    self.last_pos = pos;
                    break;
                }
            };

            let doc_type = doc.get_i32("type").unwrap_or(-1);
            if doc_type == 1 {
                if let Ok(data) = doc.get_binary_generic("data") {
                    if let Ok(chunk) = decode_chunk(data) {
                        chunks.push(chunk);
                    }
                }
            }
        }

        Ok(chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_reader() {
        let data: &[u8] = &[];
        let chunks = read_ftdc_file(data).unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_ftdc_reader_iterator() {
        let data: &[u8] = &[];
        let reader = FtdcReader::new(data);
        let records: Vec<_> = reader.collect();
        assert!(records.is_empty());
    }
}
